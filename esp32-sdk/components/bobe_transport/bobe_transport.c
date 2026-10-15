#include "bobe_transport.h"

#include "bobe_storage.h"
#include "bobe_task_config.h"
#include "esp_check.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_random.h"
#include "esp_task_wdt.h"
#include "esp_timer.h"
#include "esp_websocket_client.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "freertos/queue.h"
#include "freertos/task.h"
#include "nvs.h"

#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define BOBE_TRANSPORT_CONTROL_QUEUE_DEPTH 8U
#define BOBE_TRANSPORT_AUDIO_QUEUE_DEPTH 32U
#define BOBE_TRANSPORT_CLOSE_QUEUE_DEPTH 8U
#define BOBE_TRANSPORT_TX_JSON_BYTES (BOBE_CONTROL_MAX_BYTES + 1U)
#define BOBE_TRANSPORT_MAX_AUDIO_AGE_US UINT64_C(750000)
#define BOBE_TRANSPORT_BINARY_BYTES \
    (BOBE_MEDIA_HEADER_BYTES + BOBE_SPEAKER_FRAME_SAMPLES * sizeof(int16_t))

#ifndef CONFIG_BOBE_ALLOW_INSECURE_WS
#define CONFIG_BOBE_ALLOW_INSECURE_WS 0
#endif
#ifndef CONFIG_BOBE_BODY_ENDPOINT_URI
#define CONFIG_BOBE_BODY_ENDPOINT_URI ""
#endif

static const char *TAG = "bobe_transport";

typedef struct {
    SemaphoreHandle_t done;
    atomic_uint references;
    atomic_bool sent;
} control_completion_t;

typedef struct {
    size_t length;
    uint32_t generation;
    control_completion_t *completion;
    char *bytes;
} control_tx_item_t;

typedef struct {
    bobe_audio_frame_t frame;
    uint32_t generation;
} audio_tx_item_t;

typedef struct {
    uint32_t generation;
    bool only_if_unready;
} close_request_t;

static esp_websocket_client_handle_t client;
static QueueHandle_t control_tx_queue;
static QueueHandle_t audio_tx_queue;
static QueueHandle_t close_request_queue;
static StaticQueue_t control_tx_queue_state;
static StaticQueue_t audio_tx_queue_state;
static StaticQueue_t close_request_queue_state;
static uint8_t close_request_queue_storage[
    BOBE_TRANSPORT_CLOSE_QUEUE_DEPTH * sizeof(close_request_t)
];
static bobe_transport_callback_t transport_callback;
static void *transport_callback_context;
static bobe_transport_callback_t transport_observer;
static void *transport_observer_context;
static esp_timer_handle_t handshake_timer;
static TaskHandle_t transport_tx_task_handle;
static char device_id[BOBE_DEVICE_ID_MAX_BYTES];
static char boot_id[33];
static char firmware_version[65];
static char hardware_revision[65];
static bobe_hello_info_t hello_info;
static uint32_t build_number;
static uint32_t max_image_size;
static bobe_transport_update_verifier_t update_verifier;
static bool supplied_credentials;
static uint8_t *control_queue_storage;
static uint8_t *audio_queue_storage;
static char endpoint_uri[256];
static char *server_ca_pem;
static char *client_cert_pem;
static char *client_key_pem;
static uint64_t highest_controller_epoch;
static atomic_bool link_connected;
static atomic_bool session_ready;
static atomic_bool session_closing;
static atomic_bool desired_running;
static atomic_bool client_task_running;
static atomic_bool restart_requested;
static atomic_bool initialized;
static atomic_uint audio_enqueued_count;
static atomic_uint audio_processed_count;
static atomic_bool audio_discontinuity_pending;
static atomic_uint connection_generation;
static atomic_uint handshake_generation;
static atomic_uint playback_stream_id;
static atomic_uint playback_expected_sequence;
static atomic_uint playback_sequence_gaps;
static atomic_bool playback_sequence_started;
static atomic_bool disconnect_notified;
static atomic_bool authenticated_time_confirmed;
static bool configured;
static uint8_t message_rx[BOBE_CONTROL_MAX_BYTES + 1U];
static size_t message_rx_length;
static uint8_t message_rx_opcode;

static void *allocate_buffer(size_t size)
{
    void *buffer = heap_caps_malloc(size, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
    return buffer != NULL ? buffer : heap_caps_malloc(size, MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);
}

static void release_completion(control_completion_t *completion)
{
    if (completion != NULL &&
        atomic_fetch_sub(&completion->references, 1) == 1) {
        vSemaphoreDelete(completion->done);
        free(completion);
    }
}

static void complete_control(
    const control_tx_item_t *item,
    bool sent
)
{
    free(item->bytes);
    if (item->completion == NULL) {
        return;
    }
    atomic_store(&item->completion->sent, sent);
    xSemaphoreGive(item->completion->done);
    release_completion(item->completion);
}

static void reset_tx_queues(void)
{
    if (control_tx_queue != NULL) {
        control_tx_item_t item;
        while (xQueueReceive(control_tx_queue, &item, 0) == pdTRUE) {
            complete_control(&item, false);
        }
    }
    if (audio_tx_queue != NULL) {
        audio_tx_item_t item;
        while (xQueueReceive(audio_tx_queue, &item, 0) == pdTRUE) {
            atomic_fetch_add(&audio_processed_count, 1U);
        }
    }
}

static void reset_session(void)
{
    atomic_store(&session_ready, false);
    atomic_store(&authenticated_time_confirmed, false);
    atomic_store(&link_connected, false);
    atomic_store(&playback_stream_id, 0);
    atomic_store(&playback_expected_sequence, 0);
    atomic_store(&playback_sequence_gaps, 0);
    atomic_store(&playback_sequence_started, false);
    atomic_store(&audio_discontinuity_pending, false);
    message_rx_length = 0;
    message_rx_opcode = 0;
    reset_tx_queues();
    if (close_request_queue != NULL) {
        xQueueReset(close_request_queue);
    }
    if (handshake_timer != NULL && esp_timer_is_active(handshake_timer)) {
        ESP_ERROR_CHECK_WITHOUT_ABORT(esp_timer_stop(handshake_timer));
    }
}

static void publish_event(const bobe_transport_event_t *event)
{
    if (transport_callback != NULL) {
        transport_callback(event, transport_callback_context);
    }
    if (transport_observer != NULL) transport_observer(event, transport_observer_context);
}

esp_err_t bobe_transport_set_observer(bobe_transport_callback_t callback, void *context)
{
    if (atomic_load(&desired_running) || atomic_load(&client_task_running)) return ESP_ERR_INVALID_STATE;
    transport_observer = callback;
    transport_observer_context = context;
    return ESP_OK;
}

const bobe_capabilities_t *bobe_transport_capabilities(void)
{
    return atomic_load(&initialized) ? &hello_info.capabilities : NULL;
}

static void publish_simple(bobe_transport_event_kind_t kind)
{
    const uint32_t generation = atomic_load(&connection_generation);
    const bobe_transport_event_t event = {
        .kind = kind,
        .generation = generation,
    };
    publish_event(&event);
}

static void publish_error_for_generation(
    esp_err_t error,
    uint32_t generation
)
{
    const bobe_transport_event_t event = {
        .kind = BOBE_TRANSPORT_ERROR,
        .generation = generation,
        .data.error = error,
    };
    publish_event(&event);
}

static void publish_error(esp_err_t error)
{
    publish_error_for_generation(
        error,
        atomic_load(&connection_generation)
    );
}

static uint32_t publish_disconnected_once(void)
{
    if (!atomic_exchange(&disconnect_notified, true)) {
        const uint32_t generation =
            atomic_fetch_add(&connection_generation, 1U) + 1U;
        if (atomic_load(&desired_running)) {
            const bobe_transport_event_t event = {
                .kind = BOBE_TRANSPORT_DISCONNECTED,
                .generation = generation,
            };
            publish_event(&event);
        }
        return generation;
    }
    return atomic_load(&connection_generation);
}

static void request_close_for_generation(
    esp_err_t error,
    bool only_if_unready,
    uint32_t generation
)
{
    if (generation != atomic_load(&connection_generation)) {
        return;
    }
    atomic_store(&session_closing, true);
    atomic_store(&session_ready, false);
    publish_error_for_generation(error, generation);
    const close_request_t request = {
        .generation = generation,
        .only_if_unready = only_if_unready,
    };
    if (close_request_queue == NULL ||
        xQueueSend(close_request_queue, &request, 0) != pdTRUE) {
        ESP_LOGE(
            TAG,
            "BodyLink close queue full for generation=%u error=%s",
            (unsigned int)generation,
            esp_err_to_name(error)
        );
        return;
    }
    if (transport_tx_task_handle != NULL) {
        xTaskNotifyGive(transport_tx_task_handle);
    }
}

static void request_close(esp_err_t error, bool only_if_unready)
{
    request_close_for_generation(
        error,
        only_if_unready,
        atomic_load(&connection_generation)
    );
}

static void fail_connection(esp_err_t error)
{
    request_close(error, false);
}

static void fail_connection_for_generation(
    esp_err_t error,
    uint32_t generation
)
{
    request_close_for_generation(error, false, generation);
}

static esp_err_t start_client_task(void)
{
    if (!atomic_load(&desired_running)) {
        return ESP_ERR_INVALID_STATE;
    }
    if (atomic_exchange(&client_task_running, true)) {
        return ESP_OK;
    }
    const esp_err_t result = esp_websocket_client_start(client);
    if (result != ESP_OK) {
        atomic_store(&client_task_running, false);
    }
    return result;
}

static bool enqueue_control(
    const char *json,
    size_t length,
    bool require_ready
)
{
    if (!atomic_load(&link_connected) ||
        (require_ready && !atomic_load(&session_ready)) ||
        json == NULL || length == 0 ||
        length >= BOBE_TRANSPORT_TX_JSON_BYTES) {
        return false;
    }
    control_tx_item_t item = {
        .length = length,
        .generation = atomic_load(&connection_generation),
    };
    item.bytes = allocate_buffer(length + 1U);
    if (item.bytes == NULL) return false;
    memcpy(item.bytes, json, length);
    item.bytes[length] = '\0';
    if (xQueueSend(control_tx_queue, &item, 0) != pdTRUE) {
        free(item.bytes);
        return false;
    }
    xTaskNotifyGive(transport_tx_task_handle);
    return true;
}

static bool enqueue_hello(void)
{
    char json[1024];
    return bobe_protocol_build_hello(
               json,
               sizeof(json),
               &hello_info
           ) == BOBE_PROTOCOL_OK &&
           enqueue_control(json, strlen(json), false);
}

static bool accept_controller_epoch(uint64_t epoch)
{
    return epoch == highest_controller_epoch;
}

static void reject_handshake(esp_err_t error)
{
    request_close(error, true);
}

static bool parse_verified_update_offer(
    const uint8_t *data,
    size_t length,
    bobe_control_message_t *message
)
{
    if (update_verifier == NULL) return false;
    const time_t now = time(NULL);
    const bool trusted_time = atomic_load(&authenticated_time_confirmed) &&
                              now > 0;
    const bobe_update_manifest_context_t context = {
        .device_model = hardware_revision,
        .max_image_size = max_image_size,
        .current_build_number = build_number,
        .trusted_time_valid = trusted_time,
        .trusted_unix_time = trusted_time ? (int64_t)now : 0,
    };
    bobe_update_manifest_result_t manifest_result;
    const bobe_update_offer_result_t result =
        update_verifier(
            (const char *)data,
            length,
            &context,
            &message->data.update,
            &manifest_result
        );
    if (result == BOBE_UPDATE_OFFER_NOT_OFFER) {
        return false;
    }
    if (!trusted_time) {
        fail_connection(ESP_ERR_INVALID_STATE);
        return true;
    }
    if (result != BOBE_UPDATE_OFFER_OK) {
        ESP_LOGW(
            TAG,
            "rejected update offer result=%s manifest=%s",
            bobe_update_offer_result_name(result),
            bobe_update_manifest_result_name(manifest_result)
        );
        fail_connection(ESP_ERR_INVALID_RESPONSE);
        return true;
    }
    message->kind = BOBE_CONTROL_UPDATE_OFFER;
    return true;
}

static void process_text_message(const uint8_t *data, size_t length)
{
    if (atomic_load(&session_closing)) {
        return;
    }
    bobe_control_message_t message;
    memset(&message, 0, sizeof(message));
    if (parse_verified_update_offer(data, length, &message)) {
        if (message.kind != BOBE_CONTROL_UPDATE_OFFER ||
            !atomic_load(&session_ready)) {
            return;
        }
    } else {
        const bobe_protocol_result_t result = bobe_protocol_parse_control(
            (const char *)data,
            length,
            &message
        );
        if (result == BOBE_PROTOCOL_UNSUPPORTED) {
            return;
        }
        if (result == BOBE_PROTOCOL_VERSION_MISMATCH) {
            fail_connection(ESP_ERR_INVALID_VERSION);
            return;
        }
        if (result != BOBE_PROTOCOL_OK) {
            fail_connection(ESP_ERR_INVALID_RESPONSE);
            return;
        }
    }

    if (!atomic_load(&session_ready)) {
        if (message.kind != BOBE_CONTROL_WELCOME) {
            reject_handshake(ESP_ERR_INVALID_STATE);
            return;
        }
        if (message.data.welcome.protocol_minor <
            bobe_capabilities_protocol_minor(&hello_info.capabilities)) {
            reject_handshake(ESP_ERR_INVALID_VERSION);
            return;
        }
        if (!accept_controller_epoch(
                message.data.welcome.controller_epoch
            )) {
            reject_handshake(ESP_ERR_INVALID_STATE);
            return;
        }
        atomic_store(&session_ready, true);
        if (esp_timer_is_active(handshake_timer)) {
            ESP_ERROR_CHECK_WITHOUT_ABORT(esp_timer_stop(handshake_timer));
        }
        publish_simple(BOBE_TRANSPORT_SESSION_READY);
        return;
    } else if (message.kind == BOBE_CONTROL_WELCOME) {
        fail_connection(ESP_ERR_INVALID_STATE);
        return;
    }

    const bobe_transport_event_t event = {
        .kind = BOBE_TRANSPORT_CONTROL,
        .generation = atomic_load(&connection_generation),
        .data.control = message,
    };
    publish_event(&event);
}

static void process_binary_message(const uint8_t *data, size_t length)
{
    if (!hello_info.capabilities.speaker || atomic_load(&session_closing) ||
        !atomic_load(&session_ready) ||
        length < BOBE_MEDIA_HEADER_BYTES ||
        length > BOBE_TRANSPORT_BINARY_BYTES) {
        fail_connection(ESP_ERR_INVALID_SIZE);
        return;
    }

    bobe_media_header_t header;
    const size_t payload_size = length - BOBE_MEDIA_HEADER_BYTES;
    if (bobe_media_header_decode(data, length, &header) != BOBE_PROTOCOL_OK ||
        header.kind != BOBE_MEDIA_SPEAKER_PCM ||
        bobe_media_validate_payload(&header, payload_size) != BOBE_PROTOCOL_OK) {
        fail_connection(ESP_ERR_INVALID_RESPONSE);
        return;
    }

    const uint32_t active_stream = atomic_load(&playback_stream_id);
    if (active_stream == 0 || header.stream_id != active_stream) {
        fail_connection(ESP_ERR_INVALID_STATE);
        return;
    }
    if (atomic_load(&playback_sequence_started) &&
        header.sequence != atomic_load(&playback_expected_sequence) &&
        (header.flags & BOBE_MEDIA_FLAG_DISCONTINUITY) == 0) {
        fail_connection(ESP_ERR_INVALID_RESPONSE);
        return;
    }
    if (atomic_load(&playback_sequence_started) &&
        header.sequence != atomic_load(&playback_expected_sequence)) {
        atomic_fetch_add(&playback_sequence_gaps, 1U);
    }
    atomic_store(&playback_expected_sequence, header.sequence + 1);
    atomic_store(&playback_sequence_started, true);

    bobe_transport_event_t event = {
        .kind = BOBE_TRANSPORT_AUDIO,
        .generation = atomic_load(&connection_generation),
        .data.audio = {
            .stream_id = header.stream_id,
            .sequence = header.sequence,
            .monotonic_timestamp_us = header.monotonic_timestamp_us,
            .sample_count = BOBE_SPEAKER_FRAME_SAMPLES,
        },
    };
    memcpy(
        event.data.audio.samples,
        &data[BOBE_MEDIA_HEADER_BYTES],
        payload_size
    );
    publish_event(&event);
}

static void reset_rx_message(void)
{
    message_rx_length = 0;
    message_rx_opcode = 0;
}

static void handle_data_event(const esp_websocket_event_data_t *data)
{
    if (atomic_load(&session_closing)) {
        return;
    }
    if (data == NULL || data->data_len < 0 ||
        data->payload_offset < 0 || data->payload_len < 0 ||
        data->payload_offset > data->payload_len ||
        data->data_len > data->payload_len - data->payload_offset ||
        (data->data_len > 0 && data->data_ptr == NULL)) {
        fail_connection(ESP_ERR_INVALID_RESPONSE);
        return;
    }
    if (data->op_code >= 0x08 && data->op_code <= 0x0A) {
        return;
    }

    if ((data->op_code == 0x01 || data->op_code == 0x02) &&
        data->payload_offset == 0) {
        if (message_rx_length != 0) {
            reset_rx_message();
            fail_connection(ESP_ERR_INVALID_RESPONSE);
            return;
        }
        message_rx_opcode = data->op_code;
    } else if (data->op_code != 0x00 &&
               data->op_code != message_rx_opcode) {
        reset_rx_message();
        fail_connection(ESP_ERR_INVALID_RESPONSE);
        return;
    }

    if (message_rx_opcode == 0 ||
        message_rx_length + (size_t)data->data_len >
            sizeof(message_rx) - 1U) {
        reset_rx_message();
        fail_connection(ESP_ERR_INVALID_SIZE);
        return;
    }
    if (data->data_len > 0) {
        memcpy(
            &message_rx[message_rx_length],
            data->data_ptr,
            (size_t)data->data_len
        );
    }
    message_rx_length += (size_t)data->data_len;

    const bool last_chunk_of_frame =
        data->payload_offset + data->data_len == data->payload_len;
    if (!data->fin || !last_chunk_of_frame) {
        return;
    }

    const uint8_t opcode = message_rx_opcode;
    const size_t length = message_rx_length;
    message_rx[length] = '\0';
    reset_rx_message();

    if (opcode == 0x01) {
        process_text_message(message_rx, length);
    } else if (opcode == 0x02) {
        process_binary_message(message_rx, length);
    }
}

static void handshake_timeout_callback(void *unused)
{
    (void)unused;
    const uint32_t generation = atomic_load(&handshake_generation);
    if (generation == atomic_load(&connection_generation) &&
        atomic_load(&link_connected) &&
        !atomic_load(&session_ready)) {
        request_close_for_generation(ESP_ERR_TIMEOUT, true, generation);
    }
}

static void websocket_event_handler(
    void *unused,
    esp_event_base_t event_base,
    int32_t event_id,
    void *event_data
)
{
    (void)unused;
    (void)event_base;

    switch (event_id) {
    case WEBSOCKET_EVENT_CONNECTED:
        reset_tx_queues();
        const uint32_t generation =
            atomic_fetch_add(&connection_generation, 1) + 1U;
        atomic_store(&handshake_generation, generation);
        atomic_store(&disconnect_notified, false);
        atomic_store(&link_connected, true);
        atomic_store(&session_ready, false);
        atomic_store(&session_closing, false);
        if (!enqueue_hello()) {
            fail_connection(ESP_ERR_NO_MEM);
        }
        ESP_ERROR_CHECK_WITHOUT_ABORT(
            esp_timer_start_once(handshake_timer, UINT64_C(5000000))
        );
        publish_simple(BOBE_TRANSPORT_LINK_CONNECTED);
        break;
    case WEBSOCKET_EVENT_DISCONNECTED:
        atomic_store(&session_closing, true);
        reset_session();
        (void)publish_disconnected_once();
        break;
    case WEBSOCKET_EVENT_DATA:
        handle_data_event(event_data);
        break;
    case WEBSOCKET_EVENT_ERROR:
        atomic_store(&session_closing, true);
        reset_session();
        publish_error_for_generation(
            ESP_FAIL,
            publish_disconnected_once()
        );
        break;
    case WEBSOCKET_EVENT_CLOSED:
        atomic_store(&session_closing, true);
        reset_session();
        (void)publish_disconnected_once();
        break;
#if WS_TRANSPORT_HEADER_CALLBACK_SUPPORT
    case WEBSOCKET_EVENT_HEADER_RECEIVED:
#endif
    case WEBSOCKET_EVENT_BEFORE_CONNECT:
    case WEBSOCKET_EVENT_BEGIN:
        break;
    case WEBSOCKET_EVENT_FINISH:
        atomic_store(&client_task_running, false);
        if (atomic_load(&desired_running)) {
            atomic_store(&restart_requested, true);
            if (transport_tx_task_handle != NULL) {
                xTaskNotifyGive(transport_tx_task_handle);
            }
        }
        break;
    default:
        break;
    }
}

static bool process_control_item(const control_tx_item_t *control)
{
    const bool current =
        control->generation == atomic_load(&connection_generation) &&
        atomic_load(&link_connected) &&
        !atomic_load(&session_closing);
    const bool sent =
        current &&
        esp_websocket_client_send_text(
            client,
            control->bytes,
            (int)control->length,
            pdMS_TO_TICKS(100)
        ) >= 0;
    complete_control(control, sent);
    if (!sent && current) {
        fail_connection_for_generation(ESP_FAIL, control->generation);
        return false;
    }
    return true;
}

static void transport_tx_task(void *unused)
{
    (void)unused;
    const bool watched = esp_task_wdt_add(NULL) == ESP_OK;
    control_tx_item_t control;
    audio_tx_item_t audio;
    close_request_t close_request;
    uint8_t binary[BOBE_MEDIA_HEADER_BYTES +
                   BOBE_MIC_FRAME_SAMPLES * sizeof(int16_t)];

    for (;;) {
        if (watched) {
            ESP_ERROR_CHECK_WITHOUT_ABORT(esp_task_wdt_reset());
        }
        if (xQueueReceive(
                close_request_queue,
                &close_request,
                0
            ) == pdTRUE) {
            const bool close_still_valid =
                close_request.generation ==
                    atomic_load(&connection_generation) &&
                atomic_load(&link_connected) &&
                atomic_load(&desired_running) &&
                (!close_request.only_if_unready ||
                 !atomic_load(&session_ready));
            if (close_still_valid &&
                esp_websocket_client_close(
                    client,
                    pdMS_TO_TICKS(100)
                ) != ESP_OK) {
                atomic_store(&session_closing, true);
                reset_session();
                (void)publish_disconnected_once();
            }
            continue;
        }
        if (atomic_exchange(&restart_requested, false) &&
            atomic_load(&desired_running)) {
            const esp_err_t result = start_client_task();
            if (result != ESP_OK) {
                publish_error(result);
                (void)publish_disconnected_once();
            }
            continue;
        }
        if (!atomic_load(&link_connected)) {
            ulTaskNotifyTake(pdTRUE, pdMS_TO_TICKS(100));
            continue;
        }

        if (xQueueReceive(control_tx_queue, &control, 0) == pdTRUE) {
            (void)process_control_item(&control);
            continue;
        }

        if (xQueueReceive(
                audio_tx_queue,
                &audio,
                pdMS_TO_TICKS(20)
            ) == pdTRUE) {
            while (xQueueReceive(control_tx_queue, &control, 0) == pdTRUE) {
                if (!process_control_item(&control)) {
                    break;
                }
            }
            if (uxQueueMessagesWaiting(close_request_queue) > 0 ||
                !atomic_load(&session_ready)) {
                atomic_fetch_add(&audio_processed_count, 1U);
                continue;
            }
            if (audio.generation != atomic_load(&connection_generation) ||
                !atomic_load(&session_ready)) {
                atomic_fetch_add(&audio_processed_count, 1U);
                continue;
            }
            const uint64_t now_us = (uint64_t)esp_timer_get_time();
            if (now_us > audio.frame.monotonic_timestamp_us &&
                now_us - audio.frame.monotonic_timestamp_us >
                    BOBE_TRANSPORT_MAX_AUDIO_AGE_US) {
                atomic_store(&audio_discontinuity_pending, true);
                atomic_fetch_add(&audio_processed_count, 1U);
                continue;
            }
            const bobe_media_header_t header = {
                .protocol_major = BOBE_PROTOCOL_MAJOR,
                .kind = BOBE_MEDIA_MICROPHONE_PCM,
                .flags = atomic_exchange(
                    &audio_discontinuity_pending,
                    false
                )
                             ? BOBE_MEDIA_FLAG_DISCONTINUITY
                             : 0,
                .stream_id = audio.frame.stream_id,
                .sequence = audio.frame.sequence,
                .monotonic_timestamp_us =
                    audio.frame.monotonic_timestamp_us,
            };
            bobe_media_header_encode(&header, binary);
            memcpy(
                &binary[BOBE_MEDIA_HEADER_BYTES],
                audio.frame.samples,
                BOBE_MIC_FRAME_SAMPLES * sizeof(int16_t)
            );
            if (esp_websocket_client_send_bin(
                    client,
                    (const char *)binary,
                    sizeof(binary),
                    pdMS_TO_TICKS(100)
                ) < 0) {
                fail_connection_for_generation(
                    ESP_FAIL,
                    audio.generation
                );
            }
            atomic_fetch_add(&audio_processed_count, 1U);
        }
    }
}

static bool uri_is_secure(const char *uri)
{
    return strncmp(uri, "wss://", 6) == 0;
}

static bool uri_is_supported(const char *uri)
{
    return uri_is_secure(uri) ||
           (CONFIG_BOBE_ALLOW_INSECURE_WS &&
            strncmp(uri, "ws://", 5) == 0);
}

static char *load_nvs_string(
    const char *namespace_name,
    const char *key,
    size_t maximum_length
)
{
    nvs_handle_t nvs;
    if (nvs_open(namespace_name, NVS_READONLY, &nvs) != ESP_OK) {
        return NULL;
    }
    size_t length = 0;
    if (nvs_get_str(nvs, key, NULL, &length) != ESP_OK ||
        length < 2 || length > maximum_length) {
        nvs_close(nvs);
        return NULL;
    }
    char *value = allocate_buffer(length);
    if (value == NULL ||
        nvs_get_str(nvs, key, value, &length) != ESP_OK) {
        free(value);
        value = NULL;
    }
    nvs_close(nvs);
    return value;
}

static void clear_tls_credentials(void)
{
    free(server_ca_pem);
    free(client_cert_pem);
    free(client_key_pem);
    server_ca_pem = NULL;
    client_cert_pem = NULL;
    client_key_pem = NULL;
}

static esp_err_t load_tls_credentials(void)
{
    if (supplied_credentials) return ESP_OK;
    clear_tls_credentials();
    uint8_t active_slot = 0;
    const esp_err_t slot_result =
        bobe_storage_load_active_slot(&active_slot);
    if (slot_result == ESP_ERR_NVS_NOT_FOUND) {
        return ESP_OK;
    }
    if (slot_result != ESP_OK) {
        return slot_result;
    }
    const bobe_storage_slot_keys_t *keys =
        bobe_storage_slot_keys(active_slot);
    if (keys == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    server_ca_pem = load_nvs_string(
        BOBE_TLS_NAMESPACE,
        keys->server_ca,
        BOBE_PEM_STORAGE_MAX_BYTES
    );
    client_cert_pem = load_nvs_string(
        BOBE_TLS_NAMESPACE,
        keys->client_cert,
        BOBE_PEM_STORAGE_MAX_BYTES
    );
    client_key_pem = load_nvs_string(
        BOBE_TLS_NAMESPACE,
        keys->client_key,
        BOBE_PEM_STORAGE_MAX_BYTES
    );

    const bool any = server_ca_pem != NULL ||
                     client_cert_pem != NULL ||
                     client_key_pem != NULL;
    const bool complete = server_ca_pem != NULL &&
                          client_cert_pem != NULL &&
                          client_key_pem != NULL;
    return any && !complete ? ESP_ERR_INVALID_STATE : ESP_OK;
}

static void load_endpoint_uri(void)
{
    if (supplied_credentials) return;
    endpoint_uri[0] = '\0';
    if (CONFIG_BOBE_BODY_ENDPOINT_URI[0] != '\0') {
        const size_t configured_length =
            strlen(CONFIG_BOBE_BODY_ENDPOINT_URI);
        if (configured_length < sizeof(endpoint_uri)) {
            memcpy(
                endpoint_uri,
                CONFIG_BOBE_BODY_ENDPOINT_URI,
                configured_length + 1
            );
        } else {
            ESP_LOGE(TAG, "Configured BodyLink URI is too long");
        }
        return;
    }

    uint8_t active_slot = 0;
    const esp_err_t slot_result =
        bobe_storage_load_active_slot(&active_slot);
    if (slot_result != ESP_OK) {
        return;
    }
    const bobe_storage_slot_keys_t *keys =
        bobe_storage_slot_keys(active_slot);
    if (keys == NULL) {
        return;
    }

    nvs_handle_t nvs;
    if (nvs_open(BOBE_CLAIM_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return;
    }
    uint8_t ready = 0;
    size_t uri_length = sizeof(endpoint_uri);
    if (nvs_get_u8(nvs, BOBE_CLAIM_READY_KEY, &ready) != ESP_OK ||
        ready != 1 ||
        nvs_get_str(
            nvs,
            keys->endpoint,
            endpoint_uri,
            &uri_length
        ) != ESP_OK) {
        endpoint_uri[0] = '\0';
    }
    nvs_close(nvs);
}

static void load_controller_epoch(void)
{
    if (supplied_credentials) return;
    highest_controller_epoch = 0;
    uint8_t active_slot = 0;
    const esp_err_t slot_result =
        bobe_storage_load_active_slot(&active_slot);
    if (slot_result != ESP_OK) {
        return;
    }
    const bobe_storage_slot_keys_t *keys =
        bobe_storage_slot_keys(active_slot);
    if (keys == NULL) {
        return;
    }
    nvs_handle_t nvs;
    if (nvs_open(BOBE_CLAIM_NAMESPACE, NVS_READONLY, &nvs) == ESP_OK) {
        (void)nvs_get_u64(
            nvs,
            keys->controller_epoch,
            &highest_controller_epoch
        );
        nvs_close(nvs);
    }
}

static esp_err_t create_websocket_client(void)
{
    if (!uri_is_supported(endpoint_uri)) {
        ESP_LOGE(TAG, "Refusing unsupported BodyLink endpoint");
        return ESP_ERR_INVALID_ARG;
    }

    ESP_RETURN_ON_ERROR(
        load_tls_credentials(),
        TAG,
        "load BodyLink TLS credentials"
    );
    esp_websocket_client_config_t config = {
        .uri = endpoint_uri,
        .subprotocol = "bobe.body.v1",
        .task_core_id_set = true,
        .task_core_id = BOBE_NETWORK_CORE,
        .task_prio = BOBE_TASK_PRIORITY_WEBSOCKET,
        .task_stack = 8192,
        .buffer_size = 2048,
        .network_timeout_ms = 3000,
        .reconnect_timeout_ms = 500 + (int)(esp_random() % 1001U),
        .ping_interval_sec = 10,
        .pingpong_timeout_sec = 30,
        .disable_auto_reconnect = false,
    };
    if (uri_is_secure(endpoint_uri)) {
        if (server_ca_pem == NULL || client_cert_pem == NULL ||
            client_key_pem == NULL) {
            ESP_LOGE(
                TAG,
                "Refusing BodyLink WSS without pinned CA and client identity"
            );
            return ESP_ERR_INVALID_STATE;
        }
        config.cert_pem = server_ca_pem;
        config.client_cert = client_cert_pem;
        config.client_key = client_key_pem;
    }

    client = esp_websocket_client_init(&config);
    if (client == NULL) {
        return ESP_ERR_NO_MEM;
    }
    const esp_err_t result = esp_websocket_register_events(
        client,
        WEBSOCKET_EVENT_ANY,
        websocket_event_handler,
        NULL
    );
    if (result != ESP_OK) {
        (void)esp_websocket_client_destroy(client);
        client = NULL;
        return result;
    }
    configured = true;
    return ESP_OK;
}

static void abandon_initialization(void)
{
    if (client != NULL) {
        (void)esp_websocket_client_destroy(client);
        client = NULL;
    }
    if (handshake_timer != NULL) {
        (void)esp_timer_delete(handshake_timer);
        handshake_timer = NULL;
    }
    if (control_tx_queue != NULL) vQueueDelete(control_tx_queue);
    if (audio_tx_queue != NULL) vQueueDelete(audio_tx_queue);
    if (close_request_queue != NULL) vQueueDelete(close_request_queue);
    control_tx_queue = NULL;
    audio_tx_queue = NULL;
    close_request_queue = NULL;
    free(control_queue_storage);
    free(audio_queue_storage);
    control_queue_storage = NULL;
    audio_queue_storage = NULL;
    clear_tls_credentials();
    supplied_credentials = false;
    configured = false;
    atomic_store(&initialized, false);
}

static esp_err_t copy_supplied_credentials(const bobe_transport_credentials_t *credentials)
{
    if (credentials == NULL) return ESP_OK;
    if (credentials->endpoint == NULL || credentials->server_ca == NULL ||
        credentials->client_certificate == NULL || credentials->client_key == NULL ||
        !uri_is_secure(credentials->endpoint) || strlen(credentials->endpoint) >= sizeof(endpoint_uri) ||
        credentials->controller_epoch == 0 || credentials->controller_epoch > UINT64_C(9007199254740991)) {
        return ESP_ERR_INVALID_ARG;
    }
    const char *sources[] = {credentials->server_ca, credentials->client_certificate, credentials->client_key};
    char **destinations[] = {&server_ca_pem, &client_cert_pem, &client_key_pem};
    for (size_t index = 0; index < 3; ++index) {
        const size_t length = strnlen(sources[index], BOBE_PEM_STORAGE_MAX_BYTES);
        if (length == 0 || length >= BOBE_PEM_STORAGE_MAX_BYTES) return ESP_ERR_INVALID_SIZE;
        *destinations[index] = allocate_buffer(length + 1U);
        if (*destinations[index] == NULL) return ESP_ERR_NO_MEM;
        memcpy(*destinations[index], sources[index], length + 1U);
    }
    strcpy(endpoint_uri, credentials->endpoint);
    highest_controller_epoch = credentials->controller_epoch;
    supplied_credentials = true;
    return ESP_OK;
}

esp_err_t bobe_transport_init(
    const bobe_transport_config_t *config,
    bobe_transport_callback_t callback,
    void *context
)
{
    char validation[1024];
    if (config == NULL ||
        bobe_protocol_build_hello(validation, sizeof(validation), &config->hello) != BOBE_PROTOCOL_OK ||
        (config->verify_update != NULL && (config->build_number == 0 || config->max_image_size == 0))) {
        return ESP_ERR_INVALID_ARG;
    }
    if (atomic_exchange(&initialized, true)) {
        return ESP_ERR_INVALID_STATE;
    }
    strcpy(device_id, config->hello.device_id);
    strcpy(boot_id, config->hello.boot_id);
    strcpy(firmware_version, config->hello.firmware_version);
    strcpy(hardware_revision, config->hello.hardware_revision);
    hello_info = config->hello;
    hello_info.device_id = device_id;
    hello_info.boot_id = boot_id;
    hello_info.firmware_version = firmware_version;
    hello_info.hardware_revision = hardware_revision;
    build_number = config->build_number;
    max_image_size = config->max_image_size;
    update_verifier = config->verify_update;
    transport_callback = callback;
    transport_callback_context = context;
    atomic_store(&audio_enqueued_count, 0);
    atomic_store(&audio_processed_count, 0);

    esp_err_t result = copy_supplied_credentials(config->credentials);
    if (result != ESP_OK) {
        abandon_initialization();
        return result;
    }

    control_queue_storage = allocate_buffer(
        BOBE_TRANSPORT_CONTROL_QUEUE_DEPTH * sizeof(control_tx_item_t)
    );
    const size_t audio_depth = hello_info.capabilities.microphone ? BOBE_TRANSPORT_AUDIO_QUEUE_DEPTH : 1U;
    audio_queue_storage = allocate_buffer(
        audio_depth * sizeof(audio_tx_item_t)
    );
    if (control_queue_storage != NULL) {
        control_tx_queue = xQueueCreateStatic(
            BOBE_TRANSPORT_CONTROL_QUEUE_DEPTH,
            sizeof(control_tx_item_t),
            control_queue_storage,
            &control_tx_queue_state
        );
    }
    if (audio_queue_storage != NULL) {
        audio_tx_queue = xQueueCreateStatic(
            audio_depth,
            sizeof(audio_tx_item_t),
            audio_queue_storage,
            &audio_tx_queue_state
        );
    }
    if (control_tx_queue == NULL || audio_tx_queue == NULL) {
        abandon_initialization();
        return ESP_ERR_NO_MEM;
    }
    close_request_queue = xQueueCreateStatic(
        BOBE_TRANSPORT_CLOSE_QUEUE_DEPTH,
        sizeof(close_request_t),
        close_request_queue_storage,
        &close_request_queue_state
    );
    if (close_request_queue == NULL) {
        abandon_initialization();
        return ESP_ERR_NO_MEM;
    }

    const esp_timer_create_args_t handshake_timer_config = {
        .callback = handshake_timeout_callback,
        .name = "bobe_handshake",
    };
    result = esp_timer_create(&handshake_timer_config, &handshake_timer);
    if (result != ESP_OK) {
        abandon_initialization();
        return result;
    }
    load_controller_epoch();
    load_endpoint_uri();
    if (endpoint_uri[0] != '\0') {
        result = create_websocket_client();
        if (result != ESP_OK) {
            abandon_initialization();
            return result;
        }
    }

    if (xTaskCreatePinnedToCore(
            transport_tx_task,
            "bobe_body_tx",
            6144,
            NULL,
            BOBE_TASK_PRIORITY_TRANSPORT,
            &transport_tx_task_handle,
            BOBE_NETWORK_CORE
        ) != pdPASS) {
        abandon_initialization();
        return ESP_ERR_NO_MEM;
    }
    if (endpoint_uri[0] == '\0') {
        ESP_LOGW(TAG, "BodyLink endpoint is not configured");
    }
    return ESP_OK;
}

esp_err_t bobe_transport_start(void)
{
    if (!configured || client == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    atomic_store(&disconnect_notified, false);
    atomic_store(&desired_running, true);
    return start_client_task();
}

esp_err_t bobe_transport_stop(void)
{
    if (client == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    atomic_store(&desired_running, false);
    atomic_store(&restart_requested, false);
    if (!atomic_load(&client_task_running)) {
        reset_session();
        (void)publish_disconnected_once();
        return ESP_OK;
    }
    const esp_err_t result = esp_websocket_client_stop(client);
    if (result != ESP_OK) {
        atomic_store(&desired_running, true);
        return result;
    }
    atomic_store(&client_task_running, false);
    reset_session();
    (void)publish_disconnected_once();
    return ESP_OK;
}

bool bobe_transport_is_connected(void)
{
    return atomic_load(&session_ready);
}

uint32_t bobe_transport_generation(void)
{
    return atomic_load(&connection_generation);
}

bool bobe_transport_is_configured(void)
{
    return configured;
}

esp_err_t bobe_transport_reload_configuration(void)
{
    if (!atomic_load(&initialized) ||
        atomic_load(&desired_running) ||
        atomic_load(&client_task_running)) {
        return ESP_ERR_INVALID_STATE;
    }
    reset_session();
    if (client != NULL) {
        ESP_RETURN_ON_ERROR(
            esp_websocket_client_destroy(client),
            TAG,
            "destroy stale BodyLink client"
        );
        client = NULL;
    }
    configured = false;
    clear_tls_credentials();
    load_controller_epoch();
    load_endpoint_uri();
    return endpoint_uri[0] != '\0'
               ? create_websocket_client()
               : ESP_ERR_NOT_FOUND;
}

esp_err_t bobe_transport_configure_endpoint(const char *uri)
{
    if (uri == NULL || uri[0] == '\0' ||
        strlen(uri) >= sizeof(endpoint_uri) ||
        atomic_load(&desired_running) || !uri_is_supported(uri)) {
        return ESP_ERR_INVALID_ARG;
    }
    if (client != NULL) {
        if (strcmp(uri, endpoint_uri) == 0) {
            return ESP_OK;
        }
        const esp_err_t result =
            esp_websocket_client_set_uri(client, uri);
        if (result != ESP_OK) {
            return result;
        }
        memmove(endpoint_uri, uri, strlen(uri) + 1U);
        return ESP_OK;
    }
    memmove(endpoint_uri, uri, strlen(uri) + 1U);
    return create_websocket_client();
}

esp_err_t bobe_transport_configure_stored_endpoint(void)
{
    if (configured) {
        return ESP_OK;
    }
    load_endpoint_uri();
    return endpoint_uri[0] != '\0'
               ? bobe_transport_configure_endpoint(endpoint_uri)
               : ESP_ERR_NOT_FOUND;
}

bool bobe_transport_send_control(const char *json, size_t length)
{
    const bool queued = enqueue_control(json, length, true);
    if (!queued && atomic_load(&session_ready)) {
        fail_connection(ESP_ERR_NO_MEM);
    }
    return queued;
}

bool bobe_transport_try_send_control(const char *json, size_t length)
{
    return enqueue_control(json, length, true);
}

bool bobe_transport_send_control_sync(
    const char *json,
    size_t length,
    uint32_t timeout_ms
)
{
    if (timeout_ms == 0 || !atomic_load(&session_ready)) {
        return false;
    }
    control_completion_t *completion = calloc(1, sizeof(*completion));
    if (completion == NULL) {
        return false;
    }
    completion->done = xSemaphoreCreateBinary();
    atomic_init(&completion->references, 2);
    if (completion->done == NULL) {
        free(completion);
        return false;
    }

    control_tx_item_t item = {
        .length = length,
        .generation = atomic_load(&connection_generation),
        .completion = completion,
    };
    if (json == NULL || length == 0 ||
        length >= BOBE_TRANSPORT_TX_JSON_BYTES) {
        release_completion(completion);
        release_completion(completion);
        return false;
    }
    item.bytes = allocate_buffer(length + 1U);
    if (item.bytes == NULL) {
        release_completion(completion);
        release_completion(completion);
        return false;
    }
    memcpy(item.bytes, json, length);
    item.bytes[length] = '\0';
    if (xQueueSend(control_tx_queue, &item, 0) != pdTRUE) {
        free(item.bytes);
        release_completion(completion);
        release_completion(completion);
        return false;
    }

    const bool completed =
        xSemaphoreTake(
            completion->done,
            pdMS_TO_TICKS(timeout_ms)
        ) == pdTRUE;
    const bool sent = completed && atomic_load(&completion->sent);
    release_completion(completion);
    if (!sent && atomic_load(&session_ready)) {
        fail_connection(completed ? ESP_FAIL : ESP_ERR_TIMEOUT);
    }
    return sent;
}

bool bobe_transport_send_audio(const bobe_audio_frame_t *frame)
{
    if (!hello_info.capabilities.microphone || !atomic_load(&session_ready) || frame == NULL ||
        frame->sample_count != BOBE_MIC_FRAME_SAMPLES) {
        return false;
    }
    const audio_tx_item_t item = {
        .frame = *frame,
        .generation = atomic_load(&connection_generation),
    };
    const bool queued =
        xQueueSend(audio_tx_queue, &item, 0) == pdTRUE;
    if (!queued) {
        atomic_store(&audio_discontinuity_pending, true);
    } else {
        atomic_fetch_add(&audio_enqueued_count, 1U);
        if (transport_tx_task_handle != NULL) {
            xTaskNotifyGive(transport_tx_task_handle);
        }
    }
    return queued;
}

bool bobe_transport_flush_audio(uint32_t timeout_ms)
{
    if (audio_tx_queue == NULL) {
        return true;
    }
    const uint64_t deadline_us =
        (uint64_t)esp_timer_get_time() + (uint64_t)timeout_ms * UINT64_C(1000);
    const unsigned int target = atomic_load(&audio_enqueued_count);
    while (atomic_load(&session_ready)) {
        if (atomic_load(&audio_processed_count) == target) {
            return true;
        }
        if ((uint64_t)esp_timer_get_time() >= deadline_us) {
            return false;
        }
        vTaskDelay(pdMS_TO_TICKS(1));
    }
    return false;
}

void bobe_transport_discard_audio(void)
{
    if (audio_tx_queue != NULL) {
        audio_tx_item_t item;
        while (xQueueReceive(audio_tx_queue, &item, 0) == pdTRUE) {
            atomic_fetch_add(&audio_processed_count, 1U);
        }
    }
    atomic_store(&audio_discontinuity_pending, false);
}

void bobe_transport_set_playback_stream(uint32_t stream_id)
{
    atomic_store(&playback_stream_id, stream_id);
    atomic_store(&playback_expected_sequence, 0);
    atomic_store(&playback_sequence_gaps, 0);
    atomic_store(&playback_sequence_started, false);
}

void bobe_transport_clear_playback_stream(void)
{
    atomic_store(&playback_stream_id, 0);
    atomic_store(&playback_expected_sequence, 0);
    atomic_store(&playback_sequence_gaps, 0);
    atomic_store(&playback_sequence_started, false);
}

uint32_t bobe_transport_playback_sequence_gaps(void)
{
    return atomic_load(&playback_sequence_gaps);
}

void bobe_transport_confirm_authenticated_time(void)
{
    atomic_store(&authenticated_time_confirmed, true);
}

void bobe_transport_fail_session(esp_err_t error)
{
    fail_connection(error);
}

#include "bobe_core.h"

#include "bobe_audio.h"
#include "bobe_board.h"
#include "bobe_connectivity.h"
#include "bobe_display.h"
#include "bobe_policy.h"
#include "bobe_power.h"
#include "bobe_protocol.h"
#include "bobe_storage.h"
#include "bobe_identity.h"
#include "bobe_device_config.h"
#include "bobe_update_verify.h"
#include "bobe_task_config.h"
#include "bobe_transport.h"
#include "bobe_update.h"
#include "bobe_voice_frontend.h"
#include "esp_app_desc.h"
#include "esp_check.h"
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_ota_ops.h"
#include "esp_pm.h"
#include "esp_private/esp_clk.h"
#include "esp_random.h"
#include "esp_system.h"
#include "esp_task_wdt.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/timers.h"
#include "nvs.h"
#include "nvs_flash.h"

#include <inttypes.h>
#include <stdatomic.h>
#include <stdio.h>
#include <string.h>

static const char *TAG = "bobe_core";

#define BOBE_CAPTURE_DRAIN_TIMEOUT_MS 500U
#define BOBE_POLICY_DISPATCH_LIMIT 16U

_Static_assert(
    CONFIG_BOBE_AUDIO_PLAYBACK_CREDIT_MS <= BOBE_MAX_PLAYBACK_CREDIT_MS,
    "Playback credit exceeds the BodyLink daemon contract"
);

typedef struct {
    QueueHandle_t queue;
    TimerHandle_t battery_timer;
    TimerHandle_t health_timer;
    bobe_policy_context_t policy;
    bool ota_pending_verification;
    bool settings_dirty;
    uint8_t settings_volume;
    bobe_face_theme_id_t settings_theme;
    uint32_t reset_reason;
    int32_t temperature_millicelsius;
    uint64_t last_interaction_us;
    uint64_t notice_active_until_us;
    bobe_power_plan_t power_plan;
    bobe_battery_state_t display_battery;
    bool display_battery_valid;
    bool power_plan_applied;
    char device_id[BOBE_DEVICE_ID_MAX_BYTES];
    char boot_id[33];
} core_adapter_t;

static core_adapter_t core;
static StaticQueue_t core_queue_state;
static QueueHandle_t playback_progress_queue;
static StaticQueue_t playback_progress_queue_state;
static uint8_t playback_progress_queue_storage[
    sizeof(bobe_policy_event_t)
];
static bobe_policy_effect_batch_t policy_effects;
static atomic_uint critical_flags;
static atomic_uint playback_route_generation;
static atomic_uint latest_action_release_event_id;
static atomic_bool core_started;
static atomic_bool discovery_running;

static bool load_microphone_muted(void);
static void save_microphone_muted(bool muted);
static void update_display_status(void);
static void load_user_settings(
    uint8_t *volume,
    bobe_face_theme_id_t *theme
);
static void flush_user_settings(void);

static void log_button_event(const bobe_button_event_t *button)
{
    static const char *const button_names[] = {
        "action",
        "volume_down",
        "volume_up",
    };
    static const char *const action_names[] = {
        "pressed",
        "released",
        "long_press",
    };
    if (button->button < BOBE_BUTTON_ACTION ||
        button->button > BOBE_BUTTON_VOLUME_UP ||
        button->action < BOBE_BUTTON_PRESSED ||
        button->action > BOBE_BUTTON_LONG_PRESS) {
        return;
    }
    ESP_LOGI(
        TAG,
        "physical button event_id=%" PRIu32
        " button=%s action=%s monotonic_us=%" PRIu64,
        button->event_id,
        button_names[button->button],
        action_names[button->action],
        button->monotonic_us
    );
}

static bool post_event(const bobe_policy_event_t *event)
{
    return core.queue != NULL &&
           xQueueSend(core.queue, event, 0) == pdTRUE;
}

static void button_callback(
    const bobe_button_event_t *button,
    void *unused
)
{
    (void)unused;
    core.last_interaction_us = button->monotonic_us;
    log_button_event(button);
    if (button->button == BOBE_BUTTON_ACTION &&
        button->action == BOBE_BUTTON_RELEASED) {
        atomic_store(
            &latest_action_release_event_id,
            button->event_id
        );
        atomic_fetch_or(
            &critical_flags,
            BOBE_POLICY_CRITICAL_ACTION_RELEASE
        );
    }
    if (button->action == BOBE_BUTTON_LONG_PRESS &&
        (button->button == BOBE_BUTTON_ACTION ||
         button->button == BOBE_BUTTON_VOLUME_DOWN) &&
        bobe_board_button_is_pressed(BOBE_BUTTON_ACTION) &&
        bobe_board_button_is_pressed(BOBE_BUTTON_VOLUME_DOWN)) {
        atomic_fetch_or(
            &critical_flags,
            BOBE_POLICY_CRITICAL_SHUTDOWN
        );
    } else if (
        button->action == BOBE_BUTTON_LONG_PRESS &&
        (button->button == BOBE_BUTTON_VOLUME_UP ||
         button->button == BOBE_BUTTON_VOLUME_DOWN) &&
        bobe_board_button_is_pressed(BOBE_BUTTON_VOLUME_UP) &&
        bobe_board_button_is_pressed(BOBE_BUTTON_VOLUME_DOWN)
    ) {
        atomic_fetch_or(
            &critical_flags,
            BOBE_POLICY_CRITICAL_PROVISIONING_RECOVERY
        );
    }

    const bobe_policy_event_t event = {
        .kind = BOBE_POLICY_EVENT_BUTTON,
        .data.button.event = *button,
    };
    if (!post_event(&event)) {
        ESP_LOGW(TAG, "button event queue full");
    }
}

static void network_callback(
    const bobe_network_event_t *network,
    void *unused
)
{
    (void)unused;
    if (network->state == BOBE_NETWORK_PROVISIONING_COMPLETE) {
        atomic_fetch_or(
            &critical_flags,
            BOBE_POLICY_CRITICAL_PROVISIONING_COMPLETE
        );
        return;
    }
    if (network->state == BOBE_NETWORK_PROVISIONING_ABORTED) {
        atomic_fetch_or(
            &critical_flags,
            BOBE_POLICY_CRITICAL_PROVISIONING_ABORTED
        );
        return;
    }
    if (network->state == BOBE_NETWORK_DISCONNECTED ||
        network->state == BOBE_NETWORK_AUTH_FAILED) {
        atomic_fetch_or(
            &critical_flags,
            BOBE_POLICY_CRITICAL_NETWORK_LOST
        );
    }
    const bobe_policy_event_t event = {
        .kind = BOBE_POLICY_EVENT_NETWORK,
        .data.network = {
            .state = network->state,
            .generation = network->generation,
        },
    };
    if (!post_event(&event)) {
        ESP_LOGW(TAG, "network event queue full");
        atomic_fetch_or(
            &critical_flags,
            BOBE_POLICY_CRITICAL_EVENT_OVERFLOW
        );
    }
    update_display_status();
}

static bobe_policy_transport_kind_t policy_transport_kind(
    bobe_transport_event_kind_t kind
)
{
    switch (kind) {
    case BOBE_TRANSPORT_LINK_CONNECTED:
        return BOBE_POLICY_TRANSPORT_LINK_CONNECTED;
    case BOBE_TRANSPORT_SESSION_READY:
        return BOBE_POLICY_TRANSPORT_SESSION_READY;
    case BOBE_TRANSPORT_DISCONNECTED:
        return BOBE_POLICY_TRANSPORT_DISCONNECTED;
    case BOBE_TRANSPORT_CONTROL:
        return BOBE_POLICY_TRANSPORT_CONTROL;
    case BOBE_TRANSPORT_ERROR:
        return BOBE_POLICY_TRANSPORT_ERROR;
    case BOBE_TRANSPORT_AUDIO:
        break;
    }
    return BOBE_POLICY_TRANSPORT_ERROR;
}

static void transport_callback(
    const bobe_transport_event_t *transport,
    void *unused
)
{
    (void)unused;
    if (transport->kind == BOBE_TRANSPORT_CONTROL &&
        (transport->data.control.kind == BOBE_CONTROL_INPUT_RESULT ||
         transport->data.control.kind == BOBE_CONTROL_DISPLAY_TEXT ||
         transport->data.control.kind == BOBE_CONTROL_SENSOR_ACK)) return;
    const uint32_t current_generation = bobe_transport_generation();
    if (transport->generation != current_generation) {
        ESP_LOGW(
            TAG,
            "ignoring stale transport callback generation=%u current=%u",
            (unsigned int)transport->generation,
            (unsigned int)current_generation
        );
        return;
    }
    if (transport->kind == BOBE_TRANSPORT_ERROR) {
        ESP_LOGW(
            TAG,
            "BodyLink transport error generation=%u error=%s",
            (unsigned int)transport->generation,
            esp_err_to_name(transport->data.error)
        );
    }
    if (transport->kind == BOBE_TRANSPORT_DISCONNECTED) {
        atomic_fetch_or(
            &critical_flags,
            BOBE_POLICY_CRITICAL_TRANSPORT_LOST
        );
    }
    if (transport->kind == BOBE_TRANSPORT_AUDIO) {
        if (bobe_audio_enqueue_playback(&transport->data.audio) != ESP_OK) {
            ESP_LOGW(TAG, "playback queue rejected audio frame");
            atomic_fetch_or(
                &critical_flags,
                BOBE_POLICY_CRITICAL_PLAYBACK_FAILED
            );
        }
        return;
    }

    bobe_policy_event_t event = {
        .kind = BOBE_POLICY_EVENT_TRANSPORT,
        .data.transport = {
            .kind = policy_transport_kind(transport->kind),
            .generation = transport->generation,
        },
    };
    if (transport->kind == BOBE_TRANSPORT_CONTROL) {
        event.data.transport.control = transport->data.control;
    } else if (transport->kind == BOBE_TRANSPORT_ERROR) {
        event.data.transport.error = transport->data.error;
    }
    if (!post_event(&event)) {
        ESP_LOGW(TAG, "transport event queue full");
        atomic_fetch_or(
            &critical_flags,
            BOBE_POLICY_CRITICAL_EVENT_OVERFLOW
        );
    }
    update_display_status();
}

static bool capture_sink(
    const bobe_audio_frame_t *frame,
    void *unused
)
{
    (void)unused;
    if (bobe_voice_frontend_submit(frame)) {
        return true;
    }
    atomic_fetch_or(
        &critical_flags,
        BOBE_POLICY_CRITICAL_CAPTURE_FAILED
    );
    return false;
}

static bool enhanced_audio_sink(
    const bobe_audio_frame_t *frame,
    void *unused
)
{
    (void)unused;
    if (bobe_transport_send_audio(frame)) {
        return true;
    }
    atomic_fetch_or(
        &critical_flags,
        BOBE_POLICY_CRITICAL_CAPTURE_FAILED
    );
    return false;
}

static void voice_event_sink(
    const bobe_voice_event_t *event,
    void *unused
)
{
    (void)unused;
    if (event->kind == BOBE_VOICE_EVENT_WAKE_DETECTED) {
        core.last_interaction_us = (uint64_t)esp_timer_get_time();
        ESP_LOGI(
            TAG,
            "wake detected index=%d level=%.1f dB",
            event->wake_word_index,
            (double)event->level_db
        );
        atomic_fetch_or(
            &critical_flags,
            BOBE_POLICY_CRITICAL_WAKE_DETECTED
        );
    } else if (event->kind == BOBE_VOICE_EVENT_SPEECH_ENDED) {
        atomic_fetch_or(
            &critical_flags,
            BOBE_POLICY_CRITICAL_SPEECH_ENDED
        );
    }
}

static void playback_progress_sink(
    uint32_t stream_id,
    uint64_t played_samples,
    uint32_t credit_ms,
    void *unused
)
{
    (void)unused;
    const bobe_policy_event_t event = {
        .kind = BOBE_POLICY_EVENT_PLAYBACK_PROGRESS,
        .data.playback_progress = {
            .stream_id = stream_id,
            .route_generation =
                atomic_load(&playback_route_generation),
            .played_samples = played_samples,
            .credit_ms = credit_ms,
        },
    };
    if (playback_progress_queue != NULL) {
        (void)xQueueOverwrite(playback_progress_queue, &event);
    }
}

static void timer_callback(TimerHandle_t timer)
{
    const bobe_policy_event_t event = {
        .kind = BOBE_POLICY_EVENT_TIME_TICK,
        .data.tick.periodic_flags =
            timer == core.battery_timer
                ? BOBE_POLICY_TICK_BATTERY
                : BOBE_POLICY_TICK_HEALTH,
    };
    if (!post_event(&event)) {
        ESP_LOGW(TAG, "periodic event queue full");
    }
}

static bool make_capture_seed(bobe_policy_capture_seed_t *seed)
{
    uint8_t request_bytes[16];
    esp_fill_random(request_bytes, sizeof(request_bytes));
    memset(seed, 0, sizeof(*seed));
    seed->stream_id = esp_random();
    if (seed->stream_id == 0) {
        seed->stream_id = 1;
    }
    const int written = snprintf(
        seed->request_id,
        sizeof(seed->request_id),
        "%02x%02x%02x%02x%02x%02x%02x%02x"
        "%02x%02x%02x%02x%02x%02x%02x%02x",
        request_bytes[0],
        request_bytes[1],
        request_bytes[2],
        request_bytes[3],
        request_bytes[4],
        request_bytes[5],
        request_bytes[6],
        request_bytes[7],
        request_bytes[8],
        request_bytes[9],
        request_bytes[10],
        request_bytes[11],
        request_bytes[12],
        request_bytes[13],
        request_bytes[14],
        request_bytes[15]
    );
    seed->valid = written == 32;
    return seed->valid;
}

static void prepare_event(bobe_policy_event_t *event)
{
    event->now_us = (uint64_t)esp_timer_get_time();
    switch (event->kind) {
    case BOBE_POLICY_EVENT_BUTTON:
        event->data.button.physical_action_pressed =
            bobe_board_button_is_pressed(BOBE_BUTTON_ACTION);
        event->data.button.physical_volume_down_pressed =
            bobe_board_button_is_pressed(BOBE_BUTTON_VOLUME_DOWN);
        event->data.button.physical_volume_up_pressed =
            bobe_board_button_is_pressed(BOBE_BUTTON_VOLUME_UP);
        event->data.button.transport_connected =
            bobe_transport_is_connected();
        event->data.button.transport_configured =
            bobe_transport_is_configured();
        event->data.button.provisioning =
            bobe_connectivity_is_provisioning();
        event->data.button.current_volume = bobe_audio_volume();
        event->data.button.latest_action_release_event_id =
            atomic_load(&latest_action_release_event_id);
        memset(
            &event->data.button.capture_seed,
            0,
            sizeof(event->data.button.capture_seed)
        );
        if (event->data.button.event.button == BOBE_BUTTON_ACTION &&
            event->data.button.event.action == BOBE_BUTTON_PRESSED) {
            (void)make_capture_seed(
                &event->data.button.capture_seed
            );
        }
        break;
    case BOBE_POLICY_EVENT_NETWORK:
        event->data.network.current_generation =
            bobe_connectivity_generation();
        event->data.network.provisioning =
            bobe_connectivity_is_provisioning();
        event->data.network.network_online =
            bobe_connectivity_is_online();
        event->data.network.transport_configured =
            bobe_transport_is_configured();
        break;
    case BOBE_POLICY_EVENT_TRANSPORT:
        event->data.transport.current_generation =
            bobe_transport_generation();
        event->data.transport.audio_capturing =
            bobe_audio_is_capturing();
        event->data.transport.ota_pending_verification =
            core.ota_pending_verification;
        break;
    case BOBE_POLICY_EVENT_CRITICAL:
        event->data.critical.latest_action_release_event_id =
            atomic_load(&latest_action_release_event_id);
        event->data.critical.physical_action_pressed =
            bobe_board_button_is_pressed(BOBE_BUTTON_ACTION);
        event->data.critical.physical_volume_down_pressed =
            bobe_board_button_is_pressed(BOBE_BUTTON_VOLUME_DOWN);
        event->data.critical.physical_volume_up_pressed =
            bobe_board_button_is_pressed(BOBE_BUTTON_VOLUME_UP);
        event->data.critical.network_online =
            bobe_connectivity_is_online();
        event->data.critical.provisioning =
            bobe_connectivity_is_provisioning();
        event->data.critical.transport_connected =
            bobe_transport_is_connected();
        event->data.critical.transport_configured =
            bobe_transport_is_configured();
        memset(
            &event->data.critical.capture_seed,
            0,
            sizeof(event->data.critical.capture_seed)
        );
        if ((event->data.critical.flags &
             BOBE_POLICY_CRITICAL_WAKE_DETECTED) != 0) {
            (void)make_capture_seed(
                &event->data.critical.capture_seed
            );
        }
        break;
    case BOBE_POLICY_EVENT_TIME_TICK:
        event->data.tick.network_online =
            bobe_connectivity_is_online();
        event->data.tick.provisioning =
            bobe_connectivity_is_provisioning();
        break;
    case BOBE_POLICY_EVENT_HUB_DISCOVERY:
        event->data.discovery.current_network_generation =
            bobe_connectivity_generation();
        event->data.discovery.network_online =
            bobe_connectivity_is_online();
        event->data.discovery.transport_configured =
            bobe_transport_is_configured();
        break;
    case BOBE_POLICY_EVENT_BATTERY:
        event->data.battery.transport_connected =
            bobe_transport_is_connected();
        break;
    case BOBE_POLICY_EVENT_EFFECT_RESULT:
        event->data.effect_result.transport_generation =
            bobe_transport_generation();
        event->data.effect_result.transport_connected =
            bobe_transport_is_connected();
        event->data.effect_result.transport_configured =
            bobe_transport_is_configured();
        event->data.effect_result.network_online =
            bobe_connectivity_is_online();
        break;
    case BOBE_POLICY_EVENT_BOOT:
    case BOBE_POLICY_EVENT_VOICE_READY:
    case BOBE_POLICY_EVENT_PLAYBACK_PROGRESS:
        break;
    }
}

static void stop_wake_monitor(void)
{
    (void)bobe_audio_stop_monitor();
    bobe_voice_frontend_stop_monitor();
}

static void start_wake_monitor(void)
{
    if (!bobe_voice_frontend_wake_available()) {
        return;
    }
    if (bobe_voice_frontend_start_monitor() != ESP_OK ||
        bobe_audio_start_monitor() != ESP_OK) {
        stop_wake_monitor();
        ESP_LOGW(TAG, "wake monitor could not be started");
    }
}

static bool send_capture_request(
    const bobe_policy_effect_t *effect
)
{
    char json[512];
    const char *trigger =
        effect->data.capture_request.trigger ==
                BOBE_POLICY_TRIGGER_WAKE_WORD
            ? "wake_word"
            : "action_button";
    return bobe_protocol_build_capture_request(
               json,
               sizeof(json),
               effect->data.capture_request.request_id,
               effect->data.capture_request.stream_id,
               trigger
           ) == BOBE_PROTOCOL_OK &&
           bobe_transport_send_control(json, strlen(json));
}

static bool send_capture_state(
    const bobe_policy_effect_t *effect
)
{
    char json[512];
    return bobe_protocol_build_capture_state(
               json,
               sizeof(json),
               effect->data.capture_state.open,
               effect->data.capture_state.stream_id,
               effect->data.capture_state.lease_id,
               effect->data.capture_state.turn_id,
               bobe_policy_reason_name(
                   effect->data.capture_state.reason
               )
           ) == BOBE_PROTOCOL_OK &&
           bobe_transport_send_control(json, strlen(json));
}

static void send_capture_cancel(
    const bobe_policy_effect_t *effect
)
{
    char json[256];
    if (bobe_protocol_build_capture_cancel(
            json,
            sizeof(json),
            effect->data.capture_cancel.request_id,
            effect->data.capture_cancel.stream_id
        ) == BOBE_PROTOCOL_OK) {
        (void)bobe_transport_send_control(json, strlen(json));
    }
}

static bool send_playback_ready(
    const bobe_policy_effect_t *effect
)
{
    char json[256];
    return bobe_protocol_build_playback_ready(
               json,
               sizeof(json),
               effect->data.playback.stream_id,
               effect->data.playback.lease_id,
               effect->data.playback.turn_id,
               CONFIG_BOBE_AUDIO_PLAYBACK_CREDIT_MS
           ) == BOBE_PROTOCOL_OK &&
           bobe_transport_send_control_sync(
               json,
               strlen(json),
               250
           );
}

static void send_playback_ack(
    const bobe_policy_effect_t *effect
)
{
    char json[320];
    if (bobe_protocol_build_playback_ack(
            json,
            sizeof(json),
            effect->data.playback.stream_id,
            effect->data.playback.lease_id,
            effect->data.playback.turn_id,
            effect->data.playback.played_samples,
            effect->data.playback.credit_ms
        ) == BOBE_PROTOCOL_OK) {
        (void)bobe_transport_send_control(json, strlen(json));
    }
}

static bool send_playback_drained(
    const bobe_policy_effect_t *effect
)
{
    char json[256];
    return bobe_protocol_build_playback_drained(
               json,
               sizeof(json),
               effect->data.playback.stream_id,
               effect->data.playback.lease_id,
               effect->data.playback.turn_id
           ) == BOBE_PROTOCOL_OK &&
           bobe_transport_send_control_sync(
               json,
               strlen(json),
               250
           );
}

static bool send_playback_failed(
    const bobe_policy_effect_t *effect
)
{
    char json[256];
    return bobe_protocol_build_playback_failed(
               json,
               sizeof(json),
               effect->data.playback.stream_id,
               effect->data.playback.lease_id,
               effect->data.playback.turn_id,
               bobe_policy_reason_name(effect->data.playback.reason)
           ) == BOBE_PROTOCOL_OK &&
           bobe_transport_send_control_sync(
               json,
               strlen(json),
               250
           );
}

static void send_button(const bobe_policy_effect_t *effect)
{
    char json[512];
    if (bobe_transport_is_connected() &&
        bobe_protocol_build_button_event(
            json,
            sizeof(json),
            core.boot_id,
            &effect->data.button
        ) == BOBE_PROTOCOL_OK) {
        (void)bobe_transport_send_control(json, strlen(json));
    }
}

static void send_battery(const bobe_policy_effect_t *effect)
{
    char json[384];
    if (bobe_transport_is_connected() &&
        bobe_protocol_build_battery_state(
            json,
            sizeof(json),
            &effect->data.battery
        ) == BOBE_PROTOCOL_OK) {
        (void)bobe_transport_send_control(json, strlen(json));
    }
}

static void discovery_task(void *unused)
{
    const uint32_t network_generation = (uint32_t)(uintptr_t)unused;
    bobe_hub_candidate_t candidate;
    const esp_err_t result = bobe_connectivity_discover_hub(
        &candidate,
        3000
    );
    bobe_policy_event_t event = {
        .kind = BOBE_POLICY_EVENT_HUB_DISCOVERY,
        .data.discovery = {
            .success = result == ESP_OK,
            .network_generation = network_generation,
        },
    };
    if (result == ESP_OK) {
        (void)snprintf(
            event.data.discovery.uri,
            sizeof(event.data.discovery.uri),
            "%s",
            candidate.uri
        );
    }
    atomic_store(&discovery_running, false);
    if (!post_event(&event)) {
        atomic_fetch_or(
            &critical_flags,
            BOBE_POLICY_CRITICAL_EVENT_OVERFLOW
        );
    }
    vTaskDelete(NULL);
}

static esp_err_t start_discovery(uint32_t network_generation)
{
    if (atomic_exchange(&discovery_running, true)) {
        return ESP_OK;
    }
    if (xTaskCreate(
            discovery_task,
            "bobe_discovery",
            4096,
            (void *)(uintptr_t)network_generation,
            BOBE_TASK_PRIORITY_BACKGROUND,
            NULL
        ) != pdPASS) {
        atomic_store(&discovery_running, false);
        return ESP_ERR_NO_MEM;
    }
    return ESP_OK;
}

static uint8_t running_ota_slot(void)
{
    const esp_partition_t *const running = esp_ota_get_running_partition();
    return running != NULL && running->subtype == ESP_PARTITION_SUBTYPE_APP_OTA_1
               ? UINT8_C(1)
               : UINT8_C(0);
}

static void send_update_state(const bobe_policy_effect_t *effect)
{
    char json[256];
    const char *const reason =
        effect->data.update_state.reason[0] != '\0'
            ? effect->data.update_state.reason
            : NULL;
    if (bobe_protocol_build_update_state(
            json,
            sizeof(json),
            effect->data.update_state.phase,
            effect->data.update_state.progress_percent,
            reason
        ) == BOBE_PROTOCOL_OK) {
        (void)bobe_transport_send_control(json, strlen(json));
    }
}

static void report_health(void)
{
    flush_user_settings();
    bobe_display_stats_t display_stats;
    bobe_display_get_stats(&display_stats);
    const bobe_health_info_t health = {
        .uptime_s =
            (uint32_t)(esp_timer_get_time() / UINT64_C(1000000)),
        .wifi_rssi_dbm = bobe_connectivity_rssi_dbm(),
        .free_internal_heap =
            heap_caps_get_free_size(MALLOC_CAP_INTERNAL),
        .minimum_internal_heap =
            heap_caps_get_minimum_free_size(MALLOC_CAP_INTERNAL),
        .free_psram = heap_caps_get_free_size(MALLOC_CAP_SPIRAM),
        .audio_rx_overruns = bobe_audio_capture_overruns(),
        .audio_tx_underruns = bobe_audio_playback_underruns(),
        .lifecycle = core.policy.state.lifecycle,
        .turn = core.policy.state.turn,
        .microphone_muted = core.policy.state.microphone_muted,
        .app_version = esp_app_get_description()->version,
        .build_number = CONFIG_BOBE_BUILD_NUMBER,
        .running_slot = running_ota_slot(),
        .ota_pending_verification = core.ota_pending_verification,
        .reset_reason = core.reset_reason,
        .transport_generation = bobe_transport_generation(),
        .core_queue_depth = (uint32_t)uxQueueMessagesWaiting(core.queue),
        .core_queue_capacity = BOBE_POLICY_EVENT_QUEUE_CAPACITY,
        .core_stack_free_words =
            (uint32_t)uxTaskGetStackHighWaterMark(NULL),
        .display_stack_free_words = display_stats.stack_free_words,
        .display_flush_submitted = display_stats.flush_submitted,
        .display_flush_errors = display_stats.flush_errors,
        .display_flush_timeouts = display_stats.flush_timeouts,
        .display_flush_last_us = display_stats.flush_last_us,
        .display_flush_max_us = display_stats.flush_max_us,
        .playback_sequence_gaps =
            bobe_transport_playback_sequence_gaps(),
        .temperature_millicelsius = core.temperature_millicelsius,
        .cpu_frequency_mhz = (uint32_t)esp_clk_cpu_freq() / 1000000U,
        .display_frame_rate = display_stats.frame_rate,
        .backlight_percent = core.power_plan.backlight_percent,
    };
    ESP_LOGI(
        TAG,
        "health uptime=%us lifecycle=%s turn=%s muted=%s wifi=%ddBm "
        "internal_free=%u minimum=%u largest=%u psram_free=%u "
        "audio_overruns=%u underruns=%u core_queue=%u "
        "stack_free_words=%u soc_temp=%.1fC cpu=%uMHz display_fps=%u "
        "backlight=%u%%",
        (unsigned int)health.uptime_s,
        bobe_lifecycle_state_name(health.lifecycle),
        bobe_turn_state_name(health.turn),
        health.microphone_muted ? "true" : "false",
        health.wifi_rssi_dbm,
        (unsigned int)health.free_internal_heap,
        (unsigned int)health.minimum_internal_heap,
        (unsigned int)heap_caps_get_largest_free_block(
            MALLOC_CAP_INTERNAL
        ),
        (unsigned int)health.free_psram,
        (unsigned int)health.audio_rx_overruns,
        (unsigned int)health.audio_tx_underruns,
        (unsigned int)uxQueueMessagesWaiting(core.queue),
        (unsigned int)uxTaskGetStackHighWaterMark(NULL),
        (double)health.temperature_millicelsius / 1000.0,
        (unsigned int)health.cpu_frequency_mhz,
        (unsigned int)health.display_frame_rate,
        (unsigned int)health.backlight_percent
    );
    char json[1280];
    if (bobe_transport_is_connected() &&
        bobe_protocol_build_health(
            json,
            sizeof(json),
            &health
        ) == BOBE_PROTOCOL_OK) {
        (void)bobe_transport_send_control(json, strlen(json));
    }
}

static esp_err_t policy_failure_error(
    bobe_policy_failure_t failure
)
{
    switch (failure) {
    case BOBE_POLICY_FAILURE_TIMEOUT:
        return ESP_ERR_TIMEOUT;
    case BOBE_POLICY_FAILURE_NO_MEMORY:
        return ESP_ERR_NO_MEM;
    case BOBE_POLICY_FAILURE_GENERIC:
        return ESP_FAIL;
    }
    return ESP_FAIL;
}

static bobe_policy_result_t result_from_error(esp_err_t result)
{
    if (result == ESP_OK) {
        return BOBE_POLICY_RESULT_OK;
    }
    return result == ESP_ERR_TIMEOUT
               ? BOBE_POLICY_RESULT_TIMEOUT
               : BOBE_POLICY_RESULT_FAILED;
}

static void update_install_task(void *argument)
{
    bobe_update_manifest_t *const manifest = argument;
    const esp_err_t result = bobe_update_install(manifest);
    free(manifest);
    const bobe_policy_event_t event = {
        .kind = BOBE_POLICY_EVENT_EFFECT_RESULT,
        .data.effect_result = {
            .operation = BOBE_POLICY_OPERATION_UPDATE_INSTALL,
            .result = result == ESP_OK
                          ? BOBE_POLICY_RESULT_OK
                          : BOBE_POLICY_RESULT_FAILED,
        },
    };
    if (!post_event(&event)) {
        atomic_fetch_or(
            &critical_flags,
            BOBE_POLICY_CRITICAL_EVENT_OVERFLOW
        );
    }
    vTaskDelete(NULL);
}

static esp_err_t start_update_install(
    const bobe_update_manifest_t *manifest
)
{
    bobe_update_manifest_t *const copy = heap_caps_malloc(
        sizeof(*copy),
        MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT
    );
    if (copy == NULL) {
        return ESP_ERR_NO_MEM;
    }
    *copy = *manifest;
    if (xTaskCreatePinnedToCore(
            update_install_task,
            "bobe_update",
            8192,
            copy,
            BOBE_TASK_PRIORITY_BACKGROUND,
            NULL,
            BOBE_NETWORK_CORE
        ) != pdPASS) {
        free(copy);
        return ESP_ERR_NO_MEM;
    }
    return ESP_OK;
}

static bool execute_effect(
    const bobe_policy_effect_t *effect,
    bobe_policy_event_t *generated_event
)
{
    esp_err_t result = ESP_OK;
    bool boolean_result = true;
    bool has_result = effect->operation != BOBE_POLICY_OPERATION_NONE;

    switch (effect->kind) {
    case BOBE_POLICY_EFFECT_DISPLAY_FACE:
        if (core.policy.state.turn != BOBE_TURN_IDLE) {
            core.last_interaction_us = (uint64_t)esp_timer_get_time();
        }
        bobe_display_set_face(&effect->data.face);
        break;
    case BOBE_POLICY_EFFECT_DISPLAY_THEME:
        bobe_display_set_theme(&effect->data.theme);
        if (effect->data.theme.state_version > 0) {
            core.settings_theme = effect->data.theme.theme;
            core.settings_dirty = true;
        }
        break;
    case BOBE_POLICY_EFFECT_DISPLAY_MICROPHONE:
        if (effect->data.microphone_active) {
            core.last_interaction_us = (uint64_t)esp_timer_get_time();
        }
        bobe_display_set_microphone_active(
            effect->data.microphone_active
        );
        break;
    case BOBE_POLICY_EFFECT_DISPLAY_NOTICE: {
        const uint64_t now_us = (uint64_t)esp_timer_get_time();
        core.last_interaction_us = now_us;
        core.notice_active_until_us = now_us +
            (uint64_t)effect->data.notice.ttl_ms * UINT64_C(1000);
        bobe_display_show_notice(&effect->data.notice);
        break;
    }
    case BOBE_POLICY_EFFECT_WAKE_MONITOR_START:
        start_wake_monitor();
        break;
    case BOBE_POLICY_EFFECT_WAKE_MONITOR_STOP:
        stop_wake_monitor();
        break;
    case BOBE_POLICY_EFFECT_AUDIO_CAPTURE_START:
        result = bobe_audio_start_capture(effect->data.stream_id);
        break;
    case BOBE_POLICY_EFFECT_AUDIO_CAPTURE_STOP:
        result = bobe_audio_stop_capture();
        break;
    case BOBE_POLICY_EFFECT_VOICE_STREAM_BEGIN:
        result = bobe_voice_frontend_begin_stream(
            effect->data.stream_id
        );
        break;
    case BOBE_POLICY_EFFECT_VOICE_STREAM_END:
        result = bobe_voice_frontend_end_stream();
        break;
    case BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_OPEN:
        result = bobe_audio_open_playback(effect->data.stream_id);
        break;
    case BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_CLOSE:
        result = bobe_audio_close_playback(effect->data.stream_id);
        break;
    case BOBE_POLICY_EFFECT_AUDIO_PLAYBACK_ABORT:
        bobe_audio_abort_playback();
        break;
    case BOBE_POLICY_EFFECT_AUDIO_SET_MUTE:
        bobe_audio_set_muted(effect->data.mute.muted);
        update_display_status();
        if (effect->data.mute.persist) {
            save_microphone_muted(effect->data.mute.muted);
        }
        break;
    case BOBE_POLICY_EFFECT_AUDIO_SET_VOLUME:
        bobe_audio_set_volume(effect->data.volume);
        core.settings_volume = effect->data.volume;
        core.settings_dirty = true;
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_REQUEST:
        boolean_result = send_capture_request(effect);
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_STATE:
        boolean_result = send_capture_state(effect);
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_CAPTURE_CANCEL:
        send_capture_cancel(effect);
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_READY:
        boolean_result = send_playback_ready(effect);
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_ACK:
        send_playback_ack(effect);
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_DRAINED:
        boolean_result = send_playback_drained(effect);
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_PLAYBACK_FAILED:
        boolean_result = send_playback_failed(effect);
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_BUTTON:
        send_button(effect);
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_START:
        result = bobe_transport_start();
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_STOP:
        result = bobe_transport_stop();
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_RELOAD:
        result = bobe_transport_reload_configuration();
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_CONFIGURE_STORED:
        result = bobe_transport_configure_stored_endpoint();
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_CONFIGURE_ENDPOINT:
        result = bobe_transport_configure_endpoint(
            effect->data.endpoint.uri
        );
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_DISCARD_AUDIO:
        bobe_transport_discard_audio();
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_FLUSH_AUDIO:
        boolean_result = bobe_transport_flush_audio(
            BOBE_CAPTURE_DRAIN_TIMEOUT_MS
        );
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_SET_PLAYBACK_ROUTE:
        bobe_transport_set_playback_stream(effect->data.stream_id);
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_CLEAR_PLAYBACK_ROUTE:
        bobe_transport_clear_playback_stream();
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_FAIL_SESSION:
        bobe_transport_fail_session(
            policy_failure_error(effect->data.failure)
        );
        break;
    case BOBE_POLICY_EFFECT_CONNECTIVITY_CONFIRM_TIME:
        bobe_connectivity_confirm_authenticated_time();
        bobe_transport_confirm_authenticated_time();
        break;
    case BOBE_POLICY_EFFECT_CONNECTIVITY_OPEN_PROVISIONING:
        result = bobe_connectivity_open_provisioning_window();
        break;
    case BOBE_POLICY_EFFECT_CONNECTIVITY_DISCOVER_HUB:
        result = start_discovery(
            effect->data.discovery.network_generation
        );
        break;
    case BOBE_POLICY_EFFECT_OTA_CONFIRM:
        result = esp_ota_mark_app_valid_cancel_rollback();
        if (result == ESP_OK) {
            core.ota_pending_verification = false;
        } else {
            ESP_LOGE(
                TAG,
                "failed to confirm pending OTA image: %s",
                esp_err_to_name(result)
            );
        }
        break;
    case BOBE_POLICY_EFFECT_OTA_ROLLBACK:
        ESP_LOGE(TAG, "OTA confirmation deadline expired; rolling back");
        result = esp_ota_mark_app_invalid_rollback_and_reboot();
        break;
    case BOBE_POLICY_EFFECT_UPDATE_INSTALL:
        result = start_update_install(&effect->data.update);
        has_result = result != ESP_OK;
        break;
    case BOBE_POLICY_EFFECT_TRANSPORT_UPDATE_STATE:
        if (strcmp(effect->data.update_state.phase, "rebooting") == 0) {
            char json[256];
            if (bobe_protocol_build_update_state(
                    json,
                    sizeof(json),
                    effect->data.update_state.phase,
                    effect->data.update_state.progress_percent,
                    NULL
                ) == BOBE_PROTOCOL_OK) {
                (void)bobe_transport_send_control_sync(
                    json,
                    strlen(json),
                    1000
                );
            }
        } else {
            send_update_state(effect);
        }
        break;
    case BOBE_POLICY_EFFECT_UPDATE_REBOOT:
        flush_user_settings();
        esp_restart();
        break;
    case BOBE_POLICY_EFFECT_BOARD_SHUTDOWN:
        flush_user_settings();
        bobe_board_request_shutdown();
        break;
    case BOBE_POLICY_EFFECT_READ_BATTERY: {
        bobe_policy_event_t battery_event = {
            .kind = BOBE_POLICY_EVENT_BATTERY,
        };
        result = bobe_board_read_battery(
            &battery_event.data.battery.state
        );
        if (result == ESP_OK) {
            core.display_battery = battery_event.data.battery.state;
            core.display_battery_valid = true;
            update_display_status();
        }
        float temperature_celsius = 0;
        if (bobe_board_read_temperature(&temperature_celsius) == ESP_OK) {
            core.temperature_millicelsius =
                (int32_t)(temperature_celsius * 1000.0f);
        }
        battery_event.data.battery.valid = result == ESP_OK;
        *generated_event = battery_event;
        return true;
    }
    case BOBE_POLICY_EFFECT_SEND_BATTERY:
        send_battery(effect);
        break;
    case BOBE_POLICY_EFFECT_REPORT_HEALTH:
        report_health();
        break;
    }

    if (!has_result) {
        return false;
    }
    bobe_policy_result_t policy_result = result_from_error(result);
    if (result == ESP_OK && !boolean_result) {
        policy_result = BOBE_POLICY_RESULT_FAILED;
    }
    *generated_event = (bobe_policy_event_t){
        .kind = BOBE_POLICY_EVENT_EFFECT_RESULT,
        .data.effect_result = {
            .operation = effect->operation,
            .result = policy_result,
        },
    };
    if (effect->operation == BOBE_POLICY_OPERATION_TRANSPORT_START &&
        policy_result != BOBE_POLICY_RESULT_OK &&
        core.policy.pending_transport_start_is_retry) {
        generated_event->data.effect_result.retry_random = esp_random();
    }
    return true;
}

static void dispatch_policy_event(const bobe_policy_event_t *source)
{
    bobe_policy_event_t event = *source;
    for (size_t depth = 0; depth < BOBE_POLICY_DISPATCH_LIMIT; ++depth) {
        prepare_event(&event);
        if (event.kind == BOBE_POLICY_EVENT_TRANSPORT &&
            event.data.transport.generation !=
                event.data.transport.current_generation) {
            ESP_LOGW(
                TAG,
                "ignoring stale transport event generation=%u current=%u",
                (unsigned int)event.data.transport.generation,
                (unsigned int)event.data.transport.current_generation
            );
        } else if (
            event.kind == BOBE_POLICY_EVENT_NETWORK &&
            event.data.network.state !=
                BOBE_NETWORK_PROVISIONING_COMPLETE &&
            event.data.network.generation !=
                event.data.network.current_generation
        ) {
            ESP_LOGW(
                TAG,
                "ignoring stale network event generation=%u current=%u",
                (unsigned int)event.data.network.generation,
                (unsigned int)event.data.network.current_generation
            );
        }

        const bobe_lifecycle_state_t old_lifecycle =
            core.policy.state.lifecycle;
        const bobe_turn_state_t old_turn = core.policy.state.turn;
        if (!bobe_policy_reduce(
                &core.policy,
                &event,
                &policy_effects
            )) {
            ESP_LOGE(TAG, "core policy effect bound exceeded");
            atomic_fetch_or(
                &critical_flags,
                BOBE_POLICY_CRITICAL_EVENT_OVERFLOW
            );
            return;
        }
        if (core.policy.state.lifecycle != old_lifecycle) {
            ESP_LOGI(
                TAG,
                "lifecycle=%s",
                bobe_lifecycle_state_name(
                    core.policy.state.lifecycle
                )
            );
        }
        if (core.policy.state.turn != old_turn) {
            ESP_LOGI(
                TAG,
                "turn=%s",
                bobe_turn_state_name(core.policy.state.turn)
            );
        }
        atomic_store(
            &playback_route_generation,
            core.policy.playback_route_generation
        );

        bool generated = false;
        bobe_policy_event_t next_event = {0};
        for (size_t index = 0;
             index < policy_effects.count;
             ++index) {
            bobe_policy_event_t candidate = {0};
            if (!execute_effect(
                    &policy_effects.items[index],
                    &candidate
                )) {
                continue;
            }
            if (generated) {
                ESP_LOGE(TAG, "policy produced multiple result events");
                atomic_fetch_or(
                    &critical_flags,
                    BOBE_POLICY_CRITICAL_EVENT_OVERFLOW
                );
                return;
            }
            if (policy_effects.items[index].operation !=
                    BOBE_POLICY_OPERATION_NONE &&
                index + 1U != policy_effects.count) {
                ESP_LOGE(TAG, "policy result effect was not terminal");
                atomic_fetch_or(
                    &critical_flags,
                    BOBE_POLICY_CRITICAL_EVENT_OVERFLOW
                );
                return;
            }
            generated = true;
            next_event = candidate;
        }
        if (!generated) {
            return;
        }
        event = next_event;
    }
    ESP_LOGE(TAG, "core policy dispatch limit exceeded");
    atomic_fetch_or(
        &critical_flags,
        BOBE_POLICY_CRITICAL_EVENT_OVERFLOW
    );
}

static void update_display_status(void)
{
    int rssi = bobe_connectivity_rssi_dbm();
    if (rssi < INT8_MIN) rssi = INT8_MIN;
    if (rssi > INT8_MAX) rssi = INT8_MAX;
    const bobe_display_status_t status = {
        .battery_percent = core.display_battery.percent,
        .wifi_rssi_dbm = (int8_t)rssi,
        .battery_valid = core.display_battery_valid,
        .charging = core.display_battery.charging,
        .wifi_connected = bobe_connectivity_is_online() &&
                          bobe_transport_is_connected(),
        .microphone_muted = core.policy.state.microphone_muted,
    };
    bobe_display_set_status(&status);
}

static void apply_power_plan(uint64_t now_us)
{
    const bobe_power_input_t input = {
        .lifecycle = core.policy.state.lifecycle,
        .turn = core.policy.state.turn,
        .now_us = now_us,
        .last_interaction_us = core.last_interaction_us,
        .microphone_active = bobe_audio_is_capturing(),
        .notice_active = now_us < core.notice_active_until_us,
    };
    bobe_power_plan_t plan;
    bobe_power_plan(&input, &plan);
    if (core.power_plan_applied &&
        memcmp(&plan, &core.power_plan, sizeof(plan)) == 0) {
        return;
    }
    core.power_plan = plan;
    core.power_plan_applied = true;
    ESP_ERROR_CHECK_WITHOUT_ABORT(
        bobe_board_set_backlight(plan.backlight_percent)
    );
    bobe_display_set_frame_rate(plan.frame_rate);
    ESP_LOGI(
        TAG,
        "power backlight=%u%% fps=%u lifecycle=%s turn=%s",
        (unsigned int)plan.backlight_percent,
        (unsigned int)plan.frame_rate,
        bobe_lifecycle_state_name(input.lifecycle),
        bobe_turn_state_name(input.turn)
    );
}

static void core_task(void *unused)
{
    (void)unused;
    const bool watched = esp_task_wdt_add(NULL) == ESP_OK;
    bobe_policy_event_t event;
    for (;;) {
        if (watched) {
            ESP_ERROR_CHECK_WITHOUT_ABORT(esp_task_wdt_reset());
        }

        const bobe_policy_event_t tick = {
            .kind = BOBE_POLICY_EVENT_TIME_TICK,
        };
        dispatch_policy_event(&tick);
        apply_power_plan((uint64_t)esp_timer_get_time());

        const uint32_t flags = atomic_exchange(&critical_flags, 0);
        if (flags != 0) {
            const bobe_policy_event_t critical = {
                .kind = BOBE_POLICY_EVENT_CRITICAL,
                .data.critical.flags = flags,
            };
            dispatch_policy_event(&critical);
        }

        if (xQueueReceive(
                playback_progress_queue,
                &event,
                0
            ) == pdTRUE) {
            dispatch_policy_event(&event);
        }
        if (xQueueReceive(
                core.queue,
                &event,
                pdMS_TO_TICKS(20)
            ) == pdTRUE) {
            dispatch_policy_event(&event);
        }
    }
}

static void load_user_settings(
    uint8_t *volume,
    bobe_face_theme_id_t *theme
)
{
    *volume = 70;
    *theme = BOBE_FACE_THEME_DEFAULT;
    nvs_handle_t nvs;
    if (nvs_open(BOBE_SETTINGS_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return;
    }
    uint8_t stored_volume = 70;
    uint8_t stored_theme = BOBE_FACE_THEME_DEFAULT;
    if (nvs_get_u8(nvs, BOBE_VOLUME_KEY, &stored_volume) == ESP_OK &&
        stored_volume <= 100) {
        *volume = stored_volume;
    }
    if (nvs_get_u8(nvs, BOBE_FACE_THEME_KEY, &stored_theme) == ESP_OK &&
        stored_theme > BOBE_FACE_THEME_DEFAULT &&
        stored_theme < BOBE_FACE_THEME_COUNT) {
        *theme = (bobe_face_theme_id_t)stored_theme;
    }
    nvs_close(nvs);
}

static void flush_user_settings(void)
{
    if (!core.settings_dirty) {
        return;
    }
    nvs_handle_t nvs;
    esp_err_t result = nvs_open(
        BOBE_SETTINGS_NAMESPACE,
        NVS_READWRITE,
        &nvs
    );
    if (result != ESP_OK) {
        ESP_LOGW(TAG, "failed to open user settings: %s", esp_err_to_name(result));
        return;
    }
    result = nvs_set_u8(nvs, BOBE_VOLUME_KEY, core.settings_volume);
    if (result == ESP_OK) {
        result = nvs_set_u8(
            nvs,
            BOBE_FACE_THEME_KEY,
            (uint8_t)core.settings_theme
        );
    }
    if (result == ESP_OK) {
        result = nvs_commit(nvs);
    }
    nvs_close(nvs);
    if (result == ESP_OK) {
        core.settings_dirty = false;
    } else {
        ESP_LOGW(TAG, "failed to persist user settings: %s", esp_err_to_name(result));
    }
}

static bool load_microphone_muted(void)
{
    nvs_handle_t nvs;
    if (nvs_open(BOBE_SETTINGS_NAMESPACE, NVS_READONLY, &nvs) != ESP_OK) {
        return false;
    }
    uint8_t value = 0;
    const esp_err_t result = nvs_get_u8(
        nvs,
        BOBE_MICROPHONE_MUTED_KEY,
        &value
    );
    nvs_close(nvs);
    return result == ESP_OK && value != 0;
}

static void save_microphone_muted(bool muted)
{
    nvs_handle_t nvs;
    esp_err_t result = nvs_open(
        BOBE_SETTINGS_NAMESPACE,
        NVS_READWRITE,
        &nvs
    );
    if (result != ESP_OK) {
        ESP_LOGW(TAG, "failed to open settings: %s", esp_err_to_name(result));
        return;
    }
    result = nvs_set_u8(
        nvs,
        BOBE_MICROPHONE_MUTED_KEY,
        muted ? UINT8_C(1) : UINT8_C(0)
    );
    if (result == ESP_OK) {
        result = nvs_commit(nvs);
    }
    nvs_close(nvs);
    if (result != ESP_OK) {
        ESP_LOGW(
            TAG,
            "failed to persist microphone mute: %s",
            esp_err_to_name(result)
        );
    }
}

static esp_err_t create_identity(void)
{
    const esp_err_t result = bobe_identity_device_id(core.device_id, sizeof(core.device_id));
    if (result == ESP_OK) bobe_identity_boot_id(core.boot_id);
    return result;
}

static esp_err_t detect_pending_ota(void)
{
    const esp_partition_t *running = esp_ota_get_running_partition();
    esp_ota_img_states_t state;
    const esp_err_t result = esp_ota_get_state_partition(running, &state);
    if (result == ESP_ERR_NOT_SUPPORTED) {
        return ESP_OK;
    }
    if (result != ESP_OK) {
        return result;
    }
    core.ota_pending_verification = state == ESP_OTA_IMG_PENDING_VERIFY;
    return ESP_OK;
}

static void log_boot_stage(const char *stage)
{
    ESP_LOGI(
        TAG,
        "boot stage=%s internal_free=%u internal_largest=%u "
        "dma_largest=%u psram_free=%u",
        stage,
        (unsigned int)heap_caps_get_free_size(MALLOC_CAP_INTERNAL),
        (unsigned int)heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL),
        (unsigned int)heap_caps_get_largest_free_block(
            MALLOC_CAP_INTERNAL | MALLOC_CAP_DMA
        ),
        (unsigned int)heap_caps_get_free_size(MALLOC_CAP_SPIRAM)
    );
}

esp_err_t bobe_core_start(void)
{
    if (atomic_exchange(&core_started, true)) {
        return ESP_ERR_INVALID_STATE;
    }
    memset(&core, 0, sizeof(core));
    bobe_policy_init(&core.policy);
    core.last_interaction_us = (uint64_t)esp_timer_get_time();
    const esp_pm_config_t power_management = {
        .max_freq_mhz = CONFIG_ESP_DEFAULT_CPU_FREQ_MHZ,
        .min_freq_mhz = 80,
        .light_sleep_enable = false,
    };
    ESP_RETURN_ON_ERROR(
        esp_pm_configure(&power_management),
        TAG,
        "configure dynamic frequency scaling"
    );
    core.reset_reason = (uint32_t)esp_reset_reason();
    ESP_RETURN_ON_ERROR(nvs_flash_init(), TAG, "initialize NVS; stored data preserved on failure");
    ESP_RETURN_ON_ERROR(
        detect_pending_ota(),
        TAG,
        "inspect OTA state"
    );
    ESP_RETURN_ON_ERROR(create_identity(), TAG, "create boot identity");

    uint8_t *core_queue_storage = heap_caps_malloc(
        BOBE_POLICY_EVENT_QUEUE_CAPACITY * sizeof(bobe_policy_event_t),
        MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT
    );
    if (core_queue_storage != NULL) {
        core.queue = xQueueCreateStatic(
            BOBE_POLICY_EVENT_QUEUE_CAPACITY,
            sizeof(bobe_policy_event_t),
            core_queue_storage,
            &core_queue_state
        );
    }
    if (core.queue == NULL) {
        return ESP_ERR_NO_MEM;
    }
    playback_progress_queue = xQueueCreateStatic(
        1,
        sizeof(bobe_policy_event_t),
        playback_progress_queue_storage,
        &playback_progress_queue_state
    );
    if (playback_progress_queue == NULL) {
        return ESP_ERR_NO_MEM;
    }

    ESP_RETURN_ON_ERROR(
        bobe_board_init(button_callback, NULL),
        TAG,
        "initialize board"
    );
    log_boot_stage("board");
    ESP_RETURN_ON_ERROR(bobe_display_init(), TAG, "initialize display");
    ESP_RETURN_ON_ERROR(
        bobe_board_set_backlight(75),
        TAG,
        "enable backlight"
    );
    uint8_t restored_volume = 70;
    bobe_face_theme_id_t restored_theme = BOBE_FACE_THEME_DEFAULT;
    load_user_settings(&restored_volume, &restored_theme);
    core.settings_volume = restored_volume;
    core.settings_theme = restored_theme;
    const bobe_policy_event_t boot_event = {
        .kind = BOBE_POLICY_EVENT_BOOT,
        .data.boot = {
            .microphone_muted = load_microphone_muted(),
            .ota_pending_verification = core.ota_pending_verification,
            .volume = restored_volume,
            .theme = restored_theme,
        },
    };
    dispatch_policy_event(&boot_event);
    core.settings_dirty = false;
    log_boot_stage("display");

    ESP_RETURN_ON_ERROR(
        bobe_connectivity_init(network_callback, NULL),
        TAG,
        "initialize connectivity"
    );
    log_boot_stage("connectivity");

    ESP_RETURN_ON_ERROR(bobe_audio_init(), TAG, "initialize audio");
    log_boot_stage("audio");
    ESP_RETURN_ON_ERROR(
        bobe_voice_frontend_init(
            enhanced_audio_sink,
            NULL,
            voice_event_sink,
            NULL
        ),
        TAG,
        "initialize voice frontend"
    );
    bobe_audio_set_capture_sink(capture_sink, NULL);
    bobe_audio_set_playback_progress_sink(playback_progress_sink, NULL);
    const bobe_policy_event_t voice_ready_event = {
        .kind = BOBE_POLICY_EVENT_VOICE_READY,
    };
    dispatch_policy_event(&voice_ready_event);
    log_boot_stage("voice");
    const bobe_transport_config_t transport_config = {
        .hello = {
            .device_id = core.device_id, .boot_id = core.boot_id,
            .firmware_version = esp_app_get_description()->version,
            .hardware_revision = BOBE_DEVICE_MODEL,
            .capabilities = {
                .microphone = true, .speaker = true, .battery = true,
                .wake_word = bobe_voice_frontend_wake_available(),
                .display_width = BOBE_DISPLAY_WIDTH, .display_height = BOBE_DISPLAY_HEIGHT,
                .face_schema = 1,
#ifdef CONFIG_BOBE_ENABLE_TEXT_INPUT
                .text_input = true,
#endif
#ifdef CONFIG_BOBE_ENABLE_TEXT_OUTPUT
                .text_output = true,
#endif
#ifdef CONFIG_BOBE_ENABLE_ACTION_INPUT
                .actions = true,
#endif
#ifdef CONFIG_BOBE_ENABLE_SENSOR_REPORTS
                .sensors = true,
#endif
            },
            .max_playback_credit_ms = CONFIG_BOBE_AUDIO_PLAYBACK_CREDIT_MS,
        },
        .build_number = CONFIG_BOBE_BUILD_NUMBER,
        .max_image_size = BOBE_UPDATE_IMAGE_SIZE_MAX,
        .verify_update = bobe_update_offer_verify_embedded,
    };
    ESP_RETURN_ON_ERROR(
        bobe_transport_init(
            &transport_config,
            transport_callback,
            NULL
        ),
        TAG,
        "initialize transport"
    );
    log_boot_stage("transport");

    core.battery_timer = xTimerCreate(
        "bobe_battery",
        pdMS_TO_TICKS(10000),
        pdTRUE,
        NULL,
        timer_callback
    );
    core.health_timer = xTimerCreate(
        "bobe_health",
        pdMS_TO_TICKS(10000),
        pdTRUE,
        NULL,
        timer_callback
    );
    if (core.battery_timer == NULL || core.health_timer == NULL) {
        return ESP_ERR_NO_MEM;
    }
    if (xTimerStart(core.battery_timer, 0) != pdPASS ||
        xTimerStart(core.health_timer, 0) != pdPASS) {
        return ESP_FAIL;
    }
    if (xTaskCreatePinnedToCore(
            core_task,
            "bobe_core",
            8192,
            NULL,
            BOBE_TASK_PRIORITY_CORE,
            NULL,
            BOBE_REALTIME_CORE
        ) != pdPASS) {
        return ESP_ERR_NO_MEM;
    }

    const esp_app_desc_t *app = esp_app_get_description();
    ESP_LOGI(
        TAG,
        "device=%s boot=%s firmware=%s project=%s elf=%s",
        core.device_id,
        core.boot_id,
        app->version,
        app->project_name,
        esp_app_get_elf_sha256_str()
    );
    return ESP_OK;
}

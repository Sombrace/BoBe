#include "bobe_voice_frontend.h"

#include "bobe_task_config.h"
#if defined(CONFIG_BOBE_ENABLE_ESP_SR)
#include "esp_afe_config.h"
#include "esp_afe_sr_iface.h"
#include "esp_afe_sr_models.h"
#include "esp_wn_models.h"
#include "model_path.h"
#endif
#include "esp_heap_caps.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/event_groups.h"
#include "freertos/queue.h"
#include "freertos/semphr.h"
#include "freertos/task.h"

#include <stdatomic.h>
#include <string.h>

#define BOBE_AFE_INPUT_QUEUE_FRAMES 8U
#define BOBE_PREROLL_FRAMES 25U
#define BOBE_AFE_FEED_PAUSED_BIT BIT0
#define BOBE_AFE_FETCH_PAUSED_BIT BIT1
#define BOBE_AFE_ALL_PAUSED_BITS \
    (BOBE_AFE_FEED_PAUSED_BIT | BOBE_AFE_FETCH_PAUSED_BIT)
#define BOBE_AFE_DRAIN_TIMEOUT_MS 1000U
#define BOBE_AFE_DRAIN_IDLE_MS 100U
#define BOBE_AFE_FLUSH_SILENCE_MS 500U

#if defined(CONFIG_BOBE_ENABLE_ESP_SR)
static const char *TAG = "bobe_voice";
#endif
static bobe_voice_output_sink_t output_sink;
static void *output_context;
static bobe_voice_event_sink_t event_sink;
static void *event_context;
static atomic_bool active;
static atomic_bool monitoring;
static atomic_uint active_stream_id;
static atomic_bool wake_available;
static atomic_bool initialized;
static atomic_bool closing;
static atomic_uint output_sequence;
static SemaphoreHandle_t lifecycle_lock;

#if defined(CONFIG_BOBE_ENABLE_ESP_SR)
static const esp_afe_sr_iface_t *afe_operations;
static esp_afe_sr_data_t *afe_data;
static srmodel_list_t *models;
static QueueHandle_t input_queue;
static StaticQueue_t input_queue_state;
static uint8_t *input_queue_storage;
static int16_t *feed_buffer;
static int feed_chunk_samples;
static int feed_buffered_samples;
static bobe_audio_frame_t output_frame;
static size_t output_buffered_samples;
static vad_state_t prior_vad_state;
static bobe_audio_frame_t *preroll_frames;
static size_t preroll_next;
static size_t preroll_count;
static SemaphoreHandle_t preroll_lock;
static EventGroupHandle_t afe_pause_events;
static atomic_bool afe_pause_requested;
static atomic_bool feed_frame_in_flight;
static TaskHandle_t feed_task_handle;
static TaskHandle_t fetch_task_handle;

static bool submit_afe_chunk(const int16_t *samples)
{
    const int submitted_samples = afe_operations->feed(afe_data, samples);
    if (submitted_samples <= 0) {
        ESP_LOGE(TAG, "AFE feed rejected an input chunk");
        atomic_store(&active, false);
        return false;
    }
    if (fetch_task_handle != NULL) {
        xTaskNotifyGive(fetch_task_handle);
    }
    return true;
}

static void wait_while_afe_paused(EventBits_t paused_bit)
{
    if (!atomic_load(&afe_pause_requested)) {
        return;
    }
    xEventGroupSetBits(afe_pause_events, paused_bit);
    while (atomic_load(&afe_pause_requested)) {
        vTaskDelay(pdMS_TO_TICKS(1));
    }
    xEventGroupClearBits(afe_pause_events, paused_bit);
}

static void publish_voice_event(
    bobe_voice_event_kind_t kind,
    float level_db,
    int wake_word_index
)
{
    if (event_sink == NULL) {
        return;
    }
    const bobe_voice_event_t event = {
        .kind = kind,
        .level_db = level_db,
        .wake_word_index = wake_word_index,
    };
    event_sink(&event, event_context);
}

static void feed_task(void *unused)
{
    (void)unused;
    bobe_audio_frame_t frame;
    for (;;) {
        wait_while_afe_paused(BOBE_AFE_FEED_PAUSED_BIT);
        if (xQueueReceive(
                input_queue,
                &frame,
                pdMS_TO_TICKS(20)
            ) != pdTRUE) {
            continue;
        }
        atomic_store(&feed_frame_in_flight, true);
        if (!atomic_load(&active)) {
            atomic_store(&feed_frame_in_flight, false);
            continue;
        }
        for (size_t index = 0; index < frame.sample_count; ++index) {
            feed_buffer[feed_buffered_samples++] = frame.samples[index];
            if (feed_buffered_samples == feed_chunk_samples) {
                if (!submit_afe_chunk(feed_buffer)) {
                    break;
                }
                feed_buffered_samples = 0;
            }
        }
        atomic_store(&feed_frame_in_flight, false);
    }
}

static void emit_processed_samples(const int16_t *samples, size_t count)
{
    while (count > 0 && atomic_load(&active)) {
        const size_t available =
            BOBE_MIC_FRAME_SAMPLES - output_buffered_samples;
        const size_t copy_count = count < available ? count : available;
        memcpy(
            &output_frame.samples[output_buffered_samples],
            samples,
            copy_count * sizeof(int16_t)
        );
        output_buffered_samples += copy_count;
        samples += copy_count;
        count -= copy_count;

        if (output_buffered_samples == BOBE_MIC_FRAME_SAMPLES) {
            output_frame.sample_count = BOBE_MIC_FRAME_SAMPLES;
            output_frame.monotonic_timestamp_us =
                (uint64_t)esp_timer_get_time();
            (void)xSemaphoreTake(preroll_lock, portMAX_DELAY);
            const uint32_t stream_id = atomic_load(&active_stream_id);
            bool delivered = true;
            if (stream_id == 0) {
                output_frame.stream_id = 0;
                output_frame.sequence = 0;
                preroll_frames[preroll_next] = output_frame;
                preroll_next = (preroll_next + 1U) % BOBE_PREROLL_FRAMES;
                if (preroll_count < BOBE_PREROLL_FRAMES) {
                    ++preroll_count;
                }
                xSemaphoreGive(preroll_lock);
            } else {
                xSemaphoreGive(preroll_lock);
                output_frame.stream_id = stream_id;
                output_frame.sequence =
                    atomic_fetch_add(&output_sequence, 1);
                delivered =
                    output_sink != NULL &&
                    output_sink(&output_frame, output_context);
            }
            if (!delivered) {
                atomic_store(&active, false);
            }
            output_buffered_samples = 0;
        }
    }
}

static void fetch_task(void *unused)
{
    (void)unused;
    for (;;) {
        wait_while_afe_paused(BOBE_AFE_FETCH_PAUSED_BIT);
        if (!atomic_load(&active)) {
            (void)ulTaskNotifyTake(pdTRUE, 0);
            vTaskDelay(pdMS_TO_TICKS(20));
            continue;
        }
        if (ulTaskNotifyTake(
                pdFALSE,
                pdMS_TO_TICKS(100)
            ) == 0) {
            continue;
        }
        if (!atomic_load(&active)) {
            continue;
        }
        const afe_fetch_result_t *result = afe_operations->fetch_with_delay(
            afe_data,
            pdMS_TO_TICKS(100)
        );
        if (result == NULL || result->data == NULL ||
            result->data_size <= 0 || !atomic_load(&active)) {
            continue;
        }

        if (result->vad_state != prior_vad_state) {
            if (result->vad_state == VAD_SPEECH) {
                publish_voice_event(
                    BOBE_VOICE_EVENT_SPEECH_STARTED,
                    result->data_volume,
                    0
                );
            } else {
                publish_voice_event(
                    BOBE_VOICE_EVENT_SPEECH_ENDED,
                    result->data_volume,
                    0
                );
            }
            prior_vad_state = result->vad_state;
        }
        if (result->wakeup_state == WAKENET_DETECTED) {
            publish_voice_event(
                BOBE_VOICE_EVENT_WAKE_DETECTED,
                result->data_volume,
                result->wake_word_index
            );
        }
        emit_processed_samples(
            result->data,
            (size_t)result->data_size / sizeof(int16_t)
        );
    }
}

static esp_err_t initialize_afe(void)
{
    models = esp_srmodel_init("model");
    char *wake_model = NULL;
    if (models != NULL) {
        wake_model = esp_srmodel_filter(models, ESP_WN_PREFIX, "bobe");
    }

    afe_config_t *config = afe_config_init(
        "M",
        models,
        AFE_TYPE_SR,
        AFE_MODE_HIGH_PERF
    );
    if (config == NULL) {
        return ESP_ERR_NO_MEM;
    }
    config->aec_init = false;
    config->se_init = false;
    config->ns_init = true;
    config->ns_model_name = NULL;
    config->afe_ns_mode = AFE_NS_MODE_WEBRTC;
    config->vad_init = true;
    config->vad_model_name = NULL;
    config->vad_min_speech_ms = 128;
    config->vad_min_noise_ms = 700;
    config->vad_delay_ms = 256;
    config->vad_mute_playback = false;
    config->wakenet_init = wake_model != NULL;
    config->wakenet_model_name = wake_model;
    config->wakenet_mode = DET_MODE_90;
    config->agc_init = true;
    config->agc_compression_gain_db = 9;
    config->agc_target_level_dbfs = 6;
    config->fixed_output_channel = true;
    config->afe_perferred_core = BOBE_REALTIME_CORE;
    config->afe_perferred_priority = BOBE_TASK_PRIORITY_AFE;
    config->memory_alloc_mode = AFE_MEMORY_ALLOC_MORE_PSRAM;

    afe_operations = esp_afe_handle_from_config(config);
    afe_data = NULL;
    if (afe_operations != NULL) {
        afe_data = afe_operations->create_from_config(config);
    }
    afe_config_free(config);
    if (afe_data == NULL) {
        return ESP_FAIL;
    }
    feed_chunk_samples = afe_operations->get_feed_chunksize(afe_data);
    if (feed_chunk_samples <= 0 ||
        afe_operations->get_feed_channel_num(afe_data) != 1 ||
        afe_operations->get_samp_rate(afe_data) != 16000) {
        return ESP_ERR_INVALID_STATE;
    }
    feed_buffer = heap_caps_malloc(
        (size_t)feed_chunk_samples * sizeof(int16_t),
        MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT
    );
    input_queue_storage = heap_caps_malloc(
        BOBE_AFE_INPUT_QUEUE_FRAMES * sizeof(bobe_audio_frame_t),
        MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT
    );
    preroll_frames = heap_caps_malloc(
        BOBE_PREROLL_FRAMES * sizeof(bobe_audio_frame_t),
        MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT
    );
    preroll_lock = xSemaphoreCreateMutex();
    afe_pause_events = xEventGroupCreate();
    if (feed_buffer == NULL || input_queue_storage == NULL ||
        preroll_frames == NULL || preroll_lock == NULL ||
        afe_pause_events == NULL) {
        return ESP_ERR_NO_MEM;
    }
    input_queue = xQueueCreateStatic(
        BOBE_AFE_INPUT_QUEUE_FRAMES,
        sizeof(bobe_audio_frame_t),
        input_queue_storage,
        &input_queue_state
    );
    if (input_queue == NULL) {
        return ESP_ERR_NO_MEM;
    }
    if (xTaskCreatePinnedToCoreWithCaps(
            feed_task,
            "bobe_afe_feed",
            4096,
            NULL,
            BOBE_TASK_PRIORITY_AFE,
            &feed_task_handle,
            BOBE_REALTIME_CORE,
            MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT
        ) != pdPASS) {
        feed_task_handle = NULL;
        return ESP_ERR_NO_MEM;
    }
    if (xTaskCreatePinnedToCoreWithCaps(
            fetch_task,
            "bobe_afe_fetch",
            4096,
            NULL,
            BOBE_TASK_PRIORITY_AFE,
            &fetch_task_handle,
            BOBE_REALTIME_CORE,
            MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT
        ) != pdPASS) {
        fetch_task_handle = NULL;
        return ESP_ERR_NO_MEM;
    }
    atomic_store(&wake_available, wake_model != NULL);
    afe_operations->print_pipeline(afe_data);
    return ESP_OK;
}

static void release_afe(void)
{
    if (fetch_task_handle != NULL) {
        vTaskDeleteWithCaps(fetch_task_handle);
        fetch_task_handle = NULL;
    }
    if (feed_task_handle != NULL) {
        vTaskDeleteWithCaps(feed_task_handle);
        feed_task_handle = NULL;
    }
    if (input_queue != NULL) {
        vQueueDelete(input_queue);
        input_queue = NULL;
    }
    heap_caps_free(input_queue_storage);
    input_queue_storage = NULL;
    heap_caps_free(preroll_frames);
    preroll_frames = NULL;
    heap_caps_free(feed_buffer);
    feed_buffer = NULL;
    if (afe_pause_events != NULL) {
        vEventGroupDelete(afe_pause_events);
        afe_pause_events = NULL;
    }
    if (preroll_lock != NULL) {
        vSemaphoreDelete(preroll_lock);
        preroll_lock = NULL;
    }
    if (afe_data != NULL) {
        afe_operations->destroy(afe_data);
        afe_data = NULL;
    }
    afe_operations = NULL;
    if (models != NULL) {
        esp_srmodel_deinit(models);
        models = NULL;
    }
    atomic_store(&wake_available, false);
}
#endif

static esp_err_t reset_afe_pipeline(void)
{
#if !defined(CONFIG_BOBE_ENABLE_ESP_SR)
    return ESP_OK;
#else
    atomic_store(&active, false);
    atomic_store(&afe_pause_requested, true);
    const EventBits_t paused = xEventGroupWaitBits(
        afe_pause_events,
        BOBE_AFE_ALL_PAUSED_BITS,
        pdFALSE,
        pdTRUE,
        pdMS_TO_TICKS(250)
    );
    if ((paused & BOBE_AFE_ALL_PAUSED_BITS) != BOBE_AFE_ALL_PAUSED_BITS) {
        atomic_store(&afe_pause_requested, false);
        return ESP_ERR_TIMEOUT;
    }

    xQueueReset(input_queue);
    feed_buffered_samples = 0;
    output_buffered_samples = 0;
    atomic_store(&feed_frame_in_flight, false);
    prior_vad_state = VAD_SILENCE;
    const int reset_result = afe_operations->reset_buffer(afe_data);
    (void)xSemaphoreTake(preroll_lock, portMAX_DELAY);
    preroll_next = 0;
    preroll_count = 0;
    xSemaphoreGive(preroll_lock);
    atomic_store(&afe_pause_requested, false);
    const TickType_t release_deadline =
        xTaskGetTickCount() + pdMS_TO_TICKS(250);
    while ((xEventGroupGetBits(afe_pause_events) &
            BOBE_AFE_ALL_PAUSED_BITS) != 0) {
        if ((int32_t)(xTaskGetTickCount() - release_deadline) >= 0) {
            return ESP_ERR_TIMEOUT;
        }
        vTaskDelay(pdMS_TO_TICKS(1));
    }
    return reset_result == 1 ? ESP_OK : ESP_FAIL;
#endif
}

esp_err_t bobe_voice_frontend_init(
    bobe_voice_output_sink_t new_output_sink,
    void *new_output_context,
    bobe_voice_event_sink_t new_event_sink,
    void *new_event_context
)
{
    if (new_output_sink == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    lifecycle_lock = xSemaphoreCreateMutex();
    if (lifecycle_lock == NULL) {
        return ESP_ERR_NO_MEM;
    }
    output_sink = new_output_sink;
    output_context = new_output_context;
    event_sink = new_event_sink;
    event_context = new_event_context;

#if defined(CONFIG_BOBE_ENABLE_ESP_SR)
    const esp_err_t result = initialize_afe();
    if (result != ESP_OK) {
        ESP_LOGE(
            TAG,
            "voice front end initialization failed: %s",
            esp_err_to_name(result)
        );
        release_afe();
        vSemaphoreDelete(lifecycle_lock);
        lifecycle_lock = NULL;
        return result;
    }
#endif
    atomic_store(&initialized, true);
    return ESP_OK;
}

#if defined(CONFIG_BOBE_ENABLE_ESP_SR)
static esp_err_t drain_afe_stream(void)
{
    const uint64_t deadline_us =
        (uint64_t)esp_timer_get_time() +
        (uint64_t)BOBE_AFE_DRAIN_TIMEOUT_MS * UINT64_C(1000);
    while (uxQueueMessagesWaiting(input_queue) > 0 ||
           atomic_load(&feed_frame_in_flight)) {
        if (!atomic_load(&active)) {
            return ESP_ERR_NO_MEM;
        }
        if ((uint64_t)esp_timer_get_time() >= deadline_us) {
            return ESP_ERR_TIMEOUT;
        }
        vTaskDelay(pdMS_TO_TICKS(1));
    }

    atomic_store(&afe_pause_requested, true);
    while ((xEventGroupGetBits(afe_pause_events) &
            BOBE_AFE_ALL_PAUSED_BITS) != BOBE_AFE_ALL_PAUSED_BITS) {
        if ((uint64_t)esp_timer_get_time() >= deadline_us) {
            return ESP_ERR_TIMEOUT;
        }
        vTaskDelay(pdMS_TO_TICKS(1));
    }
    if (uxQueueMessagesWaiting(input_queue) > 0 ||
        atomic_load(&feed_frame_in_flight)) {
        return ESP_ERR_TIMEOUT;
    }

    if (feed_buffered_samples > 0) {
        memset(
            &feed_buffer[feed_buffered_samples],
            0,
            ((size_t)feed_chunk_samples -
             (size_t)feed_buffered_samples) * sizeof(int16_t)
        );
        if (afe_operations->feed(afe_data, feed_buffer) <= 0) {
            return ESP_FAIL;
        }
        feed_buffered_samples = 0;
    }
    memset(feed_buffer, 0, (size_t)feed_chunk_samples * sizeof(int16_t));
    const size_t flush_samples =
        BOBE_MIC_SAMPLE_RATE_HZ * BOBE_AFE_FLUSH_SILENCE_MS / 1000U;
    const size_t flush_chunks =
        (flush_samples + (size_t)feed_chunk_samples - 1U) /
        (size_t)feed_chunk_samples;
    for (size_t index = 0; index < flush_chunks; ++index) {
        if (afe_operations->feed(afe_data, feed_buffer) <= 0) {
            return ESP_FAIL;
        }
    }

    unsigned int drained_chunks = 0;
    for (;;) {
        if (!atomic_load(&active)) {
            return ESP_ERR_NO_MEM;
        }
        if ((uint64_t)esp_timer_get_time() >= deadline_us) {
            return ESP_ERR_TIMEOUT;
        }
        const afe_fetch_result_t *result = afe_operations->fetch_with_delay(
            afe_data,
            pdMS_TO_TICKS(BOBE_AFE_DRAIN_IDLE_MS)
        );
        if (result == NULL || result->data == NULL ||
            result->data_size <= 0) {
            break;
        }
        emit_processed_samples(
            result->data,
            (size_t)result->data_size / sizeof(int16_t)
        );
        ++drained_chunks;
    }
    ESP_LOGI(TAG, "AFE drain complete: chunks=%u", drained_chunks);
    if (output_buffered_samples > 0) {
        const int16_t silence[BOBE_MIC_FRAME_SAMPLES] = {0};
        emit_processed_samples(
            silence,
            BOBE_MIC_FRAME_SAMPLES - output_buffered_samples
        );
        if (!atomic_load(&active)) {
            return ESP_ERR_NO_MEM;
        }
    }
    return ESP_OK;
}
#endif

esp_err_t bobe_voice_frontend_begin_stream(uint32_t stream_id)
{
    if (!atomic_load(&initialized) || stream_id == 0 ||
        lifecycle_lock == NULL ||
        xSemaphoreTake(
            lifecycle_lock,
            pdMS_TO_TICKS(250)
        ) != pdTRUE) {
        return ESP_ERR_INVALID_STATE;
    }
    if (atomic_load(&active_stream_id) != 0 ||
        atomic_load(&closing)) {
        xSemaphoreGive(lifecycle_lock);
        return ESP_ERR_INVALID_STATE;
    }
    const bool was_monitoring = atomic_load(&monitoring);
    if (!was_monitoring) {
        const esp_err_t reset_result = reset_afe_pipeline();
        if (reset_result != ESP_OK) {
            xSemaphoreGive(lifecycle_lock);
            return reset_result;
        }
    }

    atomic_store(&output_sequence, 0);
#if defined(CONFIG_BOBE_ENABLE_ESP_SR)
    (void)xSemaphoreTake(preroll_lock, portMAX_DELAY);
    if (was_monitoring) {
        const size_t oldest =
            (preroll_next + BOBE_PREROLL_FRAMES - preroll_count) %
            BOBE_PREROLL_FRAMES;
        for (size_t index = 0; index < preroll_count; ++index) {
            bobe_audio_frame_t frame =
                preroll_frames[(oldest + index) % BOBE_PREROLL_FRAMES];
            frame.stream_id = stream_id;
            frame.sequence = atomic_fetch_add(&output_sequence, 1);
            if (!output_sink(&frame, output_context)) {
                xSemaphoreGive(preroll_lock);
                atomic_store(&active_stream_id, 0);
                atomic_store(&active, was_monitoring);
                xSemaphoreGive(lifecycle_lock);
                return ESP_ERR_NO_MEM;
            }
        }
    }
    preroll_next = 0;
    preroll_count = 0;
    atomic_store(&active_stream_id, stream_id);
    atomic_store(&active, true);
    xSemaphoreGive(preroll_lock);
#else
    atomic_store(&active_stream_id, stream_id);
    atomic_store(&active, true);
#endif
    xSemaphoreGive(lifecycle_lock);
    return ESP_OK;
}

esp_err_t bobe_voice_frontend_end_stream(void)
{
    if (lifecycle_lock == NULL ||
        xSemaphoreTake(
            lifecycle_lock,
            pdMS_TO_TICKS(250)
        ) != pdTRUE) {
        return ESP_ERR_TIMEOUT;
    }
    if (atomic_load(&active_stream_id) == 0) {
        xSemaphoreGive(lifecycle_lock);
        return ESP_OK;
    }
    atomic_store(&closing, true);
    esp_err_t drain_result = ESP_OK;
#if defined(CONFIG_BOBE_ENABLE_ESP_SR)
    if (input_queue != NULL) {
        drain_result = drain_afe_stream();
        if (drain_result != ESP_OK) {
            ESP_LOGE(TAG, "could not drain AFE after capture");
        }
    }
    const esp_err_t reset_result = reset_afe_pipeline();
#else
    const esp_err_t reset_result = ESP_OK;
#endif
    atomic_store(&active_stream_id, 0);
    atomic_store(&active, atomic_load(&monitoring));
    atomic_store(&closing, false);
    xSemaphoreGive(lifecycle_lock);
    if (drain_result != ESP_OK) {
        return drain_result;
    }
    return reset_result;
}

esp_err_t bobe_voice_frontend_start_monitor(void)
{
    if (!atomic_load(&initialized) ||
        !atomic_load(&wake_available) ||
        lifecycle_lock == NULL) {
        return ESP_ERR_NOT_SUPPORTED;
    }
    if (xSemaphoreTake(
            lifecycle_lock,
            pdMS_TO_TICKS(250)
        ) != pdTRUE) {
        return ESP_ERR_TIMEOUT;
    }
    if (atomic_load(&active_stream_id) != 0 ||
        atomic_load(&closing)) {
        xSemaphoreGive(lifecycle_lock);
        return ESP_ERR_INVALID_STATE;
    }
    if (atomic_load(&monitoring)) {
        xSemaphoreGive(lifecycle_lock);
        return ESP_OK;
    }
    const esp_err_t reset_result = reset_afe_pipeline();
    if (reset_result != ESP_OK) {
        xSemaphoreGive(lifecycle_lock);
        return reset_result;
    }
    atomic_store(&monitoring, true);
    atomic_store(&active, true);
    atomic_store(&active_stream_id, 0);
    xSemaphoreGive(lifecycle_lock);
    return ESP_OK;
}

void bobe_voice_frontend_stop_monitor(void)
{
    if (lifecycle_lock == NULL ||
        xSemaphoreTake(lifecycle_lock, portMAX_DELAY) != pdTRUE) {
        return;
    }
    atomic_store(&monitoring, false);
    if (atomic_load(&active_stream_id) == 0) {
        atomic_store(&active, false);
    }
    xSemaphoreGive(lifecycle_lock);
}

bool bobe_voice_frontend_submit(const bobe_audio_frame_t *frame)
{
    if (frame == NULL ||
        frame->sample_count != BOBE_MIC_FRAME_SAMPLES) {
        return false;
    }
    if (atomic_load(&closing)) {
        return true;
    }
    if (!atomic_load(&active)) {
#if defined(CONFIG_BOBE_ENABLE_ESP_SR)
        if (atomic_load(&afe_pause_requested)) {
            return true;
        }
#endif
        return false;
    }
    const uint32_t stream_id = atomic_load(&active_stream_id);
    if (stream_id != 0 && frame->stream_id == 0) {
        return true;
    }
    if (stream_id != 0 && frame->stream_id != stream_id) {
        return false;
    }
#if defined(CONFIG_BOBE_ENABLE_ESP_SR)
    return xQueueSend(input_queue, frame, 0) == pdTRUE;
#else
    return output_sink(frame, output_context);
#endif
}

bool bobe_voice_frontend_wake_available(void)
{
    return atomic_load(&wake_available);
}

#include "bobe_audio.h"
#include "bobe_board_audio.h"

#include "bobe_task_config.h"
#include "driver/i2s_std.h"
#include "esp_attr.h"
#include "esp_check.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/event_groups.h"
#include "freertos/semphr.h"
#include "freertos/task.h"

#include <stdatomic.h>
#include <string.h>

#define BOBE_PLAYBACK_CREDIT_FRAMES \
    (CONFIG_BOBE_AUDIO_PLAYBACK_CREDIT_MS / BOBE_AUDIO_FRAME_DURATION_MS)
#define BOBE_SPEAKER_DMA_DESCRIPTORS 6U
#define BOBE_PLAYBACK_REBUFFER_FRAMES BOBE_SPEAKER_DMA_DESCRIPTORS

_Static_assert(
    BOBE_PLAYBACK_CREDIT_FRAMES >= BOBE_SPEAKER_DMA_DESCRIPTORS,
    "Playback credit must cover the speaker DMA ring"
);
static const char *TAG = "bobe_audio";

typedef enum {
    PLAYBACK_TERMINAL_NONE = 0,
    PLAYBACK_TERMINAL_DRAINED,
    PLAYBACK_TERMINAL_ABORTED,
    PLAYBACK_TERMINAL_FAILED,
} playback_terminal_result_t;

static i2s_chan_handle_t microphone_channel;
static i2s_chan_handle_t speaker_channel;
static SemaphoreHandle_t state_lock;
static EventGroupHandle_t playback_events;
static bobe_audio_capture_sink_t capture_sink;
static void *capture_sink_context;
static bobe_audio_playback_progress_sink_t playback_progress_sink;
static void *playback_progress_context;
static TaskHandle_t capture_task_handle;
static TaskHandle_t playback_task_handle;
static atomic_uint capture_stream_id;
static atomic_uint playback_stream_id;
static atomic_uint capture_sequence;
static atomic_bool capturing;
static atomic_bool monitoring;
static atomic_bool microphone_muted;
static atomic_bool playing;
static atomic_bool speaker_enabled;
static atomic_bool playback_closing;
static atomic_uint capture_overruns;
static atomic_uint playback_underruns;
static atomic_uint volume_percent = 70;
static atomic_uint playback_frames_accepted;
static atomic_uint playback_frames_completed;
static atomic_uint playback_stream_generation;
static portMUX_TYPE playback_counter_lock = portMUX_INITIALIZER_UNLOCKED;
static DRAM_ATTR int16_t
    playback_ring[BOBE_PLAYBACK_CREDIT_FRAMES][BOBE_SPEAKER_FRAME_SAMPLES];
static DRAM_ATTR void
    *speaker_dma_buffers[BOBE_SPEAKER_DMA_DESCRIPTORS];
static DRAM_ATTR bool
    speaker_dma_has_audio[BOBE_SPEAKER_DMA_DESCRIPTORS];
static size_t playback_ring_read;
static size_t playback_ring_write;
static size_t playback_ring_count;
static size_t speaker_dma_audio_count;
static size_t preloaded_audio_remaining;
static bool playback_starved;
static playback_terminal_result_t playback_terminal_result;
static uint32_t playback_terminal_generation;
static uint32_t playback_terminal_stream_id;
static DRAM_ATTR int16_t speaker_silence[BOBE_SPEAKER_FRAME_SAMPLES];

static bool abort_playback_locked(uint32_t required_stream_id);

#define BOBE_PLAYBACK_TERMINAL_BIT BIT0

static void reset_playback_queue_locked(void)
{
    playback_ring_read = 0;
    playback_ring_write = 0;
    playback_ring_count = 0;
    speaker_dma_audio_count = 0;
    preloaded_audio_remaining = 0;
    playback_starved = false;
    for (size_t index = 0;
         index < BOBE_SPEAKER_DMA_DESCRIPTORS;
         ++index) {
        speaker_dma_has_audio[index] = false;
    }
}

static void clear_playback_buffers_locked(void)
{
    reset_playback_queue_locked();
    for (size_t index = 0;
         index < BOBE_SPEAKER_DMA_DESCRIPTORS;
         ++index) {
        if (speaker_dma_buffers[index] != NULL) {
            memset(
                speaker_dma_buffers[index],
                0,
                BOBE_SPEAKER_FRAME_SAMPLES * sizeof(int16_t)
            );
        }
    }
}

static void publish_playback_terminal_locked(
    uint32_t generation,
    uint32_t stream_id,
    playback_terminal_result_t result
)
{
    playback_terminal_generation = generation;
    playback_terminal_stream_id = stream_id;
    playback_terminal_result = result;
    xEventGroupSetBits(playback_events, BOBE_PLAYBACK_TERMINAL_BIT);
}

static esp_err_t finish_playback_locked(
    uint32_t generation,
    uint32_t stream_id
)
{
    portENTER_CRITICAL(&playback_counter_lock);
    if (!atomic_load(&playing) ||
        generation != atomic_load(&playback_stream_generation) ||
        stream_id != atomic_load(&playback_stream_id)) {
        portEXIT_CRITICAL(&playback_counter_lock);
        return ESP_ERR_INVALID_STATE;
    }
    atomic_store(&playing, false);
    const bool was_enabled = atomic_exchange(&speaker_enabled, false);
    portEXIT_CRITICAL(&playback_counter_lock);

    const esp_err_t disable_result =
        was_enabled ? i2s_channel_disable(speaker_channel) : ESP_OK;

    clear_playback_buffers_locked();
    atomic_store(&playback_stream_id, 0);
    publish_playback_terminal_locked(
        generation,
        stream_id,
        disable_result == ESP_OK ? PLAYBACK_TERMINAL_DRAINED
                                 : PLAYBACK_TERMINAL_FAILED
    );
    return disable_result;
}

static size_t IRAM_ATTR find_speaker_dma_index(const void *dma_buffer)
{
    size_t available_index = BOBE_SPEAKER_DMA_DESCRIPTORS;
    for (size_t index = 0;
         index < BOBE_SPEAKER_DMA_DESCRIPTORS;
         ++index) {
        if (speaker_dma_buffers[index] == dma_buffer) {
            return index;
        }
        if (available_index == BOBE_SPEAKER_DMA_DESCRIPTORS &&
            speaker_dma_buffers[index] == NULL) {
            available_index = index;
        }
    }
    return available_index;
}

static bool IRAM_ATTR speaker_sent_callback(
    i2s_chan_handle_t channel,
    i2s_event_data_t *event,
    void *context
)
{
    (void)channel;
    (void)context;
    if (event == NULL || event->dma_buf == NULL) {
        return false;
    }

    bool completed_frame = false;
    bool underrun_started = false;
    portENTER_CRITICAL_ISR(&playback_counter_lock);
    const size_t dma_index = find_speaker_dma_index(event->dma_buf);
    if (dma_index < BOBE_SPEAKER_DMA_DESCRIPTORS &&
        speaker_dma_buffers[dma_index] == NULL) {
        speaker_dma_buffers[dma_index] = event->dma_buf;
    }
    bool dma_had_audio = false;
    if (preloaded_audio_remaining > 0) {
        --preloaded_audio_remaining;
        dma_had_audio = true;
    } else if (dma_index < BOBE_SPEAKER_DMA_DESCRIPTORS &&
               speaker_dma_has_audio[dma_index]) {
        speaker_dma_has_audio[dma_index] = false;
        dma_had_audio = true;
    }
    if (dma_had_audio) {
        if (speaker_dma_audio_count > 0) {
            --speaker_dma_audio_count;
        }
        if (atomic_load_explicit(&playing, memory_order_relaxed)) {
            const uint32_t completed = atomic_load_explicit(
                &playback_frames_completed,
                memory_order_relaxed
            );
            atomic_store_explicit(
                &playback_frames_completed,
                completed + 1U,
                memory_order_relaxed
            );
            completed_frame = true;
        }
    }

    if (playback_starved &&
        (playback_ring_count >= BOBE_PLAYBACK_REBUFFER_FRAMES ||
         atomic_load_explicit(
             &playback_closing,
             memory_order_relaxed
         ))) {
        playback_starved = false;
    }

    int16_t *output = event->dma_buf;
    if (dma_index < BOBE_SPEAKER_DMA_DESCRIPTORS &&
        event->size ==
            BOBE_SPEAKER_FRAME_SAMPLES * sizeof(*output) &&
        atomic_load_explicit(&playing, memory_order_relaxed) &&
        !playback_starved &&
        playback_ring_count > 0) {
        memcpy(
            output,
            playback_ring[playback_ring_read],
            BOBE_SPEAKER_FRAME_SAMPLES * sizeof(*output)
        );
        playback_ring_read =
            (playback_ring_read + 1U) % BOBE_PLAYBACK_CREDIT_FRAMES;
        --playback_ring_count;
        ++speaker_dma_audio_count;
        speaker_dma_has_audio[dma_index] = true;
    } else {
        if (atomic_load_explicit(&playing, memory_order_relaxed) &&
            !atomic_load_explicit(
                &playback_closing,
                memory_order_relaxed
            ) &&
            !playback_starved) {
            playback_starved = true;
            underrun_started = true;
            atomic_fetch_add_explicit(
                &playback_underruns,
                1U,
                memory_order_relaxed
            );
        }
        memset(output, 0, event->size);
    }
    portEXIT_CRITICAL_ISR(&playback_counter_lock);

    BaseType_t task_woken = pdFALSE;
    if ((completed_frame || underrun_started ||
         atomic_load(&playback_closing)) &&
        playback_task_handle != NULL) {
        vTaskNotifyGiveFromISR(playback_task_handle, &task_woken);
    }

    return task_woken == pdTRUE;
}

static esp_err_t enable_speaker_locked(void)
{
    if (atomic_load(&speaker_enabled)) {
        return ESP_OK;
    }
    size_t audio_preloaded = 0;
    for (size_t index = 0;
         index < BOBE_SPEAKER_DMA_DESCRIPTORS;
         ++index) {
        const bool has_audio = playback_ring_count > 0;
        const int16_t *samples =
            has_audio ? playback_ring[playback_ring_read] : speaker_silence;
        size_t loaded = 0;
        const esp_err_t preload_result = i2s_channel_preload_data(
            speaker_channel,
            samples,
            BOBE_SPEAKER_FRAME_SAMPLES * sizeof(*samples),
            &loaded
        );
        if (preload_result != ESP_OK) {
            return preload_result;
        }
        if (loaded == 0) {
            break;
        }
        if (loaded != BOBE_SPEAKER_FRAME_SAMPLES * sizeof(*samples)) {
            return ESP_ERR_INVALID_SIZE;
        }
        if (has_audio) {
            playback_ring_read =
                (playback_ring_read + 1U) % BOBE_PLAYBACK_CREDIT_FRAMES;
            --playback_ring_count;
            ++audio_preloaded;
        }
    }
    if (atomic_load(&playback_frames_accepted) > 0 &&
        audio_preloaded == 0) {
        return ESP_FAIL;
    }
    preloaded_audio_remaining = audio_preloaded;
    speaker_dma_audio_count += audio_preloaded;

    atomic_store(&speaker_enabled, true);
    const esp_err_t result = i2s_channel_enable(speaker_channel);
    if (result != ESP_OK) {
        atomic_store(&speaker_enabled, false);
    }
    return result;
}

static esp_err_t init_microphone(void)
{
    const bobe_board_audio_config_t board = bobe_board_audio_config();
    i2s_chan_config_t channel_config =
        I2S_CHANNEL_DEFAULT_CONFIG(board.microphone_port, I2S_ROLE_MASTER);
    channel_config.dma_desc_num = 6;
    channel_config.dma_frame_num = BOBE_MIC_FRAME_SAMPLES;
    ESP_RETURN_ON_ERROR(
        i2s_new_channel(
            &channel_config,
            NULL,
            &microphone_channel
        ),
        TAG,
        "create microphone channel"
    );

    const i2s_std_config_t standard_config = board.microphone;
    return i2s_channel_init_std_mode(microphone_channel, &standard_config);
}

static esp_err_t init_speaker(void)
{
    const bobe_board_audio_config_t board = bobe_board_audio_config();
    i2s_chan_config_t channel_config =
        I2S_CHANNEL_DEFAULT_CONFIG(board.speaker_port, I2S_ROLE_MASTER);
    channel_config.dma_desc_num = BOBE_SPEAKER_DMA_DESCRIPTORS;
    channel_config.dma_frame_num = BOBE_SPEAKER_FRAME_SAMPLES;
    channel_config.auto_clear_before_cb = true;
    ESP_RETURN_ON_ERROR(
        i2s_new_channel(
            &channel_config,
            &speaker_channel,
            NULL
        ),
        TAG,
        "create speaker channel"
    );

    const i2s_std_config_t standard_config = board.speaker;
    ESP_RETURN_ON_ERROR(
        i2s_channel_init_std_mode(speaker_channel, &standard_config),
        TAG,
        "configure speaker channel"
    );
    const i2s_event_callbacks_t callbacks = {
        .on_sent = speaker_sent_callback,
    };
    return i2s_channel_register_event_callback(
        speaker_channel,
        &callbacks,
        NULL
    );
}

static void capture_task(void *unused)
{
    (void)unused;
    int32_t i2s_samples[BOBE_MIC_FRAME_SAMPLES];
    bobe_audio_frame_t frame = {
        .sample_count = BOBE_MIC_FRAME_SAMPLES,
    };

    for (;;) {
        ulTaskNotifyTake(pdTRUE, portMAX_DELAY);
        while ((atomic_load(&capturing) || atomic_load(&monitoring)) &&
               !atomic_load(&microphone_muted)) {
            size_t bytes_read = 0;
            const esp_err_t result = i2s_channel_read(
                microphone_channel,
                i2s_samples,
                sizeof(i2s_samples),
                &bytes_read,
                pdMS_TO_TICKS(100)
            );
            if (result != ESP_OK ||
                bytes_read != sizeof(i2s_samples)) {
                if (atomic_load(&capturing) ||
                    atomic_load(&monitoring)) {
                    atomic_fetch_add(&capture_overruns, 1);
                }
                continue;
            }
            if (atomic_load(&microphone_muted)) {
                continue;
            }

            frame.monotonic_timestamp_us = esp_timer_get_time();
            for (size_t index = 0; index < BOBE_MIC_FRAME_SAMPLES; ++index) {
                frame.samples[index] = (int16_t)(i2s_samples[index] >> 16);
            }

            if (xSemaphoreTake(
                    state_lock,
                    pdMS_TO_TICKS(20)
                ) != pdTRUE) {
                atomic_fetch_add(&capture_overruns, 1);
                continue;
            }
            const bool is_muted = atomic_load(&microphone_muted);
            const bool is_capturing = atomic_load(&capturing);
            if (is_capturing && !is_muted) {
                frame.stream_id = atomic_load(&capture_stream_id);
                frame.sequence = atomic_fetch_add(&capture_sequence, 1);
            } else {
                frame.stream_id = 0;
                frame.sequence = 0;
            }
            const bobe_audio_capture_sink_t sink =
                is_muted ? NULL : capture_sink;
            void *sink_context = capture_sink_context;
            xSemaphoreGive(state_lock);
            if (sink == NULL || !sink(&frame, sink_context)) {
                atomic_fetch_add(&capture_overruns, 1);
            }
        }
    }
}

static void playback_task(void *unused)
{
    (void)unused;
    uint32_t last_reported_frames = 0;
    uint32_t last_stream_generation = 0;
    uint32_t last_reported_underruns = 0;

    for (;;) {
        (void)ulTaskNotifyTake(pdTRUE, pdMS_TO_TICKS(20));
        uint32_t played = 0;
        uint32_t accepted = 0;
        uint32_t active_stream_id = 0;
        bobe_audio_playback_progress_sink_t progress_sink = NULL;
        void *progress_context = NULL;
        uint32_t generation = 0;
        if (xSemaphoreTake(state_lock, pdMS_TO_TICKS(20)) != pdTRUE) {
            continue;
        }
        generation = atomic_load(&playback_stream_generation);
        if (generation != last_stream_generation) {
            last_stream_generation = generation;
            last_reported_frames = 0;
            last_reported_underruns =
                atomic_load(&playback_underruns);
        }
        played = atomic_load(&playback_frames_completed);
        accepted = atomic_load(&playback_frames_accepted);
        active_stream_id = atomic_load(&playback_stream_id);
        progress_sink = playback_progress_sink;
        progress_context = playback_progress_context;
        bool should_finish = false;
        portENTER_CRITICAL(&playback_counter_lock);
        should_finish =
            atomic_load(&playing) &&
            atomic_load(&playback_closing) &&
            playback_ring_count == 0 &&
            speaker_dma_audio_count == 0 &&
            played >= accepted;
        portEXIT_CRITICAL(&playback_counter_lock);
        if (should_finish) {
            (void)finish_playback_locked(generation, active_stream_id);
            xSemaphoreGive(state_lock);
            continue;
        }
        const bool stream_current =
            generation == atomic_load(&playback_stream_generation) &&
            active_stream_id == atomic_load(&playback_stream_id);
        xSemaphoreGive(state_lock);
        if (stream_current && atomic_load(&playing) &&
            played != last_reported_frames &&
            progress_sink != NULL) {
            last_reported_frames = played;
            const uint32_t outstanding =
                accepted > played ? accepted - played : 0;
            const uint32_t used =
                outstanding < BOBE_PLAYBACK_CREDIT_FRAMES
                    ? outstanding
                    : BOBE_PLAYBACK_CREDIT_FRAMES;
            progress_sink(
                active_stream_id,
                (uint64_t)played * BOBE_SPEAKER_FRAME_SAMPLES,
                (BOBE_PLAYBACK_CREDIT_FRAMES - used) *
                    BOBE_AUDIO_FRAME_DURATION_MS,
                progress_context
            );
        }
        const uint32_t underruns = atomic_load(&playback_underruns);
        if (underruns != last_reported_underruns) {
            last_reported_underruns = underruns;
            ESP_LOGW(
                TAG,
                "speaker underrun stream=%u accepted=%u completed=%u",
                active_stream_id,
                accepted,
                played
            );
        }
    }
}

static void release_audio(void)
{
    if (playback_task_handle != NULL) {
        vTaskDelete(playback_task_handle);
        playback_task_handle = NULL;
    }
    if (capture_task_handle != NULL) {
        vTaskDelete(capture_task_handle);
        capture_task_handle = NULL;
    }
    if (speaker_channel != NULL) {
        (void)i2s_del_channel(speaker_channel);
        speaker_channel = NULL;
    }
    if (microphone_channel != NULL) {
        (void)i2s_del_channel(microphone_channel);
        microphone_channel = NULL;
    }
    if (playback_events != NULL) {
        vEventGroupDelete(playback_events);
        playback_events = NULL;
    }
    if (state_lock != NULL) {
        vSemaphoreDelete(state_lock);
        state_lock = NULL;
    }
}

esp_err_t bobe_audio_init(void)
{
    state_lock = xSemaphoreCreateMutex();
    playback_events = xEventGroupCreate();
    esp_err_t result = state_lock != NULL && playback_events != NULL
                           ? ESP_OK
                           : ESP_ERR_NO_MEM;
    if (result == ESP_OK) {
        result = init_microphone();
    }
    if (result == ESP_OK) {
        result = init_speaker();
    }
    if (result == ESP_OK &&
        xTaskCreatePinnedToCore(
            capture_task,
            "bobe_audio_rx",
            4096,
            NULL,
            BOBE_TASK_PRIORITY_AUDIO,
            &capture_task_handle,
            BOBE_REALTIME_CORE
        ) != pdPASS) {
        capture_task_handle = NULL;
        result = ESP_ERR_NO_MEM;
    }
    if (result == ESP_OK &&
        xTaskCreatePinnedToCore(
            playback_task,
            "bobe_audio_tx",
            4096,
            NULL,
            BOBE_TASK_PRIORITY_AUDIO,
            &playback_task_handle,
            BOBE_REALTIME_CORE
        ) != pdPASS) {
        playback_task_handle = NULL;
        result = ESP_ERR_NO_MEM;
    }
    if (result != ESP_OK) {
        ESP_LOGE(TAG, "audio initialization failed: %s", esp_err_to_name(result));
        release_audio();
    }
    return result;
}

void bobe_audio_set_playback_progress_sink(
    bobe_audio_playback_progress_sink_t sink,
    void *context
)
{
    if (state_lock != NULL &&
        xSemaphoreTake(state_lock, portMAX_DELAY) == pdTRUE) {
        playback_progress_sink = sink;
        playback_progress_context = context;
        xSemaphoreGive(state_lock);
    }
}

void bobe_audio_set_capture_sink(
    bobe_audio_capture_sink_t sink,
    void *context
)
{
    if (state_lock != NULL &&
        xSemaphoreTake(state_lock, portMAX_DELAY) == pdTRUE) {
        capture_sink = sink;
        capture_sink_context = context;
        xSemaphoreGive(state_lock);
    }
}

esp_err_t bobe_audio_start_capture(uint32_t stream_id)
{
    if (stream_id == 0 || state_lock == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    if (xSemaphoreTake(state_lock, pdMS_TO_TICKS(100)) != pdTRUE) {
        return ESP_ERR_TIMEOUT;
    }
    if (atomic_load(&microphone_muted) ||
        atomic_load(&playing) || atomic_load(&capturing)) {
        xSemaphoreGive(state_lock);
        return ESP_ERR_INVALID_STATE;
    }
    atomic_store(&capture_stream_id, stream_id);
    atomic_store(&capture_sequence, 0);
    if (!atomic_load(&monitoring)) {
        const esp_err_t enable_result =
            i2s_channel_enable(microphone_channel);
        if (enable_result != ESP_OK) {
            xSemaphoreGive(state_lock);
            return enable_result;
        }
    }
    atomic_store(&capturing, true);
    xSemaphoreGive(state_lock);
    if (!atomic_load(&monitoring)) {
        xTaskNotifyGive(capture_task_handle);
    }
    return ESP_OK;
}

esp_err_t bobe_audio_stop_capture(void)
{
    if (state_lock == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    if (xSemaphoreTake(state_lock, pdMS_TO_TICKS(100)) != pdTRUE) {
        return ESP_ERR_TIMEOUT;
    }
    if (!atomic_load(&capturing)) {
        xSemaphoreGive(state_lock);
        return ESP_OK;
    }
    atomic_store(&capturing, false);
    const esp_err_t result = atomic_load(&monitoring)
                                 ? ESP_OK
                                 : i2s_channel_disable(microphone_channel);
    xSemaphoreGive(state_lock);
    return result;
}

esp_err_t bobe_audio_start_monitor(void)
{
    if (state_lock == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    if (xSemaphoreTake(state_lock, pdMS_TO_TICKS(100)) != pdTRUE) {
        return ESP_ERR_TIMEOUT;
    }
    if (atomic_load(&microphone_muted) ||
        atomic_load(&playing) ||
        atomic_load(&playback_closing)) {
        xSemaphoreGive(state_lock);
        return ESP_ERR_INVALID_STATE;
    }
    if (atomic_load(&monitoring)) {
        xSemaphoreGive(state_lock);
        return ESP_OK;
    }
    if (!atomic_load(&capturing)) {
        const esp_err_t result = i2s_channel_enable(microphone_channel);
        if (result != ESP_OK) {
            xSemaphoreGive(state_lock);
            return result;
        }
    }
    atomic_store(&monitoring, true);
    xSemaphoreGive(state_lock);
    xTaskNotifyGive(capture_task_handle);
    return ESP_OK;
}

esp_err_t bobe_audio_stop_monitor(void)
{
    if (state_lock == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    if (xSemaphoreTake(state_lock, pdMS_TO_TICKS(100)) != pdTRUE) {
        return ESP_ERR_TIMEOUT;
    }
    if (!atomic_load(&monitoring)) {
        xSemaphoreGive(state_lock);
        return ESP_OK;
    }
    atomic_store(&monitoring, false);
    const esp_err_t result = atomic_load(&capturing)
                                 ? ESP_OK
                                 : i2s_channel_disable(microphone_channel);
    xSemaphoreGive(state_lock);
    return result;
}

static esp_err_t open_playback_locked(uint32_t stream_id)
{
    if (stream_id == 0) {
        return ESP_ERR_INVALID_ARG;
    }
    if (atomic_load(&capturing) || atomic_load(&monitoring)) {
        return ESP_ERR_INVALID_STATE;
    }
    if (atomic_load(&playing) ||
        atomic_load(&playback_closing)) {
        return ESP_ERR_INVALID_STATE;
    }
    xEventGroupClearBits(playback_events, BOBE_PLAYBACK_TERMINAL_BIT);
    portENTER_CRITICAL(&playback_counter_lock);
    reset_playback_queue_locked();
    atomic_store(&playback_frames_accepted, 0);
    atomic_store(&playback_frames_completed, 0);
    atomic_store(&playback_stream_id, stream_id);
    const uint32_t generation =
        atomic_fetch_add(&playback_stream_generation, 1) + 1U;
    playback_terminal_generation = generation;
    playback_terminal_stream_id = stream_id;
    playback_terminal_result = PLAYBACK_TERMINAL_NONE;
    atomic_store(&playback_closing, false);
    atomic_store(&playing, true);
    atomic_store(&speaker_enabled, false);
    portEXIT_CRITICAL(&playback_counter_lock);
    return ESP_OK;
}

esp_err_t bobe_audio_open_playback(uint32_t stream_id)
{
    if (stream_id == 0U) {
        return ESP_ERR_INVALID_ARG;
    }
    if (state_lock == NULL) {
        return ESP_ERR_INVALID_STATE;
    }
    if (xSemaphoreTake(state_lock, pdMS_TO_TICKS(100)) != pdTRUE) {
        return ESP_ERR_TIMEOUT;
    }
    const esp_err_t result = open_playback_locked(stream_id);
    xSemaphoreGive(state_lock);
    return result;
}

esp_err_t bobe_audio_close_playback(uint32_t stream_id)
{
    if (state_lock == NULL ||
        xSemaphoreTake(state_lock, pdMS_TO_TICKS(100)) != pdTRUE) {
        return ESP_ERR_TIMEOUT;
    }
    if (stream_id != atomic_load(&playback_stream_id)) {
        xSemaphoreGive(state_lock);
        return ESP_ERR_INVALID_ARG;
    }
    if (!atomic_load(&playing)) {
        xSemaphoreGive(state_lock);
        return ESP_ERR_INVALID_STATE;
    }
    if (atomic_load(&playback_closing)) {
        xSemaphoreGive(state_lock);
        return ESP_ERR_INVALID_STATE;
    }
    atomic_store(&playback_closing, true);
    const uint32_t generation =
        atomic_load(&playback_stream_generation);
    if (atomic_load(&playback_frames_accepted) > 0 &&
        !atomic_load(&speaker_enabled)) {
        const esp_err_t enable_result = enable_speaker_locked();
        if (enable_result != ESP_OK) {
            atomic_store(&playback_closing, false);
            xSemaphoreGive(state_lock);
            return enable_result;
        }
    }
    portENTER_CRITICAL(&playback_counter_lock);
    const bool drained =
        playback_ring_count == 0 &&
        speaker_dma_audio_count == 0 &&
        atomic_load(&playback_frames_completed) >=
            atomic_load(&playback_frames_accepted);
    portEXIT_CRITICAL(&playback_counter_lock);
    if (drained) {
        (void)finish_playback_locked(generation, stream_id);
    } else {
        xTaskNotifyGive(playback_task_handle);
    }
    xSemaphoreGive(state_lock);
    const EventBits_t bits = xEventGroupWaitBits(
        playback_events,
        BOBE_PLAYBACK_TERMINAL_BIT,
        pdTRUE,
        pdTRUE,
        pdMS_TO_TICKS(CONFIG_BOBE_AUDIO_PLAYBACK_CREDIT_MS + 200)
    );
    if ((bits & BOBE_PLAYBACK_TERMINAL_BIT) == 0) {
        (void)xSemaphoreTake(state_lock, portMAX_DELAY);
        if (stream_id == atomic_load(&playback_stream_id) &&
            generation ==
                atomic_load(&playback_stream_generation)) {
            (void)abort_playback_locked(stream_id);
        }
        atomic_store(&playback_closing, false);
        xSemaphoreGive(state_lock);
        return ESP_ERR_TIMEOUT;
    }
    (void)xSemaphoreTake(state_lock, portMAX_DELAY);
    const bool same_stream =
        playback_terminal_generation == generation &&
        playback_terminal_stream_id == stream_id;
    const playback_terminal_result_t result =
        same_stream ? playback_terminal_result : PLAYBACK_TERMINAL_FAILED;
    atomic_store(&playback_closing, false);
    xSemaphoreGive(state_lock);
    switch (result) {
    case PLAYBACK_TERMINAL_DRAINED:
        return ESP_OK;
    case PLAYBACK_TERMINAL_ABORTED:
        return ESP_ERR_INVALID_STATE;
    case PLAYBACK_TERMINAL_NONE:
    case PLAYBACK_TERMINAL_FAILED:
        return ESP_FAIL;
    }
    return ESP_FAIL;
}

static bool abort_playback_locked(uint32_t required_stream_id)
{
    const uint32_t stream_id = atomic_load(&playback_stream_id);
    if (required_stream_id != 0U && stream_id != required_stream_id) {
        return false;
    }
    const uint32_t generation =
        atomic_load(&playback_stream_generation);
    portENTER_CRITICAL(&playback_counter_lock);
    const bool was_playing = atomic_exchange(&playing, false);
    const bool was_enabled = atomic_exchange(&speaker_enabled, false);
    portEXIT_CRITICAL(&playback_counter_lock);
    const bool close_waiter = atomic_load(&playback_closing);
    atomic_fetch_add(&playback_stream_generation, 1);
    const esp_err_t disable_result =
        was_playing && was_enabled
            ? i2s_channel_disable(speaker_channel)
            : ESP_OK;
    clear_playback_buffers_locked();
    portENTER_CRITICAL(&playback_counter_lock);
    atomic_store(&playback_frames_accepted, 0);
    atomic_store(&playback_frames_completed, 0);
    portEXIT_CRITICAL(&playback_counter_lock);
    atomic_store(&playback_stream_id, 0);
    if (stream_id != 0) {
        publish_playback_terminal_locked(
            generation,
            stream_id,
            disable_result == ESP_OK ? PLAYBACK_TERMINAL_ABORTED
                                     : PLAYBACK_TERMINAL_FAILED
        );
    }
    if (!close_waiter) {
        atomic_store(&playback_closing, false);
    }
    return true;
}

void bobe_audio_abort_playback(void)
{
    if (state_lock == NULL) {
        return;
    }
    (void)xSemaphoreTake(state_lock, portMAX_DELAY);
    (void)abort_playback_locked(0U);
    xSemaphoreGive(state_lock);
}

esp_err_t bobe_audio_enqueue_playback(const bobe_audio_frame_t *frame)
{
    if (frame == NULL || state_lock == NULL) {
        return ESP_ERR_INVALID_ARG;
    }
    if (xSemaphoreTake(state_lock, pdMS_TO_TICKS(20)) != pdTRUE) {
        return ESP_ERR_TIMEOUT;
    }
    if (!atomic_load(&playing) || atomic_load(&playback_closing) ||
        frame->stream_id != atomic_load(&playback_stream_id) ||
        frame->sample_count != BOBE_SPEAKER_FRAME_SAMPLES) {
        xSemaphoreGive(state_lock);
        return ESP_ERR_INVALID_STATE;
    }
    bool queued = false;
    bool should_start = false;
    int16_t scaled_samples[BOBE_SPEAKER_FRAME_SAMPLES];
    const int32_t volume = (int32_t)atomic_load(&volume_percent);
    for (size_t index = 0;
         index < BOBE_SPEAKER_FRAME_SAMPLES;
         ++index) {
        scaled_samples[index] =
            (int16_t)(((int32_t)frame->samples[index] * volume) / 100);
    }
    portENTER_CRITICAL(&playback_counter_lock);
    const uint32_t accepted = atomic_load(&playback_frames_accepted);
    const uint32_t completed = atomic_load(&playback_frames_completed);
    if (accepted - completed < BOBE_PLAYBACK_CREDIT_FRAMES &&
        playback_ring_count < BOBE_PLAYBACK_CREDIT_FRAMES) {
        memcpy(
            playback_ring[playback_ring_write],
            scaled_samples,
            sizeof(scaled_samples)
        );
        playback_ring_write =
            (playback_ring_write + 1U) % BOBE_PLAYBACK_CREDIT_FRAMES;
        ++playback_ring_count;
        atomic_store(&playback_frames_accepted, accepted + 1U);
        should_start =
            !atomic_load(&speaker_enabled) &&
            playback_ring_count == BOBE_PLAYBACK_CREDIT_FRAMES;
        queued = true;
    }
    portEXIT_CRITICAL(&playback_counter_lock);
    if (!queued) {
        xSemaphoreGive(state_lock);
        return ESP_ERR_NO_MEM;
    }
    if (should_start) {
        const esp_err_t start_result = enable_speaker_locked();
        if (start_result != ESP_OK) {
            xSemaphoreGive(state_lock);
            return start_result;
        }
    }
    xSemaphoreGive(state_lock);
    return ESP_OK;
}

void bobe_audio_set_muted(bool muted)
{
    atomic_store(&microphone_muted, muted);
    if (state_lock == NULL) {
        return;
    }
    xSemaphoreTake(state_lock, portMAX_DELAY);
    if (muted) {
        if (atomic_load(&capturing) || atomic_load(&monitoring)) {
            const esp_err_t result = i2s_channel_disable(microphone_channel);
            if (result != ESP_OK) {
                ESP_LOGW(
                    TAG,
                    "microphone disable on mute failed: %s",
                    esp_err_to_name(result)
                );
            }
        }
        atomic_store(&capturing, false);
        atomic_store(&monitoring, false);
        atomic_store(&capture_stream_id, 0);
        atomic_store(&capture_sequence, 0);
    }
    xSemaphoreGive(state_lock);
}

void bobe_audio_set_volume(uint8_t percent)
{
    atomic_store(&volume_percent, percent <= 100 ? percent : 100);
}

uint8_t bobe_audio_volume(void)
{
    return (uint8_t)atomic_load(&volume_percent);
}

bool bobe_audio_is_capturing(void)
{
    return atomic_load(&capturing);
}

uint32_t bobe_audio_capture_overruns(void)
{
    return atomic_load(&capture_overruns);
}

uint32_t bobe_audio_playback_underruns(void)
{
    return atomic_load(&playback_underruns);
}

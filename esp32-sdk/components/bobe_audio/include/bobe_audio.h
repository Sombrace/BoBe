#pragma once

#include "bobe_types.h"
#include "esp_err.h"

#include <stdbool.h>
#include <stdint.h>

typedef bool (*bobe_audio_capture_sink_t)(
    const bobe_audio_frame_t *frame,
    void *context
);

typedef void (*bobe_audio_playback_progress_sink_t)(
    uint32_t stream_id,
    uint64_t played_samples,
    uint32_t credit_ms,
    void *context
);

esp_err_t bobe_audio_init(void);
void bobe_audio_set_capture_sink(
    bobe_audio_capture_sink_t sink,
    void *context
);
void bobe_audio_set_playback_progress_sink(
    bobe_audio_playback_progress_sink_t sink,
    void *context
);
esp_err_t bobe_audio_start_capture(uint32_t stream_id);
esp_err_t bobe_audio_stop_capture(void);
esp_err_t bobe_audio_start_monitor(void);
esp_err_t bobe_audio_stop_monitor(void);
esp_err_t bobe_audio_open_playback(uint32_t stream_id);
esp_err_t bobe_audio_close_playback(uint32_t stream_id);
void bobe_audio_abort_playback(void);
esp_err_t bobe_audio_enqueue_playback(const bobe_audio_frame_t *frame);
void bobe_audio_set_muted(bool muted);
void bobe_audio_set_volume(uint8_t percent);
uint8_t bobe_audio_volume(void);
bool bobe_audio_is_capturing(void);
uint32_t bobe_audio_capture_overruns(void);
uint32_t bobe_audio_playback_underruns(void);

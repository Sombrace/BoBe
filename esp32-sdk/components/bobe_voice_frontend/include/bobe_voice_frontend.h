#pragma once

#include "bobe_types.h"
#include "esp_err.h"

#include <stdbool.h>
#include <stdint.h>

typedef enum {
    BOBE_VOICE_EVENT_SPEECH_STARTED = 0,
    BOBE_VOICE_EVENT_SPEECH_ENDED,
    BOBE_VOICE_EVENT_WAKE_DETECTED,
} bobe_voice_event_kind_t;

typedef struct {
    bobe_voice_event_kind_t kind;
    float level_db;
    int wake_word_index;
} bobe_voice_event_t;

/* Sinks run on AFE workers and must not block or re-enter this API. */
typedef bool (*bobe_voice_output_sink_t)(
    const bobe_audio_frame_t *frame,
    void *context
);

typedef void (*bobe_voice_event_sink_t)(
    const bobe_voice_event_t *event,
    void *context
);

esp_err_t bobe_voice_frontend_init(
    bobe_voice_output_sink_t output_sink,
    void *output_context,
    bobe_voice_event_sink_t event_sink,
    void *event_context
);
esp_err_t bobe_voice_frontend_begin_stream(uint32_t stream_id);
esp_err_t bobe_voice_frontend_end_stream(void);
esp_err_t bobe_voice_frontend_start_monitor(void);
void bobe_voice_frontend_stop_monitor(void);
bool bobe_voice_frontend_submit(const bobe_audio_frame_t *frame);
bool bobe_voice_frontend_wake_available(void);

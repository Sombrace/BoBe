#pragma once

#include "bobe_types.h"
#include "esp_err.h"

#include <stdint.h>

typedef struct {
    uint32_t stack_free_words;
    uint32_t flush_submitted;
    uint32_t flush_errors;
    uint32_t flush_timeouts;
    uint32_t flush_last_us;
    uint32_t flush_max_us;
    uint32_t frame_rate;
} bobe_display_stats_t;

esp_err_t bobe_display_init(void);
void bobe_display_set_face(const bobe_face_state_t *face);
void bobe_display_set_theme(const bobe_theme_state_t *theme);
void bobe_display_set_microphone_active(bool active);
void bobe_display_show_notice(const bobe_notice_t *notice);
void bobe_display_set_status(const bobe_display_status_t *status);
void bobe_display_set_frame_rate(uint8_t frames_per_second);
void bobe_display_get_stats(bobe_display_stats_t *stats);

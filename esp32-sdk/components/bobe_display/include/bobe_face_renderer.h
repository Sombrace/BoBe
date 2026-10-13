#pragma once

#include "bobe_face_theme.h"
#include "bobe_types.h"
#include "lvgl.h"

#include <stdbool.h>
#include <stdint.h>

#define BOBE_FACE_SCROLL_AUTOMATIC INT32_MIN

typedef struct {
    bobe_face_state_t face;
    bobe_face_theme_id_t theme;
    bool microphone_active;
    bool face_changed;
    bool theme_changed;
    /*
     * Transient local confirmation drawn over the caption. An empty text or
     * BOBE_NOTICE_NONE hides it; the caller decides expiry.
     */
    bobe_notice_t notice;
    bobe_display_status_t status;
    uint32_t animation_phase;
    uint32_t lvgl_tick_ms;
    int32_t caption_scroll_position;
} bobe_face_render_input_t;

/*
 * One shared superset object tree covers every theme; switching a theme only
 * repositions, recolours, and hides objects.
 */
typedef struct {
    lv_obj_t *screen;
    lv_obj_t *decor[BOBE_FACE_MAX_DECOR];
    lv_obj_t *decor_arcs[BOBE_FACE_MAX_DECOR_ARCS];
    lv_obj_t *eye[2];
    lv_obj_t *iris[2];
    lv_obj_t *pupil[2];
    lv_obj_t *highlight[2];
    lv_obj_t *mouth;
    lv_obj_t *smile;
    lv_obj_t *caption;
    lv_obj_t *status;
    lv_obj_t *microphone_indicator;
    lv_obj_t *wifi_bar[3];
    lv_obj_t *battery_body;
    lv_obj_t *battery_fill;
    lv_obj_t *battery_tip;
    const bobe_face_theme_t *theme;
    bobe_face_theme_id_t theme_id;
    uint32_t applied_accent_rgb;
    char applied_notice[BOBE_NOTICE_MAX_BYTES];
    bobe_display_status_t applied_status;
    bool status_initialized;
    bool deterministic_scroll_active;
} bobe_face_renderer_t;

void bobe_face_renderer_create(
    bobe_face_renderer_t *renderer,
    lv_obj_t *screen,
    const bobe_face_render_input_t *input
);
void bobe_face_renderer_render(
    bobe_face_renderer_t *renderer,
    const bobe_face_render_input_t *input
);

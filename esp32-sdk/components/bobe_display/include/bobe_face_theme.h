#pragma once

#include "bobe_types.h"

#include <stdbool.h>
#include <stdint.h>

#define BOBE_FACE_MAX_DECOR 9U
#define BOBE_FACE_MAX_DECOR_ARCS 2U

#define BOBE_FACE_RGB_ACCENT UINT32_C(0xFF000000)
#define BOBE_FACE_RGB_NONE UINT32_C(0xFF000001)
#define BOBE_FACE_RADIUS_CIRCLE INT16_C(0x7FFF)

#define BOBE_FACE_CLOSED_EYE_HEIGHT 5
#define BOBE_FACE_MIN_FEATURE_SIZE 2

typedef enum {
    BOBE_FACE_FONT_SMALL = 0,
    BOBE_FACE_FONT_LARGE,
} bobe_face_font_t;

typedef enum {
    BOBE_FACE_ALIGN_LEFT = 0,
    BOBE_FACE_ALIGN_CENTER,
} bobe_face_align_t;

typedef enum {
    BOBE_FACE_EYE_DERIVED = 0,
    BOBE_FACE_EYE_FIXED,
    BOBE_FACE_EYE_GLOSS,
} bobe_face_eye_style_t;

typedef struct {
    int16_t x;
    int16_t y;
    int16_t width;
    int16_t height;
    int16_t radius;
    uint32_t fill_rgb;
    uint32_t border_rgb;
    uint8_t border_width;
} bobe_face_decor_t;

typedef struct {
    int16_t x;
    int16_t y;
    int16_t width;
    int16_t height;
    int16_t start_angle;
    int16_t end_angle;
    uint8_t stroke;
    uint32_t rgb;
} bobe_face_decor_arc_t;

typedef struct {
    int16_t x;
    int16_t y;
    int16_t width;
    int16_t height;
    int16_t radius;
    int16_t iris;
    int16_t pupil;
    int16_t highlight;
    int16_t pupil_dx;
    int16_t pupil_dy;
    int16_t highlight_dx;
    int16_t highlight_dy;
} bobe_face_eye_t;

typedef struct {
    int16_t center_x;
    int16_t smile_y;
    int16_t smile_width;
    int16_t smile_height;
    uint8_t smile_stroke;
    uint32_t smile_rgb;
    int16_t rect_y;
    int16_t rect_width;
    int16_t rect_height;
    int16_t rect_radius;
    uint8_t rect_border_width;
    uint32_t rect_fill_rgb;
    uint32_t rect_border_rgb;
} bobe_face_mouth_t;

typedef struct {
    int16_t x;
    int16_t y;
    int16_t width;
    bobe_face_font_t font;
    bobe_face_align_t align;
    uint32_t rgb;
} bobe_face_label_t;

typedef struct {
    int16_t x;
    int16_t y;
    int16_t size;
    uint32_t rgb;
} bobe_face_dot_t;

typedef struct {
    uint32_t background_rgb;
    uint32_t accent_rgb;
    const bobe_face_decor_t *decor;
    const bobe_face_decor_arc_t *arcs;
    uint8_t decor_count;
    uint8_t arc_count;
    bobe_face_eye_style_t eye_style;
    uint8_t eye_border_width;
    uint32_t eye_fill_rgb;
    uint32_t eye_border_rgb;
    uint32_t iris_rgb;
    uint32_t pupil_rgb;
    uint32_t highlight_rgb;
    bobe_face_eye_t left_eye;
    bobe_face_eye_t right_eye;
    bobe_face_mouth_t mouth;
    bobe_face_label_t caption;
    bobe_face_dot_t microphone;
} bobe_face_theme_t;

typedef enum {
    BOBE_FACE_MOUTH_SMILE = 0,
    BOBE_FACE_MOUTH_RECT,
    BOBE_FACE_MOUTH_NONE,
} bobe_face_mouth_kind_t;

typedef struct {
    int16_t x;
    int16_t y;
    int16_t width;
    int16_t height;
    bool visible;
} bobe_face_box_t;

typedef struct {
    bobe_face_box_t eye[2];
    bobe_face_box_t iris[2];
    bobe_face_box_t pupil[2];
    bobe_face_box_t highlight[2];
    bool eyes_open;
    uint32_t eye_fill_rgb;
    uint32_t eye_border_rgb;
    uint8_t eye_border_width;
    bobe_face_mouth_kind_t mouth_kind;
    bobe_face_box_t mouth;
    int16_t smile_start_angle;
    int16_t smile_end_angle;
    uint32_t mouth_fill_rgb;
    uint32_t mouth_line_rgb;
    uint32_t accent_rgb;
    uint32_t status_rgb;
} bobe_face_layout_t;

const bobe_face_theme_t *bobe_face_theme(bobe_face_theme_id_t theme);

void bobe_face_layout_resolve(
    const bobe_face_theme_t *theme,
    const bobe_face_state_t *face,
    uint32_t animation_phase,
    bobe_face_layout_t *layout
);

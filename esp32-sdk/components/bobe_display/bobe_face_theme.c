#include "bobe_device_config.h"
#include "bobe_face_theme.h"

#include <stddef.h>

#define BOBE_FACE_SAND_RGB UINT32_C(0xE8DCC4)
#define BOBE_FACE_FEATURE_RGB UINT32_C(0x3A3A3A)
#define BOBE_FACE_MOON_RGB UINT32_C(0xA69080)
#define BOBE_FACE_EYE_WHITE_RGB UINT32_C(0xFFFDFC)
#define BOBE_FACE_BACKGROUND_RGB UINT32_C(0xFAF7F2)
#define BOBE_FACE_MICROPHONE_RGB UINT32_C(0xFF3B5C)
#define BOBE_FACE_ERROR_STATUS_RGB UINT32_C(0xFF5A6F)

#define BOBE_CONCEPT_IRIS_RGB UINT32_C(0x8A6A55)
#define BOBE_CONCEPT_PUPIL_RGB UINT32_C(0x3A3634)
#define BOBE_CONCEPT_HIGHLIGHT_RGB UINT32_C(0xFFFFFF)

#define CIRCLE BOBE_FACE_RADIUS_CIRCLE
#define ACCENT BOBE_FACE_RGB_ACCENT
#define NO_FILL BOBE_FACE_RGB_NONE

#define CIRCLE_DECOR(center_x, center_y, diameter, colour)                   \
    {                                                                        \
        .x = (int16_t)((center_x) - (diameter) / 2),                         \
        .y = (int16_t)((center_y) - (diameter) / 2),                         \
        .width = (diameter),                                                 \
        .height = (diameter),                                                \
        .radius = CIRCLE,                                                    \
        .fill_rgb = (colour),                                                \
        .border_rgb = NO_FILL,                                               \
        .border_width = 0,                                                   \
    }

static const bobe_face_decor_t default_decor[] = {
    {
        .x = 48,
        .y = 26,
        .width = 184,
        .height = 184,
        .radius = CIRCLE,
        .fill_rgb = NO_FILL,
        .border_rgb = ACCENT,
        .border_width = 8,
    },
    {
        .x = 65,
        .y = 48,
        .width = 150,
        .height = 150,
        .radius = CIRCLE,
        .fill_rgb = BOBE_FACE_SAND_RGB,
        .border_rgb = BOBE_FACE_EYE_WHITE_RGB,
        .border_width = 4,
    },
};

static const bobe_face_decor_t warm_aperture_decor[] = {
    {
        .x = 30,
        .y = 8,
        .width = 220,
        .height = 196,
        .radius = 34,
        .fill_rgb = UINT32_C(0xFDF9F1),
        .border_rgb = UINT32_C(0xD3BC9E),
        .border_width = 2,
    },
    {
        .x = 18,
        .y = 216,
        .width = 74,
        .height = 3,
        .radius = 2,
        .fill_rgb = UINT32_C(0xA8632F),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
    {
        .x = 188,
        .y = 216,
        .width = 74,
        .height = 3,
        .radius = 2,
        .fill_rgb = UINT32_C(0xA8632F),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
};

static const bobe_face_decor_arc_t warm_aperture_arcs[] = {
    {
        .x = 72,
        .y = 40,
        .width = 64,
        .height = 40,
        .start_angle = 200,
        .end_angle = 340,
        .stroke = 5,
        .rgb = UINT32_C(0x3A3634),
    },
    {
        .x = 144,
        .y = 40,
        .width = 64,
        .height = 40,
        .start_angle = 200,
        .end_angle = 340,
        .stroke = 5,
        .rgb = UINT32_C(0x3A3634),
    },
};

static const bobe_face_decor_t warm_lintel_decor[] = {
    {
        .x = 8,
        .y = 10,
        .width = 264,
        .height = 220,
        .radius = 20,
        .fill_rgb = UINT32_C(0xEADCC0),
        .border_rgb = UINT32_C(0xC6AFA0),
        .border_width = 2,
    },
    {
        .x = 10,
        .y = 12,
        .width = 260,
        .height = 32,
        .radius = 18,
        .fill_rgb = UINT32_C(0xE0CDAA),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
    {
        .x = 36,
        .y = 66,
        .width = 84,
        .height = 8,
        .radius = 4,
        .fill_rgb = UINT32_C(0x2F2A26),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
    {
        .x = 160,
        .y = 66,
        .width = 84,
        .height = 8,
        .radius = 4,
        .fill_rgb = UINT32_C(0x2F2A26),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
    {
        .x = 92,
        .y = 186,
        .width = 96,
        .height = 3,
        .radius = 2,
        .fill_rgb = UINT32_C(0xB5764F),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
};

static const bobe_face_decor_t warm_ledge_decor[] = {
    {
        .x = -20,
        .y = -40,
        .width = 320,
        .height = 96,
        .radius = 48,
        .fill_rgb = UINT32_C(0xF7EEDD),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
    {
        .x = 68,
        .y = 40,
        .width = 40,
        .height = 4,
        .radius = 2,
        .fill_rgb = UINT32_C(0xB4522F),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
    {
        .x = 172,
        .y = 40,
        .width = 40,
        .height = 4,
        .radius = 2,
        .fill_rgb = UINT32_C(0xB4522F),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
    {
        .x = 0,
        .y = 192,
        .width = 280,
        .height = 2,
        .radius = 0,
        .fill_rgb = UINT32_C(0xB4522F),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
    {
        .x = 0,
        .y = 194,
        .width = 280,
        .height = 46,
        .radius = 0,
        .fill_rgb = UINT32_C(0xFBF7EF),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
};

static const bobe_face_decor_t kettle_tall_bright_decor[] = {
    CIRCLE_DECOR(67, 145, 18, UINT32_C(0xC97B5A)),
    CIRCLE_DECOR(213, 145, 18, UINT32_C(0xC97B5A)),
    {
        .x = 0,
        .y = 208,
        .width = 280,
        .height = 2,
        .radius = 0,
        .fill_rgb = UINT32_C(0xE0A35C),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
    {
        .x = 0,
        .y = 210,
        .width = 280,
        .height = 30,
        .radius = 0,
        .fill_rgb = UINT32_C(0x4A3F35),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
};

static const bobe_face_decor_t kettle_soft_bowl_decor[] = {
    CIRCLE_DECOR(61, 152, 24, UINT32_C(0xE5B4A2)),
    CIRCLE_DECOR(219, 152, 24, UINT32_C(0xE5B4A2)),
    {
        .x = 20,
        .y = 206,
        .width = 240,
        .height = 34,
        .radius = 16,
        .fill_rgb = UINT32_C(0xDCCBA9),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
};

static const bobe_face_decor_t solar_wide_awake_decor[] = {
    {
        .x = 2,
        .y = 2,
        .width = 276,
        .height = 236,
        .radius = 42,
        .fill_rgb = BOBE_FACE_SAND_RGB,
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
    CIRCLE_DECOR(100, 5, 72, BOBE_FACE_MOON_RGB),
    CIRCLE_DECOR(140, -2, 86, BOBE_FACE_EYE_WHITE_RGB),
    CIRCLE_DECOR(187, 4, 70, BOBE_FACE_MOON_RGB),
    CIRCLE_DECOR(27, 141, 12, UINT32_C(0x94472E)),
    CIRCLE_DECOR(253, 141, 12, UINT32_C(0x94472E)),
};

static const bobe_face_decor_t solar_crown_decor[] = {
    {
        .x = 2,
        .y = 2,
        .width = 276,
        .height = 236,
        .radius = 52,
        .fill_rgb = UINT32_C(0xBE9279),
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
    CIRCLE_DECOR(76, 8, 46, BOBE_FACE_MOON_RGB),
    CIRCLE_DECOR(108, 1, 58, BOBE_FACE_SAND_RGB),
    CIRCLE_DECOR(140, -5, 68, BOBE_FACE_MOON_RGB),
    CIRCLE_DECOR(174, 1, 58, BOBE_FACE_SAND_RGB),
    CIRCLE_DECOR(207, 8, 46, BOBE_FACE_MOON_RGB),
    CIRCLE_DECOR(27, 137, 12, UINT32_C(0x94472E)),
    CIRCLE_DECOR(253, 137, 12, UINT32_C(0x94472E)),
    {
        .x = 14,
        .y = 198,
        .width = 92,
        .height = 28,
        .radius = 14,
        .fill_rgb = BOBE_FACE_SAND_RGB,
        .border_rgb = NO_FILL,
        .border_width = 0,
    },
};

static const bobe_face_theme_t themes[BOBE_FACE_THEME_COUNT] = {
    [BOBE_FACE_THEME_DEFAULT] = {
        .background_rgb = BOBE_FACE_BACKGROUND_RGB,
        .accent_rgb = BOBE_FACE_DEFAULT_ACCENT_RGB,
        .decor = default_decor,
        .decor_count =
            (uint8_t)(sizeof(default_decor) / sizeof(default_decor[0])),
        .eye_style = BOBE_FACE_EYE_DERIVED,
        .eye_border_width = 2,
        .eye_fill_rgb = BOBE_FACE_EYE_WHITE_RGB,
        .eye_border_rgb = BOBE_FACE_FEATURE_RGB,
        .iris_rgb = BOBE_FACE_MOON_RGB,
        .pupil_rgb = BOBE_FACE_FEATURE_RGB,
        .highlight_rgb = BOBE_FACE_EYE_WHITE_RGB,
        .left_eye = {
            .x = 94,
            .y = 99,
            .width = 40,
            .height = 34,
            .radius = CIRCLE,
        },
        .right_eye = {
            .x = 146,
            .y = 99,
            .width = 40,
            .height = 34,
            .radius = CIRCLE,
        },
        .mouth = {
            .center_x = 140,
            .smile_y = 136,
            .smile_width = 44,
            .smile_height = 22,
            .smile_stroke = 6,
            .smile_rgb = ACCENT,
            .rect_y = 139,
            .rect_width = 40,
            .rect_height = 15,
            .rect_radius = CIRCLE,
            .rect_border_width = 3,
            .rect_fill_rgb = BOBE_FACE_FEATURE_RGB,
            .rect_border_rgb = ACCENT,
        },
        .caption = {
            .x = 12,
            .y = (int16_t)(BOBE_DISPLAY_HEIGHT - 26U),
            .width = (int16_t)(BOBE_DISPLAY_WIDTH - 24U),
            .font = BOBE_FACE_FONT_LARGE,
            .align = BOBE_FACE_ALIGN_CENTER,
            .rgb = BOBE_FACE_FEATURE_RGB,
        },
        .microphone = {
            .x = 91,
            .y = 12,
            .size = 9,
            .rgb = BOBE_FACE_MICROPHONE_RGB,
        },
    },
    [BOBE_FACE_THEME_WARM_APERTURE] = {
        .background_rgb = BOBE_FACE_SAND_RGB,
        .accent_rgb = UINT32_C(0xA8632F),
        .decor = warm_aperture_decor,
        .decor_count = (uint8_t)(sizeof(warm_aperture_decor) /
                                 sizeof(warm_aperture_decor[0])),
        .arcs = warm_aperture_arcs,
        .arc_count = (uint8_t)(sizeof(warm_aperture_arcs) /
                               sizeof(warm_aperture_arcs[0])),
        .eye_style = BOBE_FACE_EYE_FIXED,
        .eye_border_width = 3,
        .eye_fill_rgb = BOBE_FACE_EYE_WHITE_RGB,
        .eye_border_rgb = UINT32_C(0x3A3634),
        .iris_rgb = BOBE_CONCEPT_IRIS_RGB,
        .pupil_rgb = BOBE_CONCEPT_PUPIL_RGB,
        .highlight_rgb = BOBE_CONCEPT_HIGHLIGHT_RGB,
        .left_eye = {
            .x = 76,
            .y = 64,
            .width = 56,
            .height = 64,
            .radius = 26,
            .iris = 34,
            .pupil = 16,
        },
        .right_eye = {
            .x = 148,
            .y = 64,
            .width = 56,
            .height = 64,
            .radius = 26,
            .iris = 34,
            .pupil = 16,
        },
        .mouth = {
            .center_x = 140,
            .smile_y = 148,
            .smile_width = 56,
            .smile_height = 28,
            .smile_stroke = 6,
            .smile_rgb = ACCENT,
            .rect_y = 148,
            .rect_width = 20,
            .rect_height = 20,
            .rect_radius = CIRCLE,
            .rect_border_width = 0,
            .rect_fill_rgb = UINT32_C(0x3A3634),
            .rect_border_rgb = ACCENT,
        },
        .caption = {
            .x = 100,
            .y = 210,
            .width = 80,
            .font = BOBE_FACE_FONT_SMALL,
            .align = BOBE_FACE_ALIGN_CENTER,
            .rgb = UINT32_C(0x3A3634),
        },
        .microphone = {
            .x = 91,
            .y = 12,
            .size = 9,
            .rgb = BOBE_FACE_MICROPHONE_RGB,
        },
    },
    [BOBE_FACE_THEME_WARM_LINTEL] = {
        .background_rgb = BOBE_FACE_BACKGROUND_RGB,
        .accent_rgb = UINT32_C(0xB5764F),
        .decor = warm_lintel_decor,
        .decor_count = (uint8_t)(sizeof(warm_lintel_decor) /
                                 sizeof(warm_lintel_decor[0])),
        .eye_style = BOBE_FACE_EYE_FIXED,
        .eye_border_width = 3,
        .eye_fill_rgb = BOBE_FACE_EYE_WHITE_RGB,
        .eye_border_rgb = UINT32_C(0x2F2A26),
        .iris_rgb = BOBE_CONCEPT_IRIS_RGB,
        .pupil_rgb = BOBE_CONCEPT_PUPIL_RGB,
        .highlight_rgb = BOBE_CONCEPT_HIGHLIGHT_RGB,
        .left_eye = {
            .x = 36,
            .y = 74,
            .width = 84,
            .height = 56,
            .radius = 24,
            .iris = 30,
            .pupil = 14,
        },
        .right_eye = {
            .x = 160,
            .y = 74,
            .width = 84,
            .height = 56,
            .radius = 24,
            .iris = 30,
            .pupil = 14,
        },
        .mouth = {
            .center_x = 140,
            .smile_y = 150,
            .smile_width = 46,
            .smile_height = 24,
            .smile_stroke = 6,
            .smile_rgb = ACCENT,
            .rect_y = 150,
            .rect_width = 46,
            .rect_height = 14,
            .rect_radius = 7,
            .rect_border_width = 0,
            .rect_fill_rgb = UINT32_C(0xB5764F),
            .rect_border_rgb = ACCENT,
        },
        .caption = {
            .x = 24,
            .y = 198,
            .width = 150,
            .font = BOBE_FACE_FONT_LARGE,
            .align = BOBE_FACE_ALIGN_LEFT,
            .rgb = UINT32_C(0x2F2A26),
        },
        .microphone = {
            .x = 246,
            .y = 20,
            .size = 12,
            .rgb = BOBE_FACE_MICROPHONE_RGB,
        },
    },
    [BOBE_FACE_THEME_WARM_LEDGE] = {
        .background_rgb = UINT32_C(0xF1E4CA),
        .accent_rgb = UINT32_C(0xB4522F),
        .decor = warm_ledge_decor,
        .decor_count = (uint8_t)(sizeof(warm_ledge_decor) /
                                 sizeof(warm_ledge_decor[0])),
        .eye_style = BOBE_FACE_EYE_FIXED,
        .eye_border_width = 3,
        .eye_fill_rgb = BOBE_FACE_EYE_WHITE_RGB,
        .eye_border_rgb = UINT32_C(0x2C2622),
        .iris_rgb = BOBE_CONCEPT_IRIS_RGB,
        .pupil_rgb = BOBE_CONCEPT_PUPIL_RGB,
        .highlight_rgb = BOBE_CONCEPT_HIGHLIGHT_RGB,
        .left_eye = {
            .x = 62,
            .y = 56,
            .width = 52,
            .height = 84,
            .radius = 26,
            .iris = 30,
            .pupil = 14,
        },
        .right_eye = {
            .x = 166,
            .y = 56,
            .width = 52,
            .height = 84,
            .radius = 26,
            .iris = 30,
            .pupil = 14,
        },
        .mouth = {
            .center_x = 140,
            .smile_y = 152,
            .smile_width = 56,
            .smile_height = 26,
            .smile_stroke = 6,
            .smile_rgb = ACCENT,
            .rect_y = 158,
            .rect_width = 56,
            .rect_height = 8,
            .rect_radius = 4,
            .rect_border_width = 0,
            .rect_fill_rgb = UINT32_C(0x2C2622),
            .rect_border_rgb = ACCENT,
        },
        .caption = {
            .x = 16,
            .y = 204,
            .width = 160,
            .font = BOBE_FACE_FONT_LARGE,
            .align = BOBE_FACE_ALIGN_LEFT,
            .rgb = UINT32_C(0x2C2622),
        },
        .microphone = {
            .x = 245,
            .y = 209,
            .size = 14,
            .rgb = UINT32_C(0xB4522F),
        },
    },
    [BOBE_FACE_THEME_KETTLE_TALL_BRIGHT] = {
        .background_rgb = UINT32_C(0xF4EADA),
        .accent_rgb = UINT32_C(0xB5644A),
        .decor = kettle_tall_bright_decor,
        .decor_count = (uint8_t)(sizeof(kettle_tall_bright_decor) /
                                 sizeof(kettle_tall_bright_decor[0])),
        .eye_style = BOBE_FACE_EYE_GLOSS,
        .eye_border_width = 0,
        .eye_fill_rgb = UINT32_C(0x2E2A26),
        .eye_border_rgb = UINT32_C(0x2E2A26),
        .iris_rgb = UINT32_C(0x2E2A26),
        .pupil_rgb = UINT32_C(0xFAF7F2),
        .highlight_rgb = UINT32_C(0xFAF7F2),
        .left_eye = {
            .x = 70,
            .y = 70,
            .width = 40,
            .height = 54,
            .radius = 20,
            .pupil = 10,
            .highlight = 5,
            .pupil_dx = 13,
            .pupil_dy = 13,
            .highlight_dx = 24,
            .highlight_dy = 29,
        },
        .right_eye = {
            .x = 170,
            .y = 70,
            .width = 40,
            .height = 54,
            .radius = 20,
            .pupil = 10,
            .highlight = 5,
            .pupil_dx = 13,
            .pupil_dy = 13,
            .highlight_dx = 24,
            .highlight_dy = 29,
        },
        .mouth = {
            .center_x = 140,
            .smile_y = 140,
            .smile_width = 44,
            .smile_height = 24,
            .smile_stroke = 6,
            .smile_rgb = ACCENT,
            .rect_y = 140,
            .rect_width = 34,
            .rect_height = 22,
            .rect_radius = 11,
            .rect_border_width = 3,
            .rect_fill_rgb = UINT32_C(0x2E2A26),
            .rect_border_rgb = ACCENT,
        },
        .caption = {
            .x = 16,
            .y = 216,
            .width = 248,
            .font = BOBE_FACE_FONT_SMALL,
            .align = BOBE_FACE_ALIGN_CENTER,
            .rgb = UINT32_C(0xFAF7F2),
        },
        .microphone = {
            .x = 246,
            .y = 18,
            .size = 12,
            .rgb = BOBE_FACE_MICROPHONE_RGB,
        },
    },
    [BOBE_FACE_THEME_KETTLE_SOFT_BOWL] = {
        .background_rgb = UINT32_C(0xF4EADA),
        .accent_rgb = UINT32_C(0xA85A3C),
        .decor = kettle_soft_bowl_decor,
        .decor_count = (uint8_t)(sizeof(kettle_soft_bowl_decor) /
                                 sizeof(kettle_soft_bowl_decor[0])),
        .eye_style = BOBE_FACE_EYE_GLOSS,
        .eye_border_width = 0,
        .eye_fill_rgb = UINT32_C(0x2E2A26),
        .eye_border_rgb = UINT32_C(0x2E2A26),
        .iris_rgb = UINT32_C(0x2E2A26),
        .pupil_rgb = UINT32_C(0xFAF7F2),
        .highlight_rgb = UINT32_C(0xFAF7F2),
        .left_eye = {
            .x = 62,
            .y = 96,
            .width = 48,
            .height = 44,
            .radius = 22,
            .pupil = 14,
            .pupil_dx = 15,
            .pupil_dy = 15,
        },
        .right_eye = {
            .x = 170,
            .y = 96,
            .width = 48,
            .height = 44,
            .radius = 22,
            .pupil = 14,
            .pupil_dx = 15,
            .pupil_dy = 15,
        },
        .mouth = {
            .center_x = 140,
            .smile_y = 98,
            .smile_width = 90,
            .smile_height = 90,
            .smile_stroke = 10,
            .smile_rgb = ACCENT,
            .rect_y = 132,
            .rect_width = 40,
            .rect_height = 18,
            .rect_radius = CIRCLE,
            .rect_border_width = 3,
            .rect_fill_rgb = UINT32_C(0x2E2A26),
            .rect_border_rgb = ACCENT,
        },
        .caption = {
            .x = 32,
            .y = 213,
            .width = 216,
            .font = BOBE_FACE_FONT_SMALL,
            .align = BOBE_FACE_ALIGN_CENTER,
            .rgb = UINT32_C(0x2E2A26),
        },
        .microphone = {
            .x = 246,
            .y = 18,
            .size = 12,
            .rgb = BOBE_FACE_MICROPHONE_RGB,
        },
    },
    [BOBE_FACE_THEME_SOLAR_WIDE_AWAKE] = {
        .background_rgb = BOBE_FACE_BACKGROUND_RGB,
        .accent_rgb = BOBE_FACE_FEATURE_RGB,
        .decor = solar_wide_awake_decor,
        .decor_count = (uint8_t)(sizeof(solar_wide_awake_decor) /
                                 sizeof(solar_wide_awake_decor[0])),
        .eye_style = BOBE_FACE_EYE_FIXED,
        .eye_border_width = 3,
        .eye_fill_rgb = BOBE_FACE_EYE_WHITE_RGB,
        .eye_border_rgb = BOBE_FACE_FEATURE_RGB,
        .iris_rgb = BOBE_CONCEPT_IRIS_RGB,
        .pupil_rgb = BOBE_CONCEPT_PUPIL_RGB,
        .highlight_rgb = BOBE_CONCEPT_HIGHLIGHT_RGB,
        .left_eye = {
            .x = 27,
            .y = 47,
            .width = 84,
            .height = 83,
            .radius = 40,
            .iris = 48,
            .pupil = 25,
        },
        .right_eye = {
            .x = 169,
            .y = 47,
            .width = 84,
            .height = 83,
            .radius = 40,
            .iris = 48,
            .pupil = 25,
        },
        .mouth = {
            .center_x = 140,
            .smile_y = 140,
            .smile_width = 64,
            .smile_height = 34,
            .smile_stroke = 7,
            .smile_rgb = ACCENT,
            .rect_y = 140,
            .rect_width = 40,
            .rect_height = 20,
            .rect_radius = CIRCLE,
            .rect_border_width = 3,
            .rect_fill_rgb = BOBE_FACE_FEATURE_RGB,
            .rect_border_rgb = ACCENT,
        },
        .caption = {
            .x = 28,
            .y = 207,
            .width = 224,
            .font = BOBE_FACE_FONT_LARGE,
            .align = BOBE_FACE_ALIGN_CENTER,
            .rgb = BOBE_FACE_FEATURE_RGB,
        },
        .microphone = {
            .x = 21,
            .y = 135,
            .size = 12,
            .rgb = BOBE_FACE_MICROPHONE_RGB,
        },
    },
    [BOBE_FACE_THEME_SOLAR_CROWN] = {
        .background_rgb = BOBE_FACE_BACKGROUND_RGB,
        .accent_rgb = BOBE_FACE_FEATURE_RGB,
        .decor = solar_crown_decor,
        .decor_count = (uint8_t)(sizeof(solar_crown_decor) /
                                 sizeof(solar_crown_decor[0])),
        .eye_style = BOBE_FACE_EYE_FIXED,
        .eye_border_width = 3,
        .eye_fill_rgb = BOBE_FACE_EYE_WHITE_RGB,
        .eye_border_rgb = BOBE_FACE_FEATURE_RGB,
        .iris_rgb = BOBE_CONCEPT_IRIS_RGB,
        .pupil_rgb = BOBE_CONCEPT_PUPIL_RGB,
        .highlight_rgb = BOBE_CONCEPT_HIGHLIGHT_RGB,
        .left_eye = {
            .x = 41,
            .y = 61,
            .width = 84,
            .height = 65,
            .radius = 31,
            .iris = 43,
            .pupil = 22,
        },
        .right_eye = {
            .x = 139,
            .y = 55,
            .width = 99,
            .height = 73,
            .radius = 35,
            .iris = 51,
            .pupil = 27,
        },
        .mouth = {
            .center_x = 140,
            .smile_y = 142,
            .smile_width = 66,
            .smile_height = 34,
            .smile_stroke = 7,
            .smile_rgb = ACCENT,
            .rect_y = 142,
            .rect_width = 40,
            .rect_height = 20,
            .rect_radius = CIRCLE,
            .rect_border_width = 3,
            .rect_fill_rgb = BOBE_FACE_FEATURE_RGB,
            .rect_border_rgb = ACCENT,
        },
        .caption = {
            .x = 20,
            .y = 204,
            .width = 80,
            .font = BOBE_FACE_FONT_SMALL,
            .align = BOBE_FACE_ALIGN_CENTER,
            .rgb = BOBE_FACE_FEATURE_RGB,
        },
        .microphone = {
            .x = 21,
            .y = 131,
            .size = 12,
            .rgb = BOBE_FACE_MICROPHONE_RGB,
        },
    },
};

const bobe_face_theme_t *bobe_face_theme(bobe_face_theme_id_t theme)
{
    if (theme < BOBE_FACE_THEME_DEFAULT || theme >= BOBE_FACE_THEME_COUNT) {
        return &themes[BOBE_FACE_THEME_DEFAULT];
    }
    return &themes[theme];
}

typedef struct {
    int eye_dx_left;
    int eye_dx_right;
    int eye_dy;
    int eye_dw;
    int eye_dh;
    bool eyes_closed;
    bobe_face_mouth_kind_t mouth_kind;
    int mouth_dw;
    int mouth_dh;
    int mouth_dy;
    int smile_start_angle;
    int smile_end_angle;
    bool mouth_uses_border_colour;
} bobe_face_pose_t;

static int clamp_minimum(int value, int minimum)
{
    return value < minimum ? minimum : value;
}

static bobe_face_pose_t resolve_pose(
    bobe_face_expression_t expression,
    uint32_t animation_phase
)
{
    const uint32_t phase = animation_phase % 120U;
    const int breathe = phase < 60U ? (int)(phase / 20U)
                                    : (int)((119U - phase) / 20U);
    const bool blink = phase >= 92U && phase <= 96U;

    bobe_face_pose_t pose = {
        .eye_dy = breathe,
        .eyes_closed = blink,
        .mouth_kind = BOBE_FACE_MOUTH_SMILE,
        .mouth_dy = breathe,
        .smile_start_angle = 25,
        .smile_end_angle = 155,
    };

    switch (expression) {
    case BOBE_FACE_BOOTING:
    case BOBE_FACE_CONNECTING: {
        const int scan = (int)(phase / 15U) - 4;
        pose.eye_dx_left = scan;
        pose.eye_dx_right = scan;
        break;
    }
    case BOBE_FACE_UNPROVISIONED:
        pose.mouth_dw = -2;
        pose.mouth_dh = -2;
        pose.mouth_dy = 1;
        pose.smile_start_angle = 35;
        pose.smile_end_angle = 145;
        break;
    case BOBE_FACE_ATTENTIVE:
    case BOBE_FACE_LISTENING:
        pose.eye_dx_left = -2;
        pose.eye_dy = -3;
        pose.eye_dw = 2;
        pose.eye_dh = 2;
        pose.mouth_dw = 2;
        pose.mouth_dh = 1;
        pose.mouth_dy = -1;
        break;
    case BOBE_FACE_THINKING:
        pose.eye_dy = breathe - 2;
        pose.mouth_kind = BOBE_FACE_MOUTH_RECT;
        pose.mouth_dw = -25;
        pose.mouth_dh = 0;
        pose.mouth_dy = 0;
        break;
    case BOBE_FACE_SPEAKING: {
        const uint32_t mouth_phase = phase % 20U;
        const int mouth_height = mouth_phase < 10U
                                     ? 8 + (int)mouth_phase * 2
                                     : 8 + (int)(19U - mouth_phase) * 2;
        pose.mouth_kind = BOBE_FACE_MOUTH_RECT;
        pose.mouth_dw = 0;
        pose.mouth_dh = mouth_height - 15;
        pose.mouth_dy = -(mouth_height / 4);
        break;
    }
    case BOBE_FACE_HAPPY:
        pose.eye_dx_left = -2;
        pose.eye_dy = 8;
        pose.eye_dw = 2;
        pose.eye_dh = -24;
        pose.mouth_dw = 4;
        pose.mouth_dh = 2;
        pose.mouth_dy = -3;
        pose.smile_start_angle = 20;
        pose.smile_end_angle = 160;
        break;
    case BOBE_FACE_CONCERNED:
        pose.eye_dx_left = 2;
        pose.eye_dx_right = 2;
        pose.eye_dy = 3;
        pose.eye_dw = -4;
        pose.eye_dh = -7;
        pose.mouth_dw = -2;
        pose.mouth_dh = -2;
        pose.mouth_dy = 6;
        pose.smile_start_angle = 205;
        pose.smile_end_angle = 335;
        break;
    case BOBE_FACE_MUTED:
        pose.mouth_kind = BOBE_FACE_MOUTH_RECT;
        pose.mouth_dw = 2;
        pose.mouth_dh = -10;
        pose.mouth_dy = 9;
        break;
    case BOBE_FACE_OFFLINE:
        pose.eye_dx_left = 2;
        pose.eye_dx_right = 2;
        pose.eye_dy = 12;
        pose.eye_dw = -4;
        pose.eyes_closed = true;
        pose.mouth_kind = BOBE_FACE_MOUTH_RECT;
        pose.mouth_dw = 2;
        pose.mouth_dh = -9;
        pose.mouth_dy = 8;
        break;
    case BOBE_FACE_UPDATING:
        pose.eye_dy = (int)(phase % 5U) - 2;
        pose.mouth_dw = 24 + (int)(phase % 12U) - 44;
        pose.mouth_dh = -6;
        pose.mouth_dy = 2;
        pose.smile_start_angle = 30;
        pose.smile_end_angle = 150;
        break;
    case BOBE_FACE_ERROR:
        pose.eye_dx_left = 2;
        pose.eye_dx_right = 2;
        pose.eye_dy = 5;
        pose.eye_dw = -4;
        pose.eye_dh = -24;
        pose.mouth_dw = -14;
        pose.mouth_dh = -8;
        pose.mouth_dy = 9;
        pose.smile_start_angle = 205;
        pose.smile_end_angle = 335;
        pose.mouth_uses_border_colour = true;
        break;
    case BOBE_FACE_SLEEPING:
        pose.eye_dx_left = -2;
        pose.eye_dy = 12;
        pose.eye_dw = 2;
        pose.eyes_closed = true;
        pose.mouth_kind = BOBE_FACE_MOUTH_NONE;
        break;
    case BOBE_FACE_IDLE:
    default:
        break;
    }
    return pose;
}

static bobe_face_box_t centred_box(int center_x, int center_y, int size)
{
    const bobe_face_box_t box = {
        .x = (int16_t)(center_x - size / 2),
        .y = (int16_t)(center_y - size / 2),
        .width = (int16_t)size,
        .height = (int16_t)size,
        .visible = size > 0,
    };
    return box;
}

static bool box_contains(
    const bobe_face_box_t *outer,
    const bobe_face_box_t *inner
)
{
    return inner->x >= outer->x && inner->y >= outer->y &&
           inner->x + inner->width <= outer->x + outer->width &&
           inner->y + inner->height <= outer->y + outer->height;
}

static void resolve_eye(
    const bobe_face_theme_t *theme,
    const bobe_face_eye_t *base,
    const bobe_face_pose_t *pose,
    int delta_x,
    bobe_face_box_t *eye,
    bobe_face_box_t *iris,
    bobe_face_box_t *pupil,
    bobe_face_box_t *highlight
)
{
    const int width =
        clamp_minimum(base->width + pose->eye_dw, BOBE_FACE_MIN_FEATURE_SIZE);
    const int height =
        pose->eyes_closed
            ? BOBE_FACE_CLOSED_EYE_HEIGHT
            : clamp_minimum(
                  base->height + pose->eye_dh,
                  BOBE_FACE_MIN_FEATURE_SIZE
              );
    const int x = base->x + delta_x;
    const int y = base->y + pose->eye_dy;

    eye->x = (int16_t)x;
    eye->y = (int16_t)y;
    eye->width = (int16_t)width;
    eye->height = (int16_t)height;
    eye->visible = true;

    iris->visible = false;
    pupil->visible = false;
    highlight->visible = false;
    if (height < 8) {
        return;
    }

    switch (theme->eye_style) {
    case BOBE_FACE_EYE_DERIVED: {
        const int iris_size = height - 8;
        const int iris_x = x + (width - iris_size) / 2;
        const int iris_y = y + (height - iris_size) / 2;
        const int pupil_size = iris_size / 2;
        const int highlight_size = iris_size >= 12 ? 3 : 2;
        iris->x = (int16_t)iris_x;
        iris->y = (int16_t)iris_y;
        iris->width = (int16_t)iris_size;
        iris->height = (int16_t)iris_size;
        iris->visible = true;
        pupil->x = (int16_t)(iris_x + (iris_size - pupil_size) / 2);
        pupil->y = (int16_t)(iris_y + (iris_size - pupil_size) / 2);
        pupil->width = (int16_t)pupil_size;
        pupil->height = (int16_t)pupil_size;
        pupil->visible = true;
        highlight->x = (int16_t)(iris_x + 2);
        highlight->y = (int16_t)(iris_y + 2);
        highlight->width = (int16_t)highlight_size;
        highlight->height = (int16_t)highlight_size;
        highlight->visible = true;
        break;
    }
    case BOBE_FACE_EYE_FIXED: {
        const int growth = height - base->height;
        const int iris_size = clamp_minimum(
            base->iris + growth,
            BOBE_FACE_MIN_FEATURE_SIZE
        );
        const int pupil_size = clamp_minimum(
            base->pupil + growth,
            BOBE_FACE_MIN_FEATURE_SIZE
        );
        const int center_x = x + width / 2;
        const int center_y = y + height / 2;
        *iris = centred_box(center_x, center_y, iris_size);
        *pupil = centred_box(center_x, center_y, pupil_size);
        *highlight = centred_box(
            center_x - pupil_size / 3,
            center_y - pupil_size / 3,
            pupil_size / 3
        );
        break;
    }
    case BOBE_FACE_EYE_GLOSS:
        *pupil = centred_box(
            x + base->pupil_dx,
            y + base->pupil_dy,
            base->pupil
        );
        *highlight = centred_box(
            x + base->highlight_dx,
            y + base->highlight_dy,
            base->highlight
        );
        pupil->visible = pupil->visible && box_contains(eye, pupil);
        highlight->visible =
            highlight->visible && box_contains(eye, highlight);
        break;
    }
}

static uint32_t resolve_colour(uint32_t colour, uint32_t accent)
{
    return colour == BOBE_FACE_RGB_ACCENT ? accent : colour;
}

void bobe_face_layout_resolve(
    const bobe_face_theme_t *theme,
    const bobe_face_state_t *face,
    uint32_t animation_phase,
    bobe_face_layout_t *layout
)
{
    const bobe_face_pose_t pose =
        resolve_pose(face->expression, animation_phase);
    const uint32_t accent =
        face->accent_rgb == BOBE_FACE_THEME_ACCENT_RGB
            ? theme->accent_rgb
            : face->accent_rgb;

    layout->accent_rgb = accent;
    layout->status_rgb = face->expression == BOBE_FACE_ERROR
                             ? BOBE_FACE_ERROR_STATUS_RGB
                             : accent;
    layout->eyes_open = !pose.eyes_closed;
    layout->eye_border_rgb = theme->eye_border_rgb;
    layout->eye_border_width = pose.eyes_closed ? 0 : theme->eye_border_width;
    layout->eye_fill_rgb =
        pose.eyes_closed ? theme->eye_border_rgb : theme->eye_fill_rgb;

    resolve_eye(
        theme,
        &theme->left_eye,
        &pose,
        pose.eye_dx_left,
        &layout->eye[0],
        &layout->iris[0],
        &layout->pupil[0],
        &layout->highlight[0]
    );
    resolve_eye(
        theme,
        &theme->right_eye,
        &pose,
        pose.eye_dx_right,
        &layout->eye[1],
        &layout->iris[1],
        &layout->pupil[1],
        &layout->highlight[1]
    );

    layout->mouth_kind = pose.mouth_kind;
    layout->smile_start_angle = (int16_t)pose.smile_start_angle;
    layout->smile_end_angle = (int16_t)pose.smile_end_angle;
    layout->mouth.visible = pose.mouth_kind != BOBE_FACE_MOUTH_NONE;
    layout->mouth_fill_rgb =
        resolve_colour(theme->mouth.rect_fill_rgb, accent);
    if (pose.mouth_kind == BOBE_FACE_MOUTH_RECT) {
        const int width = clamp_minimum(
            theme->mouth.rect_width + pose.mouth_dw,
            BOBE_FACE_MIN_FEATURE_SIZE
        );
        const int height = clamp_minimum(
            theme->mouth.rect_height + pose.mouth_dh,
            BOBE_FACE_MIN_FEATURE_SIZE
        );
        layout->mouth.x = (int16_t)(theme->mouth.center_x - (width + 1) / 2);
        layout->mouth.y = (int16_t)(theme->mouth.rect_y + pose.mouth_dy);
        layout->mouth.width = (int16_t)width;
        layout->mouth.height = (int16_t)height;
        layout->mouth_line_rgb =
            resolve_colour(theme->mouth.rect_border_rgb, accent);
    } else {
        const int width = clamp_minimum(
            theme->mouth.smile_width + pose.mouth_dw,
            BOBE_FACE_MIN_FEATURE_SIZE
        );
        const int height = clamp_minimum(
            theme->mouth.smile_height + pose.mouth_dh,
            BOBE_FACE_MIN_FEATURE_SIZE
        );
        layout->mouth.x = (int16_t)(theme->mouth.center_x - (width + 1) / 2);
        layout->mouth.y = (int16_t)(theme->mouth.smile_y + pose.mouth_dy);
        layout->mouth.width = (int16_t)width;
        layout->mouth.height = (int16_t)height;
        layout->mouth_line_rgb =
            pose.mouth_uses_border_colour
                ? theme->eye_border_rgb
                : resolve_colour(theme->mouth.smile_rgb, accent);
    }
}

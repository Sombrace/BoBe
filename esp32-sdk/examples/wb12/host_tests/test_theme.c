#include "bobe_face_theme.h"
#include "bobe_device_config.h"
#include "bobe_protocol.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(condition)                                                     \
    do {                                                                     \
        if (!(condition)) {                                                  \
            fprintf(stderr, "check failed at %s:%d: %s\n",                 \
                    __FILE__, __LINE__, #condition);                         \
            exit(1);                                                         \
        }                                                                    \
    } while (0)

static const bobe_face_expression_t expressions[] = {
    BOBE_FACE_BOOTING,
    BOBE_FACE_UNPROVISIONED,
    BOBE_FACE_CONNECTING,
    BOBE_FACE_IDLE,
    BOBE_FACE_ATTENTIVE,
    BOBE_FACE_LISTENING,
    BOBE_FACE_THINKING,
    BOBE_FACE_SPEAKING,
    BOBE_FACE_HAPPY,
    BOBE_FACE_CONCERNED,
    BOBE_FACE_MUTED,
    BOBE_FACE_OFFLINE,
    BOBE_FACE_UPDATING,
    BOBE_FACE_ERROR,
    BOBE_FACE_SLEEPING,
};

static bobe_face_state_t make_face(bobe_face_expression_t expression)
{
    bobe_face_state_t face = {
        .expression = expression,
        .accent_rgb = BOBE_FACE_DEFAULT_ACCENT_RGB,
    };
    return face;
}

static void test_theme_names(void)
{
    static const char *const wire_names[] = {
        "warm-aperture",
        "warm-lintel",
        "warm-ledge",
        "kettle-tall-bright",
        "kettle-soft-bowl",
        "solar-wide-awake",
        "solar-crown",
    };
    CHECK(
        (int)BOBE_FACE_THEME_COUNT ==
        (int)(sizeof(wire_names) / sizeof(wire_names[0])) + 1
    );
    CHECK(bobe_face_theme_name(BOBE_FACE_THEME_DEFAULT) == NULL);
    CHECK(bobe_face_theme_name(BOBE_FACE_THEME_COUNT) == NULL);

    for (size_t index = 0; index < sizeof(wire_names) / sizeof(wire_names[0]);
         ++index) {
        const bobe_face_theme_id_t id =
            (bobe_face_theme_id_t)(BOBE_FACE_THEME_WARM_APERTURE + index);
        const char *const name = bobe_face_theme_name(id);
        CHECK(name != NULL);
        CHECK(strcmp(name, wire_names[index]) == 0);
        CHECK(strlen(name) < BOBE_FACE_THEME_ID_MAX_BYTES);

        bobe_face_theme_id_t parsed = BOBE_FACE_THEME_DEFAULT;
        CHECK(bobe_face_theme_parse(name, &parsed));
        CHECK(parsed == id);
    }

    bobe_face_theme_id_t parsed = BOBE_FACE_THEME_DEFAULT;
    CHECK(!bobe_face_theme_parse("default", &parsed));
    CHECK(!bobe_face_theme_parse("", &parsed));
    CHECK(!bobe_face_theme_parse("warm-aperture ", &parsed));
    CHECK(!bobe_face_theme_parse(NULL, &parsed));
    CHECK(!bobe_face_theme_parse("warm-aperture", NULL));
}

static void test_theme_descriptors(void)
{
    CHECK(bobe_face_theme((bobe_face_theme_id_t)-1) ==
          bobe_face_theme(BOBE_FACE_THEME_DEFAULT));
    CHECK(bobe_face_theme(BOBE_FACE_THEME_COUNT) ==
          bobe_face_theme(BOBE_FACE_THEME_DEFAULT));

    for (int id = BOBE_FACE_THEME_DEFAULT; id < BOBE_FACE_THEME_COUNT; ++id) {
        const bobe_face_theme_t *const theme =
            bobe_face_theme((bobe_face_theme_id_t)id);
        CHECK(theme != NULL);
        CHECK(theme->decor_count <= BOBE_FACE_MAX_DECOR);
        CHECK(theme->arc_count <= BOBE_FACE_MAX_DECOR_ARCS);
        CHECK(theme->decor_count == 0 || theme->decor != NULL);
        CHECK(theme->arc_count == 0 || theme->arcs != NULL);
        CHECK(theme->background_rgb <= UINT32_C(0x00FFFFFF));
        CHECK(theme->accent_rgb <= UINT32_C(0x00FFFFFF));
        CHECK(theme->left_eye.width > 0 && theme->left_eye.height > 0);
        CHECK(theme->right_eye.width > 0 && theme->right_eye.height > 0);
        CHECK(theme->mouth.smile_width > 0 && theme->mouth.smile_height > 0);
        CHECK(theme->mouth.rect_width > 0 && theme->mouth.rect_height > 0);
        CHECK(theme->caption.width > 0);
        CHECK(theme->microphone.size > 0);
        CHECK(theme->caption.y + 24 <= (int16_t)BOBE_DISPLAY_HEIGHT);
        CHECK(theme->caption.x >= 0);
        CHECK(
            theme->caption.x + theme->caption.width <=
            (int16_t)BOBE_DISPLAY_WIDTH
        );
    }
}

static void test_default_theme_matches_production(void)
{
    const bobe_face_theme_t *const theme =
        bobe_face_theme(BOBE_FACE_THEME_DEFAULT);
    CHECK(theme->background_rgb == UINT32_C(0xFAF7F2));
    CHECK(theme->accent_rgb == BOBE_FACE_DEFAULT_ACCENT_RGB);
    CHECK(theme->eye_style == BOBE_FACE_EYE_DERIVED);
    CHECK(theme->eye_border_width == 2);
    CHECK(theme->decor_count == 2);
    CHECK(theme->decor[0].x == 48 && theme->decor[0].y == 26);
    CHECK(theme->decor[0].width == 184 && theme->decor[0].height == 184);
    CHECK(theme->decor[0].border_rgb == BOBE_FACE_RGB_ACCENT);
    CHECK(theme->decor[0].border_width == 8);
    CHECK(theme->decor[1].x == 65 && theme->decor[1].y == 48);
    CHECK(theme->decor[1].width == 150 && theme->decor[1].height == 150);
    CHECK(theme->microphone.x == 91 && theme->microphone.y == 12);
    CHECK(theme->microphone.size == 9);

    bobe_face_layout_t layout;
    const bobe_face_state_t idle = make_face(BOBE_FACE_IDLE);
    bobe_face_layout_resolve(theme, &idle, 0, &layout);
    CHECK(layout.eye[0].x == 94 && layout.eye[0].y == 99);
    CHECK(layout.eye[0].width == 40 && layout.eye[0].height == 34);
    CHECK(layout.eye[1].x == 146 && layout.eye[1].y == 99);
    CHECK(layout.eye_border_width == 2);
    CHECK(layout.eye_fill_rgb == UINT32_C(0xFFFDFC));
    CHECK(layout.eye_border_rgb == UINT32_C(0x3A3A3A));
    CHECK(layout.iris[0].width == 26 && layout.iris[0].height == 26);
    CHECK(layout.iris[0].x == 94 + (40 - 26) / 2);
    CHECK(layout.iris[0].y == 99 + (34 - 26) / 2);
    CHECK(layout.pupil[0].width == 13);
    CHECK(layout.highlight[0].width == 3);
    CHECK(layout.highlight[0].x == layout.iris[0].x + 2);
    CHECK(layout.mouth_kind == BOBE_FACE_MOUTH_SMILE);
    CHECK(layout.mouth.width == 44 && layout.mouth.height == 22);
    CHECK(layout.mouth.x == (int16_t)((BOBE_DISPLAY_WIDTH - 44U) / 2U));
    CHECK(layout.mouth.y == 136);
    CHECK(layout.smile_start_angle == 25 && layout.smile_end_angle == 155);
    CHECK(layout.mouth_line_rgb == BOBE_FACE_DEFAULT_ACCENT_RGB);
    CHECK(layout.status_rgb == BOBE_FACE_DEFAULT_ACCENT_RGB);

    bobe_face_layout_resolve(theme, &idle, 94, &layout);
    CHECK(!layout.eyes_open);
    CHECK(layout.eye[0].height == 5);
    CHECK(layout.eye_border_width == 0);
    CHECK(layout.eye_fill_rgb == UINT32_C(0x3A3A3A));
    CHECK(!layout.iris[0].visible);
    CHECK(!layout.pupil[0].visible);
    CHECK(!layout.highlight[0].visible);

    const bobe_face_state_t speaking = make_face(BOBE_FACE_SPEAKING);
    for (uint32_t phase = 0; phase < 120U; ++phase) {
        bobe_face_layout_resolve(theme, &speaking, phase, &layout);
        const uint32_t mouth_phase = (phase % 120U) % 20U;
        const int expected_height = mouth_phase < 10U
                                        ? 8 + (int)mouth_phase * 2
                                        : 8 + (int)(19U - mouth_phase) * 2;
        CHECK(layout.mouth_kind == BOBE_FACE_MOUTH_RECT);
        CHECK(layout.mouth.width == 40);
        CHECK(layout.mouth.height == expected_height);
        CHECK(layout.mouth.y == 139 - expected_height / 4);
        CHECK(layout.mouth.x == (int16_t)((BOBE_DISPLAY_WIDTH - 40U) / 2U));
        CHECK(layout.mouth_fill_rgb == UINT32_C(0x3A3A3A));
        CHECK(layout.mouth_line_rgb == BOBE_FACE_DEFAULT_ACCENT_RGB);
    }

    const bobe_face_state_t updating = make_face(BOBE_FACE_UPDATING);
    for (uint32_t phase = 0; phase < 120U; ++phase) {
        bobe_face_layout_resolve(theme, &updating, phase, &layout);
        const int width = 24 + (int)(phase % 12U);
        CHECK(layout.mouth.width == width);
        CHECK(
            layout.mouth.x ==
            (int16_t)((BOBE_DISPLAY_WIDTH - (uint32_t)width) / 2U)
        );
        CHECK(layout.mouth.y == 138);
        CHECK(layout.eye[0].y == 97 + (int)(phase % 5U));
    }

    const bobe_face_state_t sleeping = make_face(BOBE_FACE_SLEEPING);
    bobe_face_layout_resolve(theme, &sleeping, 0, &layout);
    CHECK(layout.mouth_kind == BOBE_FACE_MOUTH_NONE);
    CHECK(!layout.mouth.visible);
    CHECK(layout.eye[0].x == 92 && layout.eye[0].y == 111);
    CHECK(layout.eye[0].width == 42 && layout.eye[0].height == 5);

    const bobe_face_state_t error = make_face(BOBE_FACE_ERROR);
    bobe_face_layout_resolve(theme, &error, 0, &layout);
    CHECK(layout.status_rgb == UINT32_C(0xFF5A6F));
    CHECK(layout.mouth_line_rgb == UINT32_C(0x3A3A3A));
    CHECK(layout.smile_start_angle == 205 && layout.smile_end_angle == 335);

    bobe_face_state_t accented = make_face(BOBE_FACE_IDLE);
    accented.accent_rgb = UINT32_C(0x00FF00);
    bobe_face_layout_resolve(theme, &accented, 0, &layout);
    CHECK(layout.accent_rgb == UINT32_C(0x00FF00));
    CHECK(layout.mouth_line_rgb == UINT32_C(0x00FF00));
}

static void test_layout_invariants(void)
{
    for (int id = BOBE_FACE_THEME_DEFAULT; id < BOBE_FACE_THEME_COUNT; ++id) {
        const bobe_face_theme_t *const theme =
            bobe_face_theme((bobe_face_theme_id_t)id);
        for (size_t index = 0;
             index < sizeof(expressions) / sizeof(expressions[0]);
             ++index) {
            const bobe_face_state_t face = make_face(expressions[index]);
            for (uint32_t phase = 0; phase < 120U; ++phase) {
                bobe_face_layout_t layout;
                bobe_face_layout_resolve(theme, &face, phase, &layout);
                CHECK(layout.accent_rgb <= UINT32_C(0x00FFFFFF));
                CHECK(layout.status_rgb <= UINT32_C(0x00FFFFFF));
                CHECK(layout.eye_fill_rgb <= UINT32_C(0x00FFFFFF));
                CHECK(layout.eye_border_rgb <= UINT32_C(0x00FFFFFF));
                CHECK(layout.mouth_fill_rgb <= UINT32_C(0x00FFFFFF));
                CHECK(layout.mouth_line_rgb <= UINT32_C(0x00FFFFFF));
                for (size_t side = 0; side < 2U; ++side) {
                    CHECK(layout.eye[side].visible);
                    CHECK(layout.eye[side].width >= 2);
                    CHECK(layout.eye[side].height >= 2);
                    CHECK(layout.eye[side].x >= 0);
                    CHECK(layout.eye[side].y >= 0);
                    CHECK(
                        layout.eye[side].x + layout.eye[side].width <=
                        (int16_t)BOBE_DISPLAY_WIDTH
                    );
                    CHECK(
                        layout.eye[side].y + layout.eye[side].height <=
                        (int16_t)BOBE_DISPLAY_HEIGHT
                    );
                    if (!layout.eyes_open) {
                        CHECK(!layout.iris[side].visible);
                        CHECK(!layout.pupil[side].visible);
                        CHECK(!layout.highlight[side].visible);
                    }
                    if (layout.iris[side].visible) {
                        CHECK(layout.iris[side].width >= 2);
                    }
                }
                CHECK(layout.mouth.width >= 2 && layout.mouth.height >= 2);
                CHECK(layout.mouth.x >= 0 && layout.mouth.y >= 0);
                CHECK(
                    layout.mouth.x + layout.mouth.width <=
                    (int16_t)BOBE_DISPLAY_WIDTH
                );
                CHECK(
                    layout.mouth.y + layout.mouth.height <=
                    (int16_t)BOBE_DISPLAY_HEIGHT
                );
                CHECK(
                    layout.smile_start_angle >= 0 &&
                    layout.smile_start_angle < 360
                );
                CHECK(
                    layout.smile_end_angle >= 0 && layout.smile_end_angle < 360
                );
            }
        }
    }
}

static void test_layout_is_deterministic(void)
{
    const bobe_face_state_t face = make_face(BOBE_FACE_SPEAKING);
    for (int id = BOBE_FACE_THEME_DEFAULT; id < BOBE_FACE_THEME_COUNT; ++id) {
        const bobe_face_theme_t *const theme =
            bobe_face_theme((bobe_face_theme_id_t)id);
        bobe_face_layout_t first;
        bobe_face_layout_t second;
        memset(&first, 0, sizeof(first));
        memset(&second, 0, sizeof(second));
        bobe_face_layout_resolve(theme, &face, 37, &first);
        bobe_face_layout_resolve(theme, &face, 37 + 120U, &second);
        CHECK(memcmp(&first, &second, sizeof(first)) == 0);
    }
}

int main(void)
{
    test_theme_names();
    test_theme_descriptors();
    test_default_theme_matches_production();
    test_layout_invariants();
    test_layout_is_deterministic();
    puts("BoBe face theme tests passed.");
    return 0;
}

#include "bobe_face_renderer.h"
#include "bobe_device_config.h"

#include "lvgl.h"
#include "src/debugging/test/lv_test_display.h"

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if LVGL_VERSION_MAJOR != 9 || LVGL_VERSION_MINOR != 5 || \
    LVGL_VERSION_PATCH != 0
#error "The UI references require the pinned LVGL 9.5.0"
#endif

typedef struct {
    const char *name;
    bobe_face_expression_t expression;
    bool microphone_active;
    uint32_t phase;
    uint32_t tick_ms;
    int32_t scroll_position;
    uint32_t accent_rgb;
    const char *caption;
    bobe_notice_kind_t notice_kind;
    const char *notice_text;
} visual_case_t;

static uint32_t host_tick_ms;

static uint32_t host_tick(void)
{
    return host_tick_ms;
}

static const visual_case_t visual_cases[] = {
    {"expression-booting", BOBE_FACE_BOOTING, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"expression-unprovisioned", BOBE_FACE_UNPROVISIONED, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "Open BoBe to set up", BOBE_NOTICE_NONE, ""},
    {"expression-connecting", BOBE_FACE_CONNECTING, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "Connecting to BoBe", BOBE_NOTICE_NONE, ""},
    {"expression-idle", BOBE_FACE_IDLE, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"expression-attentive", BOBE_FACE_ATTENTIVE, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"expression-listening", BOBE_FACE_LISTENING, true, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"expression-thinking", BOBE_FACE_THINKING, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"expression-speaking", BOBE_FACE_SPEAKING, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"expression-happy", BOBE_FACE_HAPPY, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"expression-concerned", BOBE_FACE_CONCERNED, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"expression-muted", BOBE_FACE_MUTED, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"expression-offline", BOBE_FACE_OFFLINE, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"expression-updating", BOBE_FACE_UPDATING, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"expression-error", BOBE_FACE_ERROR, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"expression-sleeping", BOBE_FACE_SLEEPING, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"microphone-on", BOBE_FACE_IDLE, true, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"custom-accent-caption", BOBE_FACE_HAPPY, false, 0, 0, 0, 0x1976D2,
     "Custom hello", BOBE_NOTICE_NONE, ""},
    {"phase-breathe-maximum", BOBE_FACE_IDLE, false, 40, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"phase-opacity-return", BOBE_FACE_IDLE, false, 60, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"phase-blink", BOBE_FACE_IDLE, false, 94, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"phase-speaking-maximum", BOBE_FACE_SPEAKING, false, 9, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"phase-speaking-minimum", BOBE_FACE_SPEAKING, false, 19, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"phase-connecting-scan-end", BOBE_FACE_CONNECTING, false, 119, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"phase-updating-extreme", BOBE_FACE_UPDATING, false, 11, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "", BOBE_NOTICE_NONE, ""},
    {"long-caption-baseline", BOBE_FACE_ATTENTIVE, false, 0, 1000, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB,
     "A deliberately long caption verifies the circular caption baseline "
     "without relying on wall-clock time", BOBE_NOTICE_NONE, ""},
    {"long-caption-scrolled", BOBE_FACE_ATTENTIVE, false, 0, 4000, -64,
     BOBE_FACE_DEFAULT_ACCENT_RGB,
     "A deliberately long caption verifies the circular caption baseline "
     "without relying on wall-clock time", BOBE_NOTICE_NONE, ""},
    {"notice-volume", BOBE_FACE_IDLE, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "BoBe",
     BOBE_NOTICE_VOLUME, "Volume 55%"},
    {"notice-silent", BOBE_FACE_IDLE, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "BoBe",
     BOBE_NOTICE_SILENT, "Sound off"},
    {"notice-hint-unprovisioned", BOBE_FACE_UNPROVISIONED, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "Ready to set up",
     BOBE_NOTICE_HINT, "Hold +/-: setup"},
    {"notice-hint-online", BOBE_FACE_IDLE, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "BoBe",
     BOBE_NOTICE_HINT, "Hold O: talk"},
    {"notice-hint-unmute", BOBE_FACE_MUTED, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "Microphone muted",
     BOBE_NOTICE_HINT, "Hold -: mic"},
    {"notice-update", BOBE_FACE_IDLE, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "BoBe",
     BOBE_NOTICE_UPDATE, "Update installed"},
    {"notice-expired-restores-caption", BOBE_FACE_MUTED, false, 0, 0, 0,
     BOBE_FACE_DEFAULT_ACCENT_RGB, "Microphone muted",
     BOBE_NOTICE_NONE, ""},
};

typedef struct {
    const char *name;
    bobe_face_theme_id_t theme;
    bobe_face_expression_t expression;
    bool microphone_active;
    uint32_t phase;
    const char *caption;
} theme_case_t;

static const theme_case_t theme_cases[] = {
    {"theme-warm-aperture", BOBE_FACE_THEME_WARM_APERTURE,
     BOBE_FACE_IDLE, false, 0, "Warm aperture"},
    {"theme-warm-lintel", BOBE_FACE_THEME_WARM_LINTEL,
     BOBE_FACE_LISTENING, true, 0, "Warm lintel"},
    {"theme-warm-ledge", BOBE_FACE_THEME_WARM_LEDGE,
     BOBE_FACE_HAPPY, false, 0, "Warm ledge"},
    {"theme-kettle-tall-bright", BOBE_FACE_THEME_KETTLE_TALL_BRIGHT,
     BOBE_FACE_SPEAKING, false, 9, "Kettle tall bright"},
    {"theme-kettle-soft-bowl", BOBE_FACE_THEME_KETTLE_SOFT_BOWL,
     BOBE_FACE_THINKING, false, 0, "Kettle soft bowl"},
    {"theme-solar-wide-awake", BOBE_FACE_THEME_SOLAR_WIDE_AWAKE,
     BOBE_FACE_ATTENTIVE, false, 0, "Solar wide awake"},
    {"theme-solar-crown", BOBE_FACE_THEME_SOLAR_CROWN,
     BOBE_FACE_IDLE, false, 0, "Solar crown"},
};

static void fail(const char *message, const char *detail)
{
    fprintf(stderr, "%s: %s\n", message, detail);
    exit(EXIT_FAILURE);
}

static void write_rgb565(
    lv_display_t *display,
    const char *output_dir,
    const char *case_name
)
{
    char path[1024];
    const int length = snprintf(
        path,
        sizeof(path),
        "%s/%s.rgb565",
        output_dir,
        case_name
    );
    if (length < 0 || (size_t)length >= sizeof(path)) {
        fail("Output path is too long", case_name);
    }

    FILE *const output = fopen(path, "wb");
    if (output == NULL) {
        fail("Cannot create RGB565 output", strerror(errno));
    }

    const lv_draw_buf_t *const draw_buffer =
        lv_display_get_buf_active(display);
    if (draw_buffer == NULL ||
        draw_buffer->header.cf != LV_COLOR_FORMAT_RGB565 ||
        draw_buffer->header.w != BOBE_DISPLAY_WIDTH ||
        draw_buffer->header.h != BOBE_DISPLAY_HEIGHT) {
        fclose(output);
        fail("Unexpected native display buffer", case_name);
    }

    for (uint32_t y = 0; y < BOBE_DISPLAY_HEIGHT; ++y) {
        const uint8_t *const row =
            draw_buffer->data + y * draw_buffer->header.stride;
        for (uint32_t x = 0; x < BOBE_DISPLAY_WIDTH; ++x) {
            uint16_t pixel;
            memcpy(&pixel, row + x * sizeof(pixel), sizeof(pixel));
            /* Hash numeric RGB565 in a host-independent byte order. */
            if (fputc((int)(pixel >> 8), output) == EOF ||
                fputc((int)(pixel & 0xFFU), output) == EOF) {
                fclose(output);
                fail("Cannot write RGB565 output", strerror(errno));
            }
        }
    }
    if (fclose(output) != 0) {
        fail("Cannot close RGB565 output", strerror(errno));
    }
}

static bobe_face_render_input_t make_input(const visual_case_t *test_case)
{
    bobe_face_render_input_t input = {
        .face = {
            .expression = test_case->expression,
            .accent_rgb = test_case->accent_rgb,
        },
        .microphone_active = test_case->microphone_active,
        .face_changed = true,
        .notice = {
            .kind = test_case->notice_kind,
        },
        .status = {
            .battery_percent = 78,
            .wifi_rssi_dbm = -58,
            .battery_valid = true,
            .wifi_connected = true,
            .microphone_muted =
                test_case->expression == BOBE_FACE_MUTED,
        },
        .animation_phase = test_case->phase,
        .lvgl_tick_ms = test_case->tick_ms,
        .caption_scroll_position = test_case->scroll_position,
    };
    if (test_case->expression == BOBE_FACE_CONCERNED) {
        input.status.battery_percent = 9;
        input.status.wifi_rssi_dbm = -82;
    } else if (test_case->expression == BOBE_FACE_HAPPY) {
        input.status.battery_percent = 52;
        input.status.charging = true;
    }
    if (test_case->notice_text != NULL) {
        const int notice_result = snprintf(
            input.notice.text,
            sizeof(input.notice.text),
            "%s",
            test_case->notice_text
        );
        if (notice_result < 0 ||
            (size_t)notice_result >= sizeof(input.notice.text)) {
            fail("Test notice is too long", test_case->name);
        }
    }
    const int result = snprintf(
        input.face.caption,
        sizeof(input.face.caption),
        "%s",
        test_case->caption
    );
    if (result < 0 || (size_t)result >= sizeof(input.face.caption)) {
        fail("Test caption is too long", test_case->name);
    }
    return input;
}

static bobe_face_render_input_t make_theme_input(
    const theme_case_t *test_case
)
{
    bobe_face_render_input_t input = {
        .face = {
            .expression = test_case->expression,
            .accent_rgb = BOBE_FACE_THEME_ACCENT_RGB,
        },
        .theme = test_case->theme,
        .microphone_active = test_case->microphone_active,
        .status = {
            .battery_percent = 78,
            .wifi_rssi_dbm = -58,
            .battery_valid = true,
            .wifi_connected = true,
        },
        .face_changed = true,
        .theme_changed = true,
        .animation_phase = test_case->phase,
        .lvgl_tick_ms = 0,
        .caption_scroll_position = 0,
    };
    const int result = snprintf(
        input.face.caption,
        sizeof(input.face.caption),
        "%s",
        test_case->caption
    );
    if (result < 0 || (size_t)result >= sizeof(input.face.caption)) {
        fail("Test caption is too long", test_case->name);
    }
    return input;
}

int main(int argc, char **argv)
{
    if (argc != 2) {
        fail("Usage", "bobe_ui_visual_tests OUTPUT_DIRECTORY");
    }

    lv_init();
    lv_tick_set_cb(host_tick);
    lv_display_t *const display = lv_test_display_create(
        BOBE_DISPLAY_WIDTH,
        BOBE_DISPLAY_HEIGHT
    );
    if (display == NULL) {
        fail("Cannot create LVGL native test display", "");
    }
    lv_display_set_color_format(display, LV_COLOR_FORMAT_RGB565);
    lv_display_set_default(display);

    bobe_face_render_input_t initial = make_input(&visual_cases[0]);
    bobe_face_renderer_t renderer;
    bobe_face_renderer_create(
        &renderer,
        lv_screen_active(),
        &initial
    );

    const size_t case_count =
        sizeof(visual_cases) / sizeof(visual_cases[0]);
    for (size_t index = 0; index < case_count; ++index) {
        bobe_face_render_input_t input = make_input(&visual_cases[index]);
        host_tick_ms = input.lvgl_tick_ms;
        bobe_face_renderer_render(&renderer, &input);
        lv_obj_invalidate(renderer.screen);
        lv_refr_now(display);
        write_rgb565(display, argv[1], visual_cases[index].name);
        puts(visual_cases[index].name);
    }

    const size_t theme_count =
        sizeof(theme_cases) / sizeof(theme_cases[0]);
    for (size_t index = 0; index < theme_count; ++index) {
        bobe_face_render_input_t input =
            make_theme_input(&theme_cases[index]);
        host_tick_ms = input.lvgl_tick_ms;
        bobe_face_renderer_render(&renderer, &input);
        lv_obj_invalidate(renderer.screen);
        lv_refr_now(display);
        write_rgb565(display, argv[1], theme_cases[index].name);
        puts(theme_cases[index].name);
    }

    lv_deinit();
    return EXIT_SUCCESS;
}

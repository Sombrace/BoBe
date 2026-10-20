#include "bobe_device_config.h"
#include "bobe_face_renderer.h"

#include <stdio.h>
#include <string.h>

_Static_assert(
    BOBE_DISPLAY_WIDTH >= 240U,
    "face layout must fit the landscape display"
);

static lv_color_t face_colour(uint32_t rgb)
{
    return lv_color_hex(rgb & UINT32_C(0x00FFFFFF));
}

static int32_t face_radius(int16_t radius)
{
    return radius == BOBE_FACE_RADIUS_CIRCLE ? LV_RADIUS_CIRCLE
                                             : (int32_t)radius;
}

static const lv_font_t *face_font(bobe_face_font_t font)
{
    return font == BOBE_FACE_FONT_LARGE ? &lv_font_montserrat_18
                                        : &lv_font_montserrat_14;
}

static lv_text_align_t face_text_align(bobe_face_align_t align)
{
    return align == BOBE_FACE_ALIGN_LEFT ? LV_TEXT_ALIGN_LEFT
                                         : LV_TEXT_ALIGN_CENTER;
}

static void configure_shape(lv_obj_t *object, lv_color_t color)
{
    lv_obj_remove_style_all(object);
    lv_obj_set_style_bg_opa(object, LV_OPA_COVER, 0);
    lv_obj_set_style_bg_color(object, color, 0);
    lv_obj_set_style_radius(object, LV_RADIUS_CIRCLE, 0);
    lv_obj_set_clickable(object, false);
}

static void configure_arc(lv_obj_t *object, lv_color_t color, int32_t width)
{
    lv_obj_remove_style_all(object);
    lv_obj_set_style_arc_color(object, color, LV_PART_MAIN);
    lv_obj_set_style_arc_width(object, width, LV_PART_MAIN);
    lv_obj_set_style_arc_rounded(object, true, LV_PART_MAIN);
    lv_obj_set_style_arc_opa(object, LV_OPA_COVER, LV_PART_MAIN);
    lv_obj_set_style_arc_opa(object, LV_OPA_TRANSP, LV_PART_INDICATOR);
    lv_obj_set_style_bg_opa(object, LV_OPA_TRANSP, LV_PART_KNOB);
    lv_obj_set_clickable(object, false);
}

static void apply_box(lv_obj_t *object, const bobe_face_box_t *box)
{
    lv_obj_set_hidden(object, !box->visible);
    if (!box->visible) {
        return;
    }
    lv_obj_set_size(object, box->width, box->height);
    lv_obj_set_pos(object, box->x, box->y);
}

static uint32_t resolve_rgb(uint32_t rgb, uint32_t accent)
{
    return rgb == BOBE_FACE_RGB_ACCENT ? accent : rgb;
}

static void apply_decor(
    lv_obj_t *object,
    const bobe_face_decor_t *decor,
    uint32_t accent
)
{
    lv_obj_set_hidden(object, false);
    lv_obj_set_size(object, decor->width, decor->height);
    lv_obj_set_pos(object, decor->x, decor->y);
    lv_obj_set_style_radius(object, face_radius(decor->radius), 0);
    if (decor->fill_rgb == BOBE_FACE_RGB_NONE) {
        lv_obj_set_style_bg_opa(object, LV_OPA_TRANSP, 0);
    } else {
        lv_obj_set_style_bg_opa(object, LV_OPA_COVER, 0);
        lv_obj_set_style_bg_color(
            object,
            face_colour(resolve_rgb(decor->fill_rgb, accent)),
            0
        );
    }
    if (decor->border_width == 0 || decor->border_rgb == BOBE_FACE_RGB_NONE) {
        lv_obj_set_style_border_width(object, 0, 0);
        return;
    }
    lv_obj_set_style_border_opa(object, LV_OPA_COVER, 0);
    lv_obj_set_style_border_color(
        object,
        face_colour(resolve_rgb(decor->border_rgb, accent)),
        0
    );
    lv_obj_set_style_border_width(object, decor->border_width, 0);
}

static void apply_accent(bobe_face_renderer_t *renderer, uint32_t accent)
{
    const bobe_face_theme_t *const theme = renderer->theme;
    for (uint8_t index = 0; index < theme->decor_count; ++index) {
        const bobe_face_decor_t *const decor = &theme->decor[index];
        if (decor->fill_rgb == BOBE_FACE_RGB_ACCENT) {
            lv_obj_set_style_bg_color(
                renderer->decor[index],
                face_colour(accent),
                0
            );
        }
        if (decor->border_rgb == BOBE_FACE_RGB_ACCENT &&
            decor->border_width > 0) {
            lv_obj_set_style_border_color(
                renderer->decor[index],
                face_colour(accent),
                0
            );
        }
    }
    for (uint8_t index = 0; index < theme->arc_count; ++index) {
        if (theme->arcs[index].rgb == BOBE_FACE_RGB_ACCENT) {
            lv_obj_set_style_arc_color(
                renderer->decor_arcs[index],
                face_colour(accent),
                LV_PART_MAIN
            );
        }
    }
}

static void apply_local_status(
    bobe_face_renderer_t *renderer,
    const bobe_face_render_input_t *input,
    uint32_t accent
)
{
    const bobe_display_status_t *const status = &input->status;
    const uint8_t bars = !status->wifi_connected
                             ? 0U
                             : status->wifi_rssi_dbm >= -55 ? 3U
                             : status->wifi_rssi_dbm >= -70 ? 2U : 1U;
    for (uint8_t index = 0; index < 3U; ++index) {
        lv_obj_set_style_bg_color(
            renderer->wifi_bar[index],
            face_colour(index < bars ? accent : UINT32_C(0xD4D0CC)),
            0
        );
    }
    const uint8_t battery = status->battery_percent > 100U
                                ? 100U : status->battery_percent;
    lv_obj_set_width(renderer->battery_fill, 2 + (int32_t)battery * 14 / 100);
    const uint32_t battery_rgb = status->charging
                                     ? UINT32_C(0x43A047)
                                     : battery <= 15U
                                           ? UINT32_C(0xFF5A6F) : accent;
    lv_obj_set_style_bg_color(
        renderer->battery_fill,
        face_colour(battery_rgb),
        0
    );
    lv_obj_set_hidden(renderer->battery_body, !status->battery_valid);
    lv_obj_set_hidden(renderer->battery_fill, !status->battery_valid);
    lv_obj_set_hidden(renderer->battery_tip, !status->battery_valid);
    renderer->applied_status = *status;
    renderer->status_initialized = true;
}

static void apply_caption_style(bobe_face_renderer_t *renderer)
{
    const bobe_face_label_t *const caption = &renderer->theme->caption;
    lv_obj_set_width(renderer->caption, caption->width);
    lv_obj_set_pos(renderer->caption, caption->x, caption->y);
    lv_obj_set_style_text_align(
        renderer->caption,
        face_text_align(caption->align),
        0
    );
    lv_obj_set_style_text_color(
        renderer->caption,
        face_colour(caption->rgb),
        0
    );
    lv_obj_set_style_text_font(
        renderer->caption,
        face_font(caption->font),
        0
    );
}

static void apply_theme(bobe_face_renderer_t *renderer, uint32_t accent)
{
    const bobe_face_theme_t *const theme = renderer->theme;

    lv_obj_set_style_bg_color(
        renderer->screen,
        face_colour(theme->background_rgb),
        0
    );

    for (uint8_t index = 0; index < BOBE_FACE_MAX_DECOR; ++index) {
        if (index < theme->decor_count) {
            apply_decor(renderer->decor[index], &theme->decor[index], accent);
        } else {
            lv_obj_set_hidden(renderer->decor[index], true);
        }
    }
    for (uint8_t index = 0; index < BOBE_FACE_MAX_DECOR_ARCS; ++index) {
        lv_obj_t *const object = renderer->decor_arcs[index];
        if (index >= theme->arc_count) {
            lv_obj_set_hidden(object, true);
            continue;
        }
        const bobe_face_decor_arc_t *const arc = &theme->arcs[index];
        lv_obj_set_hidden(object, false);
        lv_obj_set_size(object, arc->width, arc->height);
        lv_obj_set_pos(object, arc->x, arc->y);
        lv_arc_set_bg_angles(object, arc->start_angle, arc->end_angle);
        lv_obj_set_style_arc_color(
            object,
            face_colour(resolve_rgb(arc->rgb, accent)),
            LV_PART_MAIN
        );
        lv_obj_set_style_arc_width(object, arc->stroke, LV_PART_MAIN);
    }

    const bobe_face_eye_t *const eyes[2] = {
        &theme->left_eye,
        &theme->right_eye,
    };
    for (size_t side = 0; side < 2U; ++side) {
        lv_obj_set_style_radius(
            renderer->eye[side],
            face_radius(eyes[side]->radius),
            0
        );
        lv_obj_set_style_bg_color(
            renderer->iris[side],
            face_colour(theme->iris_rgb),
            0
        );
        lv_obj_set_style_bg_color(
            renderer->pupil[side],
            face_colour(theme->pupil_rgb),
            0
        );
        lv_obj_set_style_bg_color(
            renderer->highlight[side],
            face_colour(theme->highlight_rgb),
            0
        );
    }

    lv_obj_set_style_radius(
        renderer->mouth,
        face_radius(theme->mouth.rect_radius),
        0
    );
    lv_obj_set_style_arc_width(
        renderer->smile,
        theme->mouth.smile_stroke,
        LV_PART_MAIN
    );

    lv_obj_set_size(
        renderer->microphone_indicator,
        theme->microphone.size,
        theme->microphone.size
    );
    lv_obj_set_pos(
        renderer->microphone_indicator,
        theme->microphone.x,
        theme->microphone.y
    );
    lv_obj_set_style_bg_color(
        renderer->microphone_indicator,
        face_colour(theme->microphone.rgb),
        0
    );

    apply_caption_style(renderer);
}

static const char *expression_status(bobe_face_expression_t expression)
{
    switch (expression) {
    case BOBE_FACE_UNPROVISIONED:
        return "Set up";
    case BOBE_FACE_CONNECTING:
        return "";
    case BOBE_FACE_LISTENING:
        return "Listening";
    case BOBE_FACE_THINKING:
        return "Thinking";
    case BOBE_FACE_SPEAKING:
        return "Speaking";
    case BOBE_FACE_MUTED:
        return "Muted";
    case BOBE_FACE_OFFLINE:
        return "Offline";
    case BOBE_FACE_UPDATING:
        return "Updating";
    case BOBE_FACE_ERROR:
        return "Needs help";
    case BOBE_FACE_SLEEPING:
        return "Sleeping";
    case BOBE_FACE_BOOTING:
        return "Starting";
    case BOBE_FACE_IDLE:
    case BOBE_FACE_ATTENTIVE:
    case BOBE_FACE_HAPPY:
    case BOBE_FACE_CONCERNED:
        return "";
    }
    return "";
}

static void apply_eyes(
    bobe_face_renderer_t *renderer,
    const bobe_face_layout_t *layout
)
{
    for (size_t side = 0; side < 2U; ++side) {
        apply_box(renderer->eye[side], &layout->eye[side]);
        lv_obj_set_style_bg_color(
            renderer->eye[side],
            face_colour(layout->eye_fill_rgb),
            0
        );
        lv_obj_set_style_border_color(
            renderer->eye[side],
            face_colour(layout->eye_border_rgb),
            0
        );
        lv_obj_set_style_border_width(
            renderer->eye[side],
            layout->eye_border_width,
            0
        );
        apply_box(renderer->iris[side], &layout->iris[side]);
        apply_box(renderer->pupil[side], &layout->pupil[side]);
        apply_box(renderer->highlight[side], &layout->highlight[side]);
    }
}

static void apply_mouth(
    bobe_face_renderer_t *renderer,
    const bobe_face_layout_t *layout
)
{
    if (layout->mouth_kind == BOBE_FACE_MOUTH_SMILE) {
        lv_obj_set_hidden(renderer->mouth, true);
        lv_obj_set_hidden(renderer->smile, false);
        lv_obj_set_size(
            renderer->smile,
            layout->mouth.width,
            layout->mouth.height
        );
        lv_obj_set_pos(renderer->smile, layout->mouth.x, layout->mouth.y);
        lv_arc_set_bg_angles(
            renderer->smile,
            layout->smile_start_angle,
            layout->smile_end_angle
        );
        lv_obj_set_style_arc_color(
            renderer->smile,
            face_colour(layout->mouth_line_rgb),
            LV_PART_MAIN
        );
        return;
    }
    if (layout->mouth_kind == BOBE_FACE_MOUTH_RECT) {
        lv_obj_set_hidden(renderer->smile, true);
        lv_obj_set_hidden(renderer->mouth, false);
        lv_obj_set_size(
            renderer->mouth,
            layout->mouth.width,
            layout->mouth.height
        );
        lv_obj_set_pos(renderer->mouth, layout->mouth.x, layout->mouth.y);
        lv_obj_set_style_bg_color(
            renderer->mouth,
            face_colour(layout->mouth_fill_rgb),
            0
        );
        lv_obj_set_style_border_color(
            renderer->mouth,
            face_colour(layout->mouth_line_rgb),
            0
        );
        lv_obj_set_style_border_width(
            renderer->mouth,
            renderer->theme->mouth.rect_border_width,
            0
        );
        return;
    }
    lv_obj_set_hidden(renderer->smile, true);
    lv_obj_set_hidden(renderer->mouth, true);
}

static void set_caption_scroll(
    bobe_face_renderer_t *renderer,
    uint32_t lvgl_tick_ms,
    int32_t position
)
{
    if (position == BOBE_FACE_SCROLL_AUTOMATIC) {
        if (renderer->deterministic_scroll_active) {
            lv_anim_t *const animation = lv_anim_get(renderer->caption, NULL);
            if (animation != NULL) {
                lv_anim_resume(animation);
            }
        }
        renderer->deterministic_scroll_active = false;
        return;
    }

    lv_obj_update_layout(renderer->screen);
    lv_anim_t *const animation = lv_anim_get(renderer->caption, NULL);
    renderer->deterministic_scroll_active = true;
    if (animation == NULL || animation->exec_cb == NULL) {
        return;
    }

    lv_anim_pause(animation);
    animation->last_timer_run = lvgl_tick_ms;
    animation->pause_time = lvgl_tick_ms;
    animation->current_value = position;
    animation->exec_cb(animation->var, position);
}

void bobe_face_renderer_render(
    bobe_face_renderer_t *renderer,
    const bobe_face_render_input_t *input
)
{
    const bool theme_changed =
        input->theme_changed || input->theme != renderer->theme_id;
    if (theme_changed) {
        renderer->theme_id = input->theme;
        renderer->theme = bobe_face_theme(input->theme);
    }

    bobe_face_layout_t layout;
    bobe_face_layout_resolve(
        renderer->theme,
        &input->face,
        input->animation_phase,
        &layout
    );

    if (theme_changed) {
        apply_theme(renderer, layout.accent_rgb);
        renderer->applied_accent_rgb = layout.accent_rgb;
    } else if (renderer->applied_accent_rgb != layout.accent_rgb) {
        apply_accent(renderer, layout.accent_rgb);
        renderer->applied_accent_rgb = layout.accent_rgb;
    }

    const char *const notice_text =
        input->notice.kind != BOBE_NOTICE_NONE ? input->notice.text : "";
    const bool notice_changed =
        strcmp(notice_text, renderer->applied_notice) != 0;
    if (input->face_changed || notice_changed) {
        const char *const status_text =
            expression_status(input->face.expression);
        const char *caption_text = input->face.caption;
        if (notice_text[0] != '\0') {
            caption_text = notice_text;
        } else if (caption_text[0] == '\0') {
            caption_text = "BoBe";
        }
        lv_label_set_text(renderer->caption, caption_text);
        snprintf(
            renderer->applied_notice,
            sizeof(renderer->applied_notice),
            "%s",
            notice_text
        );
        lv_label_set_text(renderer->status, status_text);
        lv_obj_set_hidden(renderer->status, status_text[0] == '\0');
        lv_obj_align(renderer->status, LV_ALIGN_TOP_MID, 0, 6);
    }
    if (!renderer->status_initialized ||
        memcmp(
            &input->status,
            &renderer->applied_status,
            sizeof(input->status)
        ) != 0 ||
        input->face_changed || theme_changed) {
        apply_local_status(renderer, input, layout.accent_rgb);
    }

    lv_obj_set_hidden(
        renderer->microphone_indicator,
        !input->microphone_active
    );

    lv_obj_set_style_bg_color(
        renderer->status,
        face_colour(layout.status_rgb),
        0
    );
    apply_eyes(renderer, &layout);
    apply_mouth(renderer, &layout);

    set_caption_scroll(
        renderer,
        input->lvgl_tick_ms,
        input->caption_scroll_position
    );
}

void bobe_face_renderer_create(
    bobe_face_renderer_t *renderer,
    lv_obj_t *screen,
    const bobe_face_render_input_t *input
)
{
    memset(renderer, 0, sizeof(*renderer));
    renderer->screen = screen;
    renderer->theme_id = input->theme;
    renderer->theme = bobe_face_theme(input->theme);

    const bobe_face_theme_t *const theme = renderer->theme;
    const uint32_t accent =
        input->face.accent_rgb == BOBE_FACE_THEME_ACCENT_RGB
            ? theme->accent_rgb
            : input->face.accent_rgb;

    lv_obj_remove_style_all(screen);
    lv_obj_set_style_bg_opa(screen, LV_OPA_COVER, 0);
    lv_obj_set_style_bg_color(
        screen,
        face_colour(theme->background_rgb),
        0
    );

    for (size_t index = 0; index < BOBE_FACE_MAX_DECOR; ++index) {
        renderer->decor[index] = lv_obj_create(screen);
        configure_shape(renderer->decor[index], lv_color_black());
        lv_obj_set_style_border_opa(
            renderer->decor[index],
            LV_OPA_TRANSP,
            0
        );
        lv_obj_set_style_border_color(
            renderer->decor[index],
            lv_color_black(),
            0
        );
        lv_obj_set_style_border_width(renderer->decor[index], 0, 0);
        lv_obj_set_hidden(renderer->decor[index], true);
    }
    for (size_t index = 0; index < BOBE_FACE_MAX_DECOR_ARCS; ++index) {
        renderer->decor_arcs[index] = lv_arc_create(screen);
        configure_arc(renderer->decor_arcs[index], lv_color_black(), 1);
        lv_obj_set_hidden(renderer->decor_arcs[index], true);
    }

    renderer->eye[0] = lv_obj_create(screen);
    renderer->eye[1] = lv_obj_create(screen);
    renderer->iris[0] = lv_obj_create(screen);
    renderer->iris[1] = lv_obj_create(screen);
    renderer->pupil[0] = lv_obj_create(screen);
    renderer->pupil[1] = lv_obj_create(screen);
    renderer->highlight[0] = lv_obj_create(screen);
    renderer->highlight[1] = lv_obj_create(screen);
    renderer->mouth = lv_obj_create(screen);
    for (size_t side = 0; side < 2U; ++side) {
        configure_shape(renderer->eye[side], face_colour(theme->eye_fill_rgb));
        configure_shape(renderer->iris[side], face_colour(theme->iris_rgb));
        configure_shape(renderer->pupil[side], face_colour(theme->pupil_rgb));
        configure_shape(
            renderer->highlight[side],
            face_colour(theme->highlight_rgb)
        );
    }
    configure_shape(
        renderer->mouth,
        face_colour(resolve_rgb(theme->mouth.rect_fill_rgb, accent))
    );

    renderer->smile = lv_arc_create(screen);
    configure_arc(
        renderer->smile,
        face_colour(resolve_rgb(theme->mouth.smile_rgb, accent)),
        theme->mouth.smile_stroke
    );

    renderer->microphone_indicator = lv_obj_create(screen);
    configure_shape(
        renderer->microphone_indicator,
        face_colour(theme->microphone.rgb)
    );
    lv_obj_set_size(
        renderer->microphone_indicator,
        theme->microphone.size,
        theme->microphone.size
    );
    lv_obj_set_pos(
        renderer->microphone_indicator,
        theme->microphone.x,
        theme->microphone.y
    );
    lv_obj_set_hidden(renderer->microphone_indicator, true);

    for (uint8_t index = 0; index < 3U; ++index) {
        renderer->wifi_bar[index] = lv_obj_create(screen);
        configure_shape(renderer->wifi_bar[index], face_colour(accent));
        lv_obj_set_size(renderer->wifi_bar[index], 3, 3 + (int32_t)index * 3);
        lv_obj_set_pos(renderer->wifi_bar[index], 12 + (int32_t)index * 5,
                       18 - (int32_t)index * 3);
        lv_obj_set_style_radius(renderer->wifi_bar[index], 1, 0);
    }
    renderer->battery_body = lv_obj_create(screen);
    configure_shape(renderer->battery_body, face_colour(UINT32_C(0xFFFDFC)));
    lv_obj_set_size(renderer->battery_body, 20, 10);
    lv_obj_set_pos(renderer->battery_body, 246, 10);
    lv_obj_set_style_border_width(renderer->battery_body, 1, 0);
    lv_obj_set_style_border_color(renderer->battery_body, face_colour(accent), 0);
    lv_obj_set_style_radius(renderer->battery_body, 2, 0);
    renderer->battery_fill = lv_obj_create(renderer->battery_body);
    configure_shape(renderer->battery_fill, face_colour(accent));
    lv_obj_set_height(renderer->battery_fill, 6);
    lv_obj_set_pos(renderer->battery_fill, 1, 1);
    lv_obj_set_style_radius(renderer->battery_fill, 1, 0);
    renderer->battery_tip = lv_obj_create(screen);
    configure_shape(renderer->battery_tip, face_colour(accent));
    lv_obj_set_size(renderer->battery_tip, 2, 5);
    lv_obj_set_pos(renderer->battery_tip, 266, 12);
    lv_obj_set_style_radius(renderer->battery_tip, 1, 0);

    renderer->status = lv_label_create(screen);
    lv_obj_set_width(renderer->status, LV_SIZE_CONTENT);
    lv_obj_align(renderer->status, LV_ALIGN_TOP_MID, 0, 6);
    lv_obj_set_style_bg_opa(renderer->status, LV_OPA_COVER, 0);
    lv_obj_set_style_bg_color(renderer->status, face_colour(accent), 0);
    lv_obj_set_style_radius(renderer->status, LV_RADIUS_CIRCLE, 0);
    lv_obj_set_style_pad_hor(renderer->status, 8, 0);
    lv_obj_set_style_pad_ver(renderer->status, 3, 0);
    lv_obj_set_style_text_align(renderer->status, LV_TEXT_ALIGN_CENTER, 0);
    lv_obj_set_style_text_color(
        renderer->status,
        face_colour(UINT32_C(0xFFFDFC)),
        0
    );
    lv_obj_set_style_text_font(renderer->status, &lv_font_montserrat_14, 0);

    renderer->caption = lv_label_create(screen);
    apply_caption_style(renderer);
    lv_label_set_long_mode(renderer->caption, LV_LABEL_LONG_MODE_SCROLL_CIRCULAR);

    bobe_face_render_input_t initial = *input;
    initial.face_changed = true;
    initial.theme_changed = true;
    bobe_face_renderer_render(renderer, &initial);
}

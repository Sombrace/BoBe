#include "bobe_device_config.h"
#include "bobe_display.h"
#include "bobe_board_display.h"

#include "bobe_face_renderer.h"
#include "bobe_task_config.h"
#include "esp_check.h"
#include "esp_heap_caps.h"
#include "esp_lcd_panel_ops.h"
#include "esp_log.h"
#include "esp_lvgl_port.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/semphr.h"
#include "lvgl.h"

#include <stdatomic.h>
#include <string.h>

#define BOBE_DISPLAY_BUFFER_LINES 20
#define BOBE_DISPLAY_FLUSH_TIMEOUT_MS 2000U
#define BOBE_DISPLAY_HEALTH_LOG_INTERVAL_US UINT64_C(10000000)

static const char *TAG = "bobe_display";

static lv_display_t *lvgl_display;
static bobe_face_renderer_t face_renderer;
static QueueHandle_t face_queue;
static QueueHandle_t theme_queue;
static QueueHandle_t notice_queue;
static QueueHandle_t status_queue;
static SemaphoreHandle_t flush_completion;
static StaticSemaphore_t flush_completion_state;
static esp_lcd_panel_io_handle_t panel_io;
static esp_lcd_panel_handle_t panel;
static bool swap_color_bytes;
static atomic_bool microphone_active;
static atomic_bool flush_pending;
static atomic_uint flush_started_us;
static atomic_uint flush_submitted;
static atomic_uint flush_completed;
static atomic_uint flush_errors;
static atomic_uint flush_timeouts;
static atomic_uint flush_last_latency_us;
static atomic_uint flush_max_latency_us;
static atomic_uint display_stack_free_words;
static atomic_uint requested_frame_rate = CONFIG_BOBE_UI_FPS;
static atomic_uint applied_frame_rate = CONFIG_BOBE_UI_FPS;
static bobe_face_state_t active_face = {
    .expression = BOBE_FACE_BOOTING,
    .accent_rgb = BOBE_FACE_THEME_ACCENT_RGB,
};
static bobe_face_theme_id_t active_theme = BOBE_FACE_THEME_DEFAULT;
static uint64_t active_theme_version;
static bobe_notice_t active_notice;
static bobe_display_status_t active_status;
static uint64_t notice_expiry_us;
static uint32_t animation_tick;
static uint64_t last_health_log_us;

void bobe_lvgl_memory_link(void);

static void update_maximum(atomic_uint *maximum, unsigned int candidate)
{
    unsigned int current = atomic_load(maximum);
    while (candidate > current &&
           !atomic_compare_exchange_weak(maximum, &current, candidate)) {
    }
}

static bool display_flush_complete(
    esp_lcd_panel_io_handle_t unused_io,
    esp_lcd_panel_io_event_data_t *unused_event,
    void *unused_context
)
{
    (void)unused_io;
    (void)unused_event;
    (void)unused_context;

    const unsigned int latency_us =
        (unsigned int)esp_timer_get_time() -
        atomic_load_explicit(&flush_started_us, memory_order_relaxed);
    atomic_store_explicit(
        &flush_last_latency_us,
        latency_us,
        memory_order_relaxed
    );
    update_maximum(&flush_max_latency_us, latency_us);
    atomic_fetch_add_explicit(
        &flush_completed,
        1U,
        memory_order_relaxed
    );
    atomic_store_explicit(&flush_pending, false, memory_order_release);

    BaseType_t task_woken = pdFALSE;
    xSemaphoreGiveFromISR(flush_completion, &task_woken);
    return task_woken == pdTRUE;
}

static void display_flush_wait(lv_display_t *display)
{
    (void)display;
    if (xSemaphoreTake(
            flush_completion,
            pdMS_TO_TICKS(BOBE_DISPLAY_FLUSH_TIMEOUT_MS)
        ) == pdTRUE) {
        return;
    }

    atomic_fetch_add(&flush_timeouts, 1U);
    ESP_LOGE(
        TAG,
        "LCD flush timed out after %u ms: submitted=%u completed=%u "
        "errors=%u pending=%d internal_free=%u largest=%u",
        (unsigned int)BOBE_DISPLAY_FLUSH_TIMEOUT_MS,
        atomic_load(&flush_submitted),
        atomic_load(&flush_completed),
        atomic_load(&flush_errors),
        atomic_load(&flush_pending),
        (unsigned int)heap_caps_get_free_size(MALLOC_CAP_INTERNAL),
        (unsigned int)heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL)
    );
    esp_restart();
}

static void display_flush(
    lv_display_t *display,
    const lv_area_t *area,
    uint8_t *color_map
)
{
    (void)display;
    (void)xSemaphoreTake(flush_completion, 0);
    if (swap_color_bytes) {
        lv_draw_rgb565_swap(color_map, lv_area_get_size(area));
    }
    atomic_store_explicit(
        &flush_started_us,
        (unsigned int)esp_timer_get_time(),
        memory_order_relaxed
    );
    atomic_store_explicit(&flush_pending, true, memory_order_release);

    const esp_err_t result = esp_lcd_panel_draw_bitmap(
        panel,
        area->x1,
        area->y1,
        area->x2 + 1,
        area->y2 + 1,
        color_map
    );
    if (result == ESP_OK) {
        atomic_fetch_add_explicit(
            &flush_submitted,
            1U,
            memory_order_relaxed
        );
        return;
    }

    atomic_fetch_add(&flush_errors, 1U);
    atomic_store_explicit(&flush_pending, false, memory_order_release);
    ESP_LOGE(
        TAG,
        "LCD flush submission failed for (%d,%d)-(%d,%d): %s; "
        "internal_free=%u largest=%u",
        (int)area->x1,
        (int)area->y1,
        (int)area->x2,
        (int)area->y2,
        esp_err_to_name(result),
        (unsigned int)heap_caps_get_free_size(MALLOC_CAP_INTERNAL),
        (unsigned int)heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL)
    );
    xSemaphoreGive(flush_completion);
}

static void face_timer(lv_timer_t *timer)
{
    uint32_t target_fps = atomic_load(&requested_frame_rate);
    if (target_fps == 0U || target_fps > CONFIG_BOBE_UI_FPS) {
        target_fps = CONFIG_BOBE_UI_FPS;
    }
    if (active_notice.kind != BOBE_NOTICE_NONE && target_fps < 10U) {
        target_fps = 10U;
    }
    if (target_fps != atomic_load(&applied_frame_rate)) {
        atomic_store(&applied_frame_rate, target_fps);
        lv_timer_set_period(timer, (1000U + target_fps - 1U) / target_fps);
    }
    bobe_face_state_t pending;
    bool face_changed = false;
    if (xQueueReceive(face_queue, &pending, 0) == pdTRUE) {
        active_face = pending;
        face_changed = true;
    }
    bobe_theme_state_t pending_theme;
    bool theme_changed = false;
    if (xQueueReceive(theme_queue, &pending_theme, 0) == pdTRUE &&
        pending_theme.theme != active_theme) {
        active_theme = pending_theme.theme;
        theme_changed = true;
    }
    const uint64_t frame_us = (uint64_t)esp_timer_get_time();
    bobe_notice_t pending_notice;
    if (xQueueReceive(notice_queue, &pending_notice, 0) == pdTRUE) {
        active_notice = pending_notice;
        notice_expiry_us =
            frame_us + (uint64_t)pending_notice.ttl_ms * UINT64_C(1000);
    }
    if (active_notice.kind != BOBE_NOTICE_NONE &&
        frame_us >= notice_expiry_us) {
        memset(&active_notice, 0, sizeof(active_notice));
        notice_expiry_us = 0;
    }
    bobe_display_status_t pending_status;
    if (xQueueReceive(status_queue, &pending_status, 0) == pdTRUE) {
        active_status = pending_status;
    }
    const bobe_face_render_input_t input = {
        .face = active_face,
        .theme = active_theme,
        .microphone_active = atomic_load(&microphone_active),
        .face_changed = face_changed,
        .theme_changed = theme_changed,
        .notice = active_notice,
        .status = active_status,
        .animation_phase = animation_tick++,
        .lvgl_tick_ms = lv_tick_get(),
        .caption_scroll_position = BOBE_FACE_SCROLL_AUTOMATIC,
    };
    bobe_face_renderer_render(&face_renderer, &input);

    const uint64_t now_us = frame_us;
    if (now_us - last_health_log_us >=
        BOBE_DISPLAY_HEALTH_LOG_INTERVAL_US) {
        last_health_log_us = now_us;
        atomic_store(
            &display_stack_free_words,
            (unsigned int)uxTaskGetStackHighWaterMark(NULL)
        );
        ESP_LOGI(
            TAG,
            "health flush=%u/%u errors=%u timeouts=%u pending=%d "
            "last_us=%u max_us=%u stack_free_words=%u internal_free=%u "
            "largest=%u",
            atomic_load(&flush_completed),
            atomic_load(&flush_submitted),
            atomic_load(&flush_errors),
            atomic_load(&flush_timeouts),
            atomic_load(&flush_pending),
            atomic_load(&flush_last_latency_us),
            atomic_load(&flush_max_latency_us),
            (unsigned int)uxTaskGetStackHighWaterMark(NULL),
            (unsigned int)heap_caps_get_free_size(MALLOC_CAP_INTERNAL),
            (unsigned int)heap_caps_get_largest_free_block(
                MALLOC_CAP_INTERNAL
            )
        );
    }
}

static esp_err_t create_face_objects(void)
{
    if (!lvgl_port_lock(1000)) {
        return ESP_ERR_TIMEOUT;
    }

    lv_display_set_default(lvgl_display);
    lv_obj_t *screen = lv_screen_active();
    const bobe_face_render_input_t input = {
        .face = active_face,
        .theme = active_theme,
        .microphone_active = atomic_load(&microphone_active),
        .face_changed = true,
        .theme_changed = true,
        .animation_phase = animation_tick++,
        .lvgl_tick_ms = lv_tick_get(),
        .caption_scroll_position = BOBE_FACE_SCROLL_AUTOMATIC,
    };
    bobe_face_renderer_create(&face_renderer, screen, &input);
    lv_timer_t *const animation_timer = lv_timer_create(
        face_timer,
        (uint32_t)((1000 + CONFIG_BOBE_UI_FPS - 1) /
                   CONFIG_BOBE_UI_FPS),
        NULL
    );
    lv_obj_invalidate(screen);
    lv_refr_now(lvgl_display);
    lvgl_port_unlock();
    return animation_timer != NULL ? ESP_OK : ESP_ERR_NO_MEM;
}

esp_err_t bobe_display_init(void)
{
    bobe_lvgl_memory_link();
    flush_completion = xSemaphoreCreateBinaryStatic(&flush_completion_state);
    if (flush_completion == NULL) {
        return ESP_ERR_NO_MEM;
    }
    bobe_board_display_t board_display;
    ESP_RETURN_ON_ERROR(
        bobe_board_create_display(
            BOBE_DISPLAY_WIDTH * BOBE_DISPLAY_BUFFER_LINES * sizeof(uint16_t),
            &board_display
        ),
        TAG,
        "initialize panel"
    );

    panel_io = board_display.io;
    panel = board_display.panel;
    swap_color_bytes = board_display.swap_color_bytes;

    face_queue = xQueueCreate(1, sizeof(bobe_face_state_t));
    if (face_queue == NULL) {
        return ESP_ERR_NO_MEM;
    }

    theme_queue = xQueueCreate(1, sizeof(bobe_theme_state_t));
    if (theme_queue == NULL) {
        return ESP_ERR_NO_MEM;
    }

    notice_queue = xQueueCreate(1, sizeof(bobe_notice_t));
    if (notice_queue == NULL) {
        return ESP_ERR_NO_MEM;
    }
    status_queue = xQueueCreate(1, sizeof(bobe_display_status_t));
    if (status_queue == NULL) {
        return ESP_ERR_NO_MEM;
    }

    lvgl_port_cfg_t port_config = ESP_LVGL_PORT_INIT_CONFIG();
    port_config.task_priority = BOBE_TASK_PRIORITY_UI;
    port_config.task_stack = 8192;
    port_config.task_stack_caps = MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT;
    port_config.task_max_sleep_ms = 100;
    port_config.task_affinity = BOBE_NETWORK_CORE;
    ESP_RETURN_ON_ERROR(
        lvgl_port_init(&port_config),
        TAG,
        "initialize LVGL port"
    );

    const lvgl_port_display_cfg_t display_config = {
        .io_handle = panel_io,
        .panel_handle = panel,
        .buffer_size = BOBE_DISPLAY_WIDTH * BOBE_DISPLAY_BUFFER_LINES,
        .double_buffer = false,
        .hres = BOBE_DISPLAY_WIDTH,
        .vres = BOBE_DISPLAY_HEIGHT,
        .monochrome = false,
        .rotation = {
            .swap_xy = board_display.swap_xy,
            .mirror_x = board_display.mirror_x,
            .mirror_y = board_display.mirror_y,
        },
        .color_format = LV_COLOR_FORMAT_RGB565,
        .flags = {
            .buff_dma = true,
            .buff_spiram = false,
            .sw_rotate = false,
            .swap_bytes = board_display.swap_color_bytes,
        },
    };
    if (!lvgl_port_lock(0)) {
        return ESP_ERR_TIMEOUT;
    }
    lvgl_display = lvgl_port_add_disp(&display_config);
    if (lvgl_display == NULL) {
        lvgl_port_unlock();
        return ESP_ERR_NO_MEM;
    }
    lv_display_set_flush_cb(lvgl_display, display_flush);
    lv_display_set_flush_wait_cb(lvgl_display, display_flush_wait);
    const esp_lcd_panel_io_callbacks_t panel_callbacks = {
        .on_color_trans_done = display_flush_complete,
    };
    const esp_err_t callback_result =
        esp_lcd_panel_io_register_event_callbacks(
            panel_io,
            &panel_callbacks,
            NULL
        );
    lvgl_port_unlock();
    ESP_RETURN_ON_ERROR(
        callback_result,
        TAG,
        "register checked LCD completion callback"
    );
    ESP_LOGI(
        TAG,
        "LVGL uses one %u-byte internal DMA buffer; internal free=%u largest=%u",
        (unsigned int)(
            BOBE_DISPLAY_WIDTH * BOBE_DISPLAY_BUFFER_LINES *
            sizeof(uint16_t)
        ),
        (unsigned int)heap_caps_get_free_size(MALLOC_CAP_INTERNAL),
        (unsigned int)heap_caps_get_largest_free_block(MALLOC_CAP_INTERNAL)
    );
    return create_face_objects();
}

void bobe_display_set_face(const bobe_face_state_t *face)
{
    if (face != NULL && face_queue != NULL) {
        (void)xQueueOverwrite(face_queue, face);
        (void)lvgl_port_task_wake(LVGL_PORT_EVENT_USER, NULL);
    }
}

void bobe_display_set_theme(const bobe_theme_state_t *theme)
{
    if (theme == NULL || theme_queue == NULL) {
        return;
    }
    if (theme->theme == active_theme &&
        theme->state_version == active_theme_version) {
        return;
    }
    active_theme_version = theme->state_version;
    (void)xQueueOverwrite(theme_queue, theme);
    (void)lvgl_port_task_wake(LVGL_PORT_EVENT_USER, NULL);
}

void bobe_display_set_status(const bobe_display_status_t *status)
{
    if (status != NULL && status_queue != NULL) {
        (void)xQueueOverwrite(status_queue, status);
        (void)lvgl_port_task_wake(LVGL_PORT_EVENT_USER, NULL);
    }
}

void bobe_display_set_frame_rate(uint8_t frames_per_second)
{
    if (frames_per_second == 0U || frames_per_second > CONFIG_BOBE_UI_FPS) {
        return;
    }
    atomic_store(&requested_frame_rate, frames_per_second);
    (void)lvgl_port_task_wake(LVGL_PORT_EVENT_USER, NULL);
}

void bobe_display_get_stats(bobe_display_stats_t *stats)
{
    if (stats == NULL) {
        return;
    }
    *stats = (bobe_display_stats_t){
        .stack_free_words = atomic_load(&display_stack_free_words),
        .flush_submitted = atomic_load(&flush_submitted),
        .flush_errors = atomic_load(&flush_errors),
        .flush_timeouts = atomic_load(&flush_timeouts),
        .flush_last_us = atomic_load(&flush_last_latency_us),
        .flush_max_us = atomic_load(&flush_max_latency_us),
        .frame_rate = atomic_load(&applied_frame_rate),
    };
}

void bobe_display_set_microphone_active(bool active)
{
    atomic_store(&microphone_active, active);
    (void)lvgl_port_task_wake(LVGL_PORT_EVENT_USER, NULL);
}

void bobe_display_show_notice(const bobe_notice_t *notice)
{
    if (notice == NULL || notice_queue == NULL ||
        notice->kind == BOBE_NOTICE_NONE || notice->ttl_ms == 0U) {
        return;
    }
    (void)xQueueOverwrite(notice_queue, notice);
    (void)lvgl_port_task_wake(LVGL_PORT_EVENT_USER, NULL);
}

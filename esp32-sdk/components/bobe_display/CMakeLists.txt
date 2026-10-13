set(bobe_display_requires
    bobe_common
    bobe_board
    esp_lcd
    esp_lvgl_port
    esp_system
    esp_timer
    freertos
    heap
    log
    lvgl
)

idf_component_register(
    PRIV_REQUIRES bobe_device_config
    SRCS
        "bobe_display.c"
        "bobe_face_renderer.c"
        "bobe_face_theme.c"
        "bobe_lvgl_memory.c"
    INCLUDE_DIRS "include"
    REQUIRES bobe_platform ${bobe_display_requires}
)

idf_component_register(
    SRCS "bobe_audio.c"
    INCLUDE_DIRS "include"
    REQUIRES
        bobe_platform bobe_common
        bobe_board
        esp_driver_i2s
        esp_timer
        freertos
        log
)

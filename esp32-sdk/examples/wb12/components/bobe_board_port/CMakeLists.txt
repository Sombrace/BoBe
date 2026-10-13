idf_component_register(
    SRCS "bobe_board.c" "bobe_board_audio.c" "bobe_board_display.c" "bobe_board_key.c" "bobe_battery.c"
    INCLUDE_DIRS "include"
    REQUIRES
        bobe_board bobe_common esp_adc esp_driver_gpio esp_driver_ledc
        esp_driver_tsens esp_driver_spi esp_hw_support esp_timer freertos log
    EMBED_TXTFILES "update-signing-public.pem"
)

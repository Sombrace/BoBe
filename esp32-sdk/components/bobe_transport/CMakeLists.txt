idf_component_register(
    SRCS "bobe_transport.c"
    INCLUDE_DIRS "include"
    REQUIRES
        bobe_platform
        bobe_protocol
        esp_system
        freertos
        heap
        log
        nvs_flash
        esp_timer
        esp_websocket_client
)

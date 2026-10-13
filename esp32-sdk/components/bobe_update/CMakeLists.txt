idf_component_register(
    PRIV_REQUIRES bobe_device_config
    SRCS
        "bobe_update_verify_esp.c"
        "bobe_update.c"
    INCLUDE_DIRS "include"
    REQUIRES
        bobe_platform bobe_common
        bobe_protocol
        bobe_board
        app_update
        cjson
        esp_app_format
        esp_http_client
        heap
        mbedtls
        nvs_flash
)

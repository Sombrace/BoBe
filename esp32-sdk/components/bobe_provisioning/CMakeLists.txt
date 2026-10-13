set(credentials_source "bobe_provisioning_credentials_stub.c")
set(private_credentials
    "${PROJECT_DIR}/private/provisioning/bobe_provisioning_credentials_private.c"
)
if(EXISTS "${private_credentials}")
    set(credentials_source "${private_credentials}")
endif()

idf_component_register(
    SRCS
        "bobe_provisioning.c"
        "${credentials_source}"
    INCLUDE_DIRS "include"
    REQUIRES
        bobe_platform bobe_common
        bt
        cjson
        esp_hw_support
        esp_http_server
        esp_timer
        freertos
        log
        mbedtls
        network_provisioning
        nvs_flash
)

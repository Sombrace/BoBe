set(private_includes "")
if(EXISTS "${PROJECT_DIR}/private/bobe_credentials.h")
    list(APPEND private_includes "${PROJECT_DIR}/private")
endif()
idf_component_register(SRCS "main.c" INCLUDE_DIRS "." ${private_includes}
    REQUIRES bobe_client protocol_examples_common nvs_flash esp_event esp_netif)

idf_component_register(SRCS "esp_bobe_client.c" INCLUDE_DIRS "include"
    REQUIRES bobe_protocol
    PRIV_REQUIRES bobe_transport bobe_platform esp_app_format esp_system)

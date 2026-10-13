idf_component_register(SRCS "bobe_storage.c" "bobe_identity.c" INCLUDE_DIRS "include"
    REQUIRES nvs_flash esp_hw_support)

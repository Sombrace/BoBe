idf_component_register(
    PRIV_REQUIRES bobe_device_config
    SRCS "esp_bobe_sdk.c"
    INCLUDE_DIRS "include"
    REQUIRES bobe_platform bobe_board bobe_core bobe_client bobe_usb_onboarding esp_common freertos log
)

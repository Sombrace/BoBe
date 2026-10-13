idf_component_register(
    PRIV_REQUIRES bobe_device_config
    SRCS
        "bobe_usb_onboarding.c"
        "bobe_usb_onboarding_protocol.c"
    INCLUDE_DIRS "include"
    PRIV_INCLUDE_DIRS "."
    REQUIRES
        bobe_platform bobe_common
        bobe_connectivity
        esp_driver_usb_serial_jtag
        esp_hw_support
        freertos
        vfs
)

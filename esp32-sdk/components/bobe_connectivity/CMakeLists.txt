idf_component_register(
    PRIV_REQUIRES bobe_device_config
    SRCS
        "bobe_connectivity.c"
        "bobe_wifi_recovery.c"
    INCLUDE_DIRS "include"
    REQUIRES
        bobe_platform bobe_common
        bobe_protocol
        bobe_provisioning
        esp_event
        esp_hw_support
        esp_netif
        esp_timer
        esp_wifi
        freertos
        log
        lwip
        mdns
        nvs_flash
)

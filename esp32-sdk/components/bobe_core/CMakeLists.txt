set(recovery_seed_source "bobe_recovery_seed_stub.c")
set(private_recovery_seed
    "${PROJECT_DIR}/private/recovery/bobe_recovery_seed_private.c"
)
if(CONFIG_BOBE_ENABLE_DEV_WIFI_BOOTSTRAP AND EXISTS "${private_recovery_seed}")
    set(recovery_seed_source "${private_recovery_seed}")
endif()

idf_component_register(
    PRIV_REQUIRES bobe_device_config
    SRCS
        "bobe_core.c"
        "bobe_notice.c"
        "bobe_policy.c"
        "bobe_state.c"
        "${recovery_seed_source}"
    INCLUDE_DIRS "include"
    REQUIRES
        bobe_platform bobe_audio
        bobe_board
        bobe_common
        bobe_connectivity
        bobe_display
        bobe_protocol
        bobe_transport
        bobe_update
        bobe_voice_frontend
        app_update
        esp_app_format
        esp_hw_support
        esp_pm
        esp_system
        esp_timer
        freertos
        heap
        log
        nvs_flash
)

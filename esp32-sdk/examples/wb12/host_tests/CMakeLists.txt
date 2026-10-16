cmake_minimum_required(VERSION 3.20)
project(bobe_host_tests C)

option(BOBE_ENABLE_SANITIZERS "Enable AddressSanitizer and UBSan" OFF)

enable_testing()

if(NOT ESP_BOBE_SDK_ROOT)
    get_filename_component(ESP_BOBE_SDK_ROOT "${CMAKE_CURRENT_LIST_DIR}/../../.." ABSOLUTE)
endif()
include_directories("${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/include"
                    "${CMAKE_CURRENT_LIST_DIR}/../components/bobe_device_config/include"
                    "${CMAKE_CURRENT_LIST_DIR}/../components/bobe_board_port/include")

add_executable(
    bobe_host_tests
    test_main.c
    ../components/bobe_board_port/bobe_battery.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/bobe_control.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/bobe_json.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/bobe_interaction.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/bobe_frame.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_core/bobe_state.c
    ../managed_components/espressif__cjson/cJSON/cJSON.c
)
add_executable(
    bobe_core_policy_tests
    test_core_policy.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_core/bobe_notice.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_core/bobe_policy.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_core/bobe_state.c
)
add_executable(
    bobe_notice_tests
    test_notice.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_core/bobe_notice.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_core/bobe_state.c
)
add_executable(
    bobe_theme_tests
    test_theme.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_display/bobe_face_theme.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/bobe_control.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/bobe_json.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/bobe_interaction.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/bobe_frame.c
    ../managed_components/espressif__cjson/cJSON/cJSON.c
)
add_executable(
    bobe_wifi_recovery_tests
    test_wifi_recovery.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_connectivity/bobe_wifi_recovery.c
)
add_executable(
    bobe_usb_onboarding_tests
    test_usb_onboarding.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_usb_onboarding/bobe_usb_onboarding_protocol.c
)
add_executable(
    bobe_update_manifest_tests
    test_update_manifest.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/bobe_update_manifest.c
)
add_executable(
    bobe_update_offer_tests
    test_update_offer.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/bobe_update_manifest.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/bobe_update_offer.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/bobe_json.c
    ../managed_components/espressif__cjson/cJSON/cJSON.c
)
add_executable(
    bobe_claim_tests
    test_claim.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_common/bobe_claim.c
)
add_executable(
    bobe_power_tests
    test_power.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_common/bobe_power.c
)
add_executable(
    bobe_audio_tests
    test_audio.c
    idf/idf_mock.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_audio/bobe_audio.c
)
add_executable(
    bobe_voice_frontend_tests
    test_voice_frontend.c
    idf/idf_mock.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_voice_frontend/bobe_voice_frontend.c
)
add_executable(
    bobe_claim_store_tests
    test_claim_store.c
    idf/idf_mock.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_provisioning/bobe_claim_store.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_platform/bobe_storage.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_common/bobe_claim.c
)
set_source_files_properties(
    ../managed_components/espressif__cjson/cJSON/cJSON.c
    PROPERTIES
        COMPILE_OPTIONS "-Wno-deprecated-declarations"
)

target_include_directories(
    bobe_host_tests
    PRIVATE
        ${ESP_BOBE_SDK_ROOT}/components/bobe_common/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_core/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/include
        ../managed_components/espressif__cjson/cJSON
)
target_include_directories(
    bobe_core_policy_tests
    PRIVATE
        ${ESP_BOBE_SDK_ROOT}/components/bobe_common/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_core/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/include
)
target_include_directories(
    bobe_notice_tests
    PRIVATE
        ${ESP_BOBE_SDK_ROOT}/components/bobe_common/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_core/include
)
target_include_directories(
    bobe_theme_tests
    PRIVATE
        ${ESP_BOBE_SDK_ROOT}/components/bobe_common/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_display/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/include
        ../managed_components/espressif__cjson/cJSON
)
target_include_directories(
    bobe_wifi_recovery_tests
    PRIVATE
        ${ESP_BOBE_SDK_ROOT}/components/bobe_connectivity/include
)
target_include_directories(
    bobe_usb_onboarding_tests
    PRIVATE
        ${ESP_BOBE_SDK_ROOT}/components/bobe_usb_onboarding
)
target_include_directories(
    bobe_update_manifest_tests
    PRIVATE
        ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/include
)
target_include_directories(
    bobe_update_offer_tests
    PRIVATE
        ${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/include
        ../managed_components/espressif__cjson/cJSON
)
target_include_directories(
    bobe_claim_tests
    PRIVATE
        ${ESP_BOBE_SDK_ROOT}/components/bobe_common/include
)
target_include_directories(
    bobe_power_tests
    PRIVATE
        ${ESP_BOBE_SDK_ROOT}/components/bobe_common/include
)
foreach(target bobe_audio_tests bobe_voice_frontend_tests bobe_claim_store_tests)
    target_include_directories(${target} BEFORE PRIVATE idf)
endforeach()
target_include_directories(
    bobe_audio_tests
    PRIVATE
        ${ESP_BOBE_SDK_ROOT}/components/bobe_audio/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_board/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_platform/include
)
target_include_directories(
    bobe_voice_frontend_tests
    PRIVATE
        ${ESP_BOBE_SDK_ROOT}/components/bobe_voice_frontend/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_platform/include
)
target_include_directories(
    bobe_claim_store_tests
    PRIVATE
        ${ESP_BOBE_SDK_ROOT}/components/bobe_provisioning
        ${ESP_BOBE_SDK_ROOT}/components/bobe_platform/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_common/include
)

target_compile_definitions(
    bobe_host_tests
    PRIVATE
        CONFIG_BOBE_AUDIO_PLAYBACK_CREDIT_MS=300
)
target_compile_definitions(
    bobe_theme_tests
    PRIVATE
        CONFIG_BOBE_AUDIO_PLAYBACK_CREDIT_MS=300
)
target_compile_definitions(
    bobe_audio_tests
    PRIVATE
        CONFIG_BOBE_AUDIO_PLAYBACK_CREDIT_MS=300
)
target_compile_definitions(
    bobe_voice_frontend_tests
    PRIVATE
        CONFIG_BOBE_ENABLE_ESP_SR=1
)

foreach(
    target
    bobe_host_tests
    bobe_core_policy_tests
    bobe_notice_tests
    bobe_theme_tests
    bobe_wifi_recovery_tests
    bobe_usb_onboarding_tests
    bobe_update_manifest_tests
    bobe_update_offer_tests
    bobe_claim_tests
    bobe_power_tests
    bobe_audio_tests
    bobe_voice_frontend_tests
    bobe_claim_store_tests
)
    target_compile_features(${target} PRIVATE c_std_11)
    target_compile_options(
        ${target}
        PRIVATE
            -Wall
            -Wextra
            -Werror
            -Wpedantic
    )
    if(BOBE_ENABLE_SANITIZERS)
        target_compile_options(
            ${target}
            PRIVATE
                -fno-omit-frame-pointer
                -fsanitize=address,undefined
        )
        target_link_options(
            ${target}
            PRIVATE
                -fsanitize=address,undefined
        )
    endif()
endforeach()

add_test(NAME bobe_host_tests COMMAND bobe_host_tests)
add_test(NAME bobe_core_policy_tests COMMAND bobe_core_policy_tests)
add_test(NAME bobe_notice_tests COMMAND bobe_notice_tests)
add_test(NAME bobe_theme_tests COMMAND bobe_theme_tests)
add_test(NAME bobe_wifi_recovery_tests COMMAND bobe_wifi_recovery_tests)
add_test(NAME bobe_usb_onboarding_tests COMMAND bobe_usb_onboarding_tests)
add_test(NAME bobe_update_manifest_tests COMMAND bobe_update_manifest_tests)
add_test(NAME bobe_update_offer_tests COMMAND bobe_update_offer_tests)
add_test(NAME bobe_claim_tests COMMAND bobe_claim_tests)
add_test(NAME bobe_power_tests COMMAND bobe_power_tests)
add_test(NAME bobe_audio_tests COMMAND bobe_audio_tests)
add_test(NAME bobe_voice_frontend_tests COMMAND bobe_voice_frontend_tests)
add_test(NAME bobe_claim_store_tests COMMAND bobe_claim_store_tests)

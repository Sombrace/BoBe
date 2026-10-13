cmake_minimum_required(VERSION 3.20)
project(esp_bobe_sdk_contracts C)
enable_testing()

get_filename_component(SDK_ROOT "${CMAKE_CURRENT_LIST_DIR}/.." ABSOLUTE)
set(CJSON_ROOT "${SDK_ROOT}/examples/wb12/managed_components/espressif__cjson/cJSON")

add_executable(bobe_device_contract
    test_device_contract.c
    ${SDK_ROOT}/components/bobe_protocol/bobe_control.c
    ${SDK_ROOT}/components/bobe_protocol/bobe_json.c
    ${SDK_ROOT}/components/bobe_protocol/bobe_interaction.c
    ${SDK_ROOT}/components/bobe_protocol/bobe_update_manifest.c
    ${CJSON_ROOT}/cJSON.c
)
target_include_directories(bobe_device_contract PRIVATE
    device_config
    ${SDK_ROOT}/components/bobe_common/include
    ${SDK_ROOT}/components/bobe_protocol/include
    ${SDK_ROOT}/components/bobe_protocol/include
    ${CJSON_ROOT}
)
target_compile_definitions(bobe_device_contract PRIVATE CONFIG_BOBE_AUDIO_PLAYBACK_CREDIT_MS=300)
target_compile_features(bobe_device_contract PRIVATE c_std_11)
target_compile_options(bobe_device_contract PRIVATE -Wall -Wextra -Werror -Wpedantic
    -fno-omit-frame-pointer -fsanitize=address,undefined)
target_link_options(bobe_device_contract PRIVATE -fsanitize=address,undefined)
set_source_files_properties(${CJSON_ROOT}/cJSON.c PROPERTIES
    COMPILE_OPTIONS "-Wno-deprecated-declarations")
add_test(NAME independent_device_contract COMMAND bobe_device_contract)

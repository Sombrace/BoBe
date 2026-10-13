cmake_minimum_required(VERSION 3.20)
project(bobe_ui_visual_tests LANGUAGES C CXX)

enable_testing()

if(NOT ESP_BOBE_SDK_ROOT)
    get_filename_component(ESP_BOBE_SDK_ROOT "${CMAKE_CURRENT_LIST_DIR}/../../../.." ABSOLUTE)
endif()
include_directories("${ESP_BOBE_SDK_ROOT}/components/bobe_protocol/include"
                    "${CMAKE_CURRENT_LIST_DIR}/../../components/bobe_device_config/include")

set(LV_BUILD_CONF_PATH "${CMAKE_CURRENT_SOURCE_DIR}/lv_conf.h" CACHE FILEPATH "" FORCE)
set(CONFIG_LV_BUILD_DEMOS OFF CACHE BOOL "" FORCE)
set(CONFIG_LV_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(CONFIG_LV_USE_THORVG_INTERNAL OFF CACHE BOOL "" FORCE)
add_subdirectory(
    ../../managed_components/lvgl__lvgl
    "${CMAKE_CURRENT_BINARY_DIR}/lvgl"
)

add_executable(
    bobe_ui_visual_tests
    visual_test.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_display/bobe_face_renderer.c
    ${ESP_BOBE_SDK_ROOT}/components/bobe_display/bobe_face_theme.c
)
target_include_directories(
    bobe_ui_visual_tests
    PRIVATE
        ${ESP_BOBE_SDK_ROOT}/components/bobe_common/include
        ${ESP_BOBE_SDK_ROOT}/components/bobe_display/include
)
target_link_libraries(bobe_ui_visual_tests PRIVATE lvgl::lvgl)
target_compile_features(bobe_ui_visual_tests PRIVATE c_std_11)
target_compile_options(
    bobe_ui_visual_tests
    PRIVATE
        -Wall
        -Wextra
        -Werror
        -Wpedantic
)

set(BOBE_UI_REFERENCES "${CMAKE_CURRENT_SOURCE_DIR}/references.sha256")
set(BOBE_UI_OUTPUT "${CMAKE_CURRENT_BINARY_DIR}/rendered")

add_test(
    NAME bobe_ui_visual_tests
    COMMAND
        "${CMAKE_COMMAND}"
        "-DMODE=compare"
        "-DRENDERER=$<TARGET_FILE:bobe_ui_visual_tests>"
        "-DOUTPUT_DIR=${BOBE_UI_OUTPUT}"
        "-DREFERENCE_FILE=${BOBE_UI_REFERENCES}"
        -P "${CMAKE_CURRENT_SOURCE_DIR}/visual_hashes.cmake"
)

add_custom_target(
    bobe_ui_regenerate
    COMMAND
        "${CMAKE_COMMAND}"
        "-DMODE=regenerate"
        "-DRENDERER=$<TARGET_FILE:bobe_ui_visual_tests>"
        "-DOUTPUT_DIR=${BOBE_UI_OUTPUT}"
        "-DREFERENCE_FILE=${BOBE_UI_REFERENCES}"
        -P "${CMAKE_CURRENT_SOURCE_DIR}/visual_hashes.cmake"
    DEPENDS bobe_ui_visual_tests
    VERBATIM
)

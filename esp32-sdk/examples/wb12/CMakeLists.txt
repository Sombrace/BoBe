cmake_minimum_required(VERSION 3.20)

# A consuming project can live anywhere and point to an SDK checkout.
if(NOT ESP_BOBE_SDK_PATH AND DEFINED ENV{ESP_BOBE_SDK_PATH})
    set(ESP_BOBE_SDK_PATH "$ENV{ESP_BOBE_SDK_PATH}")
endif()
if(NOT ESP_BOBE_SDK_PATH)
    get_filename_component(ESP_BOBE_SDK_PATH "${CMAKE_CURRENT_LIST_DIR}/../.." ABSOLUTE)
endif()
include("${ESP_BOBE_SDK_PATH}/cmake/esp_bobe_sdk.cmake")
include($ENV{IDF_PATH}/tools/cmake/project.cmake)
set(PROJECT_VER "0.2.0")
project(bobe_wb12)

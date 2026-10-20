cmake_minimum_required(VERSION 3.24)
project(esp_bobe VERSION 0.2.0 LANGUAGES C CXX)

include(CTest)
include(FetchContent)
option(ESP_BOBE_SANITIZE "Enable address and undefined-behavior sanitizers" OFF)
set(ENABLE_CJSON_TEST OFF CACHE BOOL "" FORCE)
set(ENABLE_CJSON_UTILS OFF CACHE BOOL "" FORCE)
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(CMAKE_POLICY_VERSION_MINIMUM 3.10)
FetchContent_Declare(cjson
    URL https://codeload.github.com/DaveGamble/cJSON/tar.gz/b2890c8d76bbb64e710585ebc0a917196b9c67e7
    URL_HASH SHA256=c3866048d822428ab289d20842c75a39c92c26e967fed2f0706f6f15e6894082
    DOWNLOAD_EXTRACT_TIMESTAMP TRUE
)
FetchContent_MakeAvailable(cjson)
target_include_directories(cjson PUBLIC "$<BUILD_INTERFACE:${cjson_SOURCE_DIR}>")

add_subdirectory(components/bobe_protocol)
add_subdirectory(components/bobe_text)
add_executable(bobe-protocol tools/native/protocol_probe.c)
target_link_libraries(bobe-protocol PRIVATE bobe_protocol)
target_compile_options(bobe_protocol PRIVATE -Wall -Wextra -Werror -Wpedantic)
if(ESP_BOBE_SANITIZE AND CMAKE_C_COMPILER_ID MATCHES "Clang|GNU")
    target_compile_options(bobe_protocol PUBLIC -fno-omit-frame-pointer -fsanitize=address,undefined)
    target_link_options(bobe_protocol PUBLIC -fsanitize=address,undefined)
endif()
if(BUILD_TESTING)
    add_subdirectory(tests/portable)
endif()

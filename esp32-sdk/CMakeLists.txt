cmake_minimum_required(VERSION 3.24)
project(esp_bobe VERSION 0.2.0 LANGUAGES C CXX)

include(CTest)
include(FetchContent)
option(ESP_BOBE_SANITIZE "Enable address and undefined-behavior sanitizers" OFF)
set(ENABLE_CJSON_TEST OFF CACHE BOOL "" FORCE)
set(ENABLE_CJSON_UTILS OFF CACHE BOOL "" FORCE)
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(CMAKE_POLICY_VERSION_MINIMUM 3.5)
FetchContent_Declare(cjson
    URL https://codeload.github.com/DaveGamble/cJSON/tar.gz/c859b25da02955fef659d658b8f324b5cde87be3
    URL_HASH SHA256=880a86445ad5cf38f877bac31198268d8fac5d2ce996ebf55d3066a4109ddd8d
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

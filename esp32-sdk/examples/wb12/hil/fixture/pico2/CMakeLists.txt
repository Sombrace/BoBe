cmake_minimum_required(VERSION 3.13)

include(pico_sdk_import.cmake)

project(bobe_fixture C CXX ASM)

set(CMAKE_C_STANDARD 11)
set(CMAKE_CXX_STANDARD 17)

pico_sdk_init()

add_executable(bobe_fixture main.c)
set_source_files_properties(
    main.c
    PROPERTIES
        COMPILE_OPTIONS "-Wall;-Wextra;-Werror;-Wpedantic"
)
target_link_libraries(
    bobe_fixture
    PRIVATE
        pico_stdlib
        pico_unique_id
        hardware_watchdog
)
pico_enable_stdio_usb(bobe_fixture 1)
pico_enable_stdio_uart(bobe_fixture 0)
pico_add_extra_outputs(bobe_fixture)

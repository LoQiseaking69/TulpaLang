# Raspberry Pi Pico (RP2040) firmware written in Tulpa.
#   export PICO_SDK_PATH=/path/to/pico-sdk
#   cmake -B build && cmake --build build      ->  build/@NAME@.uf2
cmake_minimum_required(VERSION 3.13)
include($ENV{PICO_SDK_PATH}/external/pico_sdk_import.cmake)
project(@NAME@ C CXX ASM)
set(CMAKE_C_STANDARD 11)
pico_sdk_init()

find_program(TULPA tulpa REQUIRED)
set(APP_SRC ${CMAKE_CURRENT_SOURCE_DIR}/app.rav)
set(APP_OBJ ${CMAKE_CURRENT_BINARY_DIR}/app.o)
add_custom_command(
    OUTPUT ${APP_OBJ} ${CMAKE_CURRENT_BINARY_DIR}/app.h
    COMMAND ${TULPA} build --target rp2040 ${APP_SRC} -o ${APP_OBJ}
    DEPENDS ${APP_SRC}
    COMMENT "tulpa build --target rp2040 app.rav"
    VERBATIM)

add_executable(@NAME@ main.c tulpa/tulpa_rt.c tulpa/tulpa_dyn.c tulpa/hal_pico.c ${APP_OBJ})
target_include_directories(@NAME@ PRIVATE tulpa ${CMAKE_CURRENT_BINARY_DIR})
# Programs outside the static native subset run on the Tulpa VM (tulpa_dyn.c), which wants a
# deeper stack than the SDK's 2 KB default. Unused, the VM is dropped by the linker.
target_compile_definitions(@NAME@ PRIVATE PICO_STACK_SIZE=0xC000 TULPA_DYN_C_STACK=44000)
target_link_libraries(@NAME@ pico_stdlib hardware_i2c hardware_spi hardware_pwm hardware_uart)
pico_enable_stdio_usb(@NAME@ 1)
pico_enable_stdio_uart(@NAME@ 1)
pico_add_extra_outputs(@NAME@)

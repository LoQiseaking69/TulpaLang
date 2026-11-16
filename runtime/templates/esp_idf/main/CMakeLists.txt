idf_component_register(
    SRCS "main.c" "tulpa/tulpa_rt.c" "tulpa/tulpa_dyn.c" "tulpa/hal_esp_idf.c"
    INCLUDE_DIRS "tulpa" "${CMAKE_CURRENT_BINARY_DIR}"
    REQUIRES driver esp_timer esp_driver_gpio esp_driver_i2c esp_driver_spi esp_driver_ledc esp_driver_uart
             esp_wifi esp_netif esp_event nvs_flash lwip)

# Compile app.rav for this chip. Xtensa chips (esp32, esp32s2, esp32s3) need Espressif's clang:
#   python $IDF_PATH/tools/idf_tools.py install esp-clang
# tulpa finds it in ~/.espressif automatically (or set TULPA_CLANG).
find_program(TULPA tulpa REQUIRED)
set(APP_SRC ${CMAKE_CURRENT_SOURCE_DIR}/../app.rav)
set(APP_OBJ ${CMAKE_CURRENT_BINARY_DIR}/app.o)
add_custom_command(
    OUTPUT ${APP_OBJ} ${CMAKE_CURRENT_BINARY_DIR}/app.h
    COMMAND ${TULPA} build --target ${IDF_TARGET} ${APP_SRC} -o ${APP_OBJ}
    DEPENDS ${APP_SRC}
    COMMENT "tulpa build --target ${IDF_TARGET} app.rav"
    VERBATIM)
add_custom_target(tulpa_app DEPENDS ${APP_OBJ})
add_dependencies(${COMPONENT_LIB} tulpa_app)
target_link_libraries(${COMPONENT_LIB} PRIVATE ${APP_OBJ} m)
# Programs outside the static native subset run on the Tulpa VM (tulpa_dyn.c); let it use most
# of the main task's stack (CONFIG_ESP_MAIN_TASK_STACK_SIZE in sdkconfig.defaults).
target_compile_definitions(${COMPONENT_LIB} PRIVATE TULPA_DYN_C_STACK=44000)

# ESP32-family firmware written in Tulpa (ESP-IDF 5.x).
#   idf.py set-target @TARGET@ && idf.py build flash monitor
cmake_minimum_required(VERSION 3.16)
include($ENV{IDF_PATH}/tools/cmake/project.cmake)
project(@NAME@)

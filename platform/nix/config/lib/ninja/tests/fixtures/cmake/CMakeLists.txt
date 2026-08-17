cmake_minimum_required(VERSION 3.13)
project(greet C)
add_executable(greet src/main.c src/greet.c)
target_include_directories(greet PRIVATE src/include)

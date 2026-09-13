cmake_minimum_required(VERSION 3.16)
set(PROJECT_NAME vicinae-fuzzy)
project(${PROJECT_NAME})
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(BUILD_TESTS "Build the test suite" OFF)

add_library(${PROJECT_NAME} INTERFACE)
add_library(vicinae::fuzzy ALIAS ${PROJECT_NAME})
target_include_directories(${PROJECT_NAME} INTERFACE ${CMAKE_CURRENT_SOURCE_DIR}/include)
target_compile_features(${PROJECT_NAME} INTERFACE cxx_std_23)

# Suite 0 rung 1 (docs/rust-engine/PLAN.md §8.1a). Off by default: nothing that
# ships needs it, and CI compiles it directly with `c++ -std=c++23` rather than
# through CMake, because the whole point is that this library needs no build
# system, no Qt and no server. The target exists so the probe is buildable the
# ordinary way too, and so it does not rot unnoticed.
option(BUILD_FUZZY_PROBE "Build the scorer probe used by Suite 0 rung 1" OFF)

if (BUILD_FUZZY_PROBE)
	add_executable(vicinae-fuzzy-probe probe/main.cpp)
	target_link_libraries(vicinae-fuzzy-probe PRIVATE ${PROJECT_NAME})
	target_compile_features(vicinae-fuzzy-probe PRIVATE cxx_std_23)
endif()

if (BUILD_TESTS)
	set(TEST_TARGET ${PROJECT_NAME}-tests)
	find_package(Catch2 3 REQUIRED)
	add_executable(${TEST_TARGET} tests/main.cpp)
	target_link_libraries(${TEST_TARGET} PRIVATE Catch2::Catch2WithMain ${PROJECT_NAME})
endif()

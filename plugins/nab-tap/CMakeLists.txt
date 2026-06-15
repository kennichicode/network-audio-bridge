cmake_minimum_required(VERSION 3.22)

project(NABTap VERSION 0.1.0)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

set(NAB_JUCE_PATH
    "/Users/kenichikawabata/Documents/Claude/StageDAW-Recorder/JUCE"
    CACHE PATH "Path to a local JUCE checkout")

if(NOT EXISTS "${NAB_JUCE_PATH}/CMakeLists.txt")
    message(FATAL_ERROR "JUCE not found at NAB_JUCE_PATH=${NAB_JUCE_PATH}")
endif()

add_subdirectory("${NAB_JUCE_PATH}" "${CMAKE_BINARY_DIR}/JUCE")

juce_add_plugin(NABTap
    COMPANY_NAME "Kenichi Kawabata"
    BUNDLE_ID "com.kenichikawabata.nabtap"
    IS_SYNTH FALSE
    NEEDS_MIDI_INPUT FALSE
    NEEDS_MIDI_OUTPUT FALSE
    IS_MIDI_EFFECT FALSE
    COPY_PLUGIN_AFTER_BUILD FALSE
    PLUGIN_MANUFACTURER_CODE Keni
    PLUGIN_CODE NbTp
    FORMATS VST3
    PRODUCT_NAME "NAB Tap")

juce_generate_juce_header(NABTap)

target_sources(NABTap
    PRIVATE
        Source/PluginEditor.cpp
        Source/PluginProcessor.cpp)

target_compile_definitions(NABTap
    PUBLIC
        JUCE_WEB_BROWSER=0
        JUCE_USE_CURL=0
        JUCE_VST3_CAN_REPLACE_VST2=0)

target_link_libraries(NABTap
    PRIVATE
        juce::juce_audio_utils)

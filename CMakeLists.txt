cmake_minimum_required(VERSION 3.22)
project(AudioZero VERSION 0.1.0 LANGUAGES C CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
if(APPLE)
    set(CMAKE_OSX_DEPLOYMENT_TARGET "11.0" CACHE STRING "Minimum macOS version")
endif()

include(FetchContent)
FetchContent_Declare(juce GIT_REPOSITORY https://github.com/juce-framework/JUCE.git GIT_TAG 8.0.10 GIT_SHALLOW TRUE)
FetchContent_MakeAvailable(juce)

option(AUDIOZERO_BUILD_TESTS "Build AudioZero tests" OFF)
option(AUDIOZERO_BUILD_OPERATOR_PLUGIN "Build the Operator synth as a standalone VST3 plugin" ON)

# Engine, model, DSP and the editors the built-in devices need.
set(AUDIOZERO_CORE_SOURCES
    src/model/Project.cpp
    src/model/ProjectIO.cpp
    src/engine/AudioEngine.cpp
    src/dsp/OperatorSynth.cpp
    src/dsp/BuiltinEffects.cpp
    src/plugins/PluginManager.cpp
    src/ui/Theme.cpp
    src/ui/OperatorEditor.cpp)
list(TRANSFORM AUDIOZERO_CORE_SOURCES PREPEND "${CMAKE_CURRENT_SOURCE_DIR}/" OUTPUT_VARIABLE AUDIOZERO_CORE_SOURCES_ABS)

set(AUDIOZERO_DEFINITIONS
    JUCE_WEB_BROWSER=0
    JUCE_USE_CURL=0
    JUCE_PLUGINHOST_VST3=1
    JUCE_PLUGINHOST_AU=0
    JUCE_PLUGINHOST_LADSPA=0
    JUCE_USE_MP3AUDIOFORMAT=1
    JUCE_DISPLAY_SPLASH_SCREEN=0)

set(AUDIOZERO_MODULES
    juce::juce_audio_utils
    juce::juce_audio_processors
    juce::juce_audio_formats
    juce::juce_audio_devices
    juce::juce_dsp
    juce::juce_gui_extra)

juce_add_binary_data(AudioZeroData SOURCES resources/icon.png)

juce_add_gui_app(AudioZero
    PRODUCT_NAME "AudioZero"
    COMPANY_NAME "NOISYLOOP"
    BUNDLE_ID "com.noisyloop.audiozero"
    ICON_BIG resources/icon.png
    DOCUMENT_EXTENSIONS azproj
    MICROPHONE_PERMISSION_ENABLED TRUE
    MICROPHONE_PERMISSION_TEXT "AudioZero uses audio input for recording.")

target_sources(AudioZero PRIVATE
    ${AUDIOZERO_CORE_SOURCES}
    src/Main.cpp
    src/ui/Session.cpp
    src/ui/MainComponent.cpp
    src/ui/TransportBar.cpp
    src/ui/Browser.cpp
    src/ui/ArrangementView.cpp
    src/ui/PianoRoll.cpp
    src/ui/DevicePanel.cpp
    src/ui/MixerView.cpp)
target_compile_definitions(AudioZero PRIVATE ${AUDIOZERO_DEFINITIONS}
    JUCE_APPLICATION_NAME_STRING="$<TARGET_PROPERTY:AudioZero,JUCE_PRODUCT_NAME>"
    JUCE_APPLICATION_VERSION_STRING="$<TARGET_PROPERTY:AudioZero,JUCE_VERSION>")
target_link_libraries(AudioZero PRIVATE AudioZeroData ${AUDIOZERO_MODULES}
    juce::juce_recommended_config_flags juce::juce_recommended_lto_flags juce::juce_recommended_warning_flags)

# The built-in Operator synth, also shipped as a VST3 for other DAWs.
if(AUDIOZERO_BUILD_OPERATOR_PLUGIN)
    juce_add_plugin(AudioZeroOperator
        COMPANY_NAME "NOISYLOOP"
        BUNDLE_ID "com.noisyloop.audiozero.operator"
        PLUGIN_MANUFACTURER_CODE Nzlp
        PLUGIN_CODE Azop
        FORMATS VST3 Standalone
        PRODUCT_NAME "AudioZero Operator"
        IS_SYNTH TRUE
        NEEDS_MIDI_INPUT TRUE
        VST3_CATEGORIES Instrument Synth
        ICON_BIG resources/icon.png
        COPY_PLUGIN_AFTER_BUILD FALSE)
    target_sources(AudioZeroOperator PRIVATE
        src/plugins/OperatorPlugin.cpp
        src/dsp/OperatorSynth.cpp
        src/ui/Theme.cpp
        src/ui/OperatorEditor.cpp)
    target_compile_definitions(AudioZeroOperator PUBLIC JUCE_WEB_BROWSER=0 JUCE_USE_CURL=0 JUCE_VST3_CAN_REPLACE_VST2=0 JUCE_DISPLAY_SPLASH_SCREEN=0)
    target_link_libraries(AudioZeroOperator PRIVATE juce::juce_audio_utils
        juce::juce_recommended_config_flags juce::juce_recommended_warning_flags)
endif()

if(AUDIOZERO_BUILD_TESTS)
    enable_testing()
    add_subdirectory(tests)
endif()

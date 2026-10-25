# Engine / DSP / project tests. Run headless on every platform.
juce_add_console_app(AudioZeroCoreTests PRODUCT_NAME "AudioZeroCoreTests")
target_sources(AudioZeroCoreTests PRIVATE CoreTests.cpp
    ${AUDIOZERO_CORE_SOURCES_ABS})
target_compile_definitions(AudioZeroCoreTests PRIVATE ${AUDIOZERO_DEFINITIONS})
target_link_libraries(AudioZeroCoreTests PRIVATE ${AUDIOZERO_MODULES} juce::juce_recommended_config_flags)
add_test(NAME CoreTests COMMAND AudioZeroCoreTests)
set_tests_properties(CoreTests PROPERTIES TIMEOUT 120)

# Loads the Operator VST3 through AudioZero's own plugin host (scan, instantiate, play, save, reload).
if(TARGET AudioZeroOperator_VST3)
    add_dependencies(AudioZeroCoreTests AudioZeroOperator_VST3)
    add_test(NAME OperatorVST3Hosting
             COMMAND AudioZeroCoreTests --vst3 "$<TARGET_FILE_DIR:AudioZeroOperator>/VST3/AudioZero Operator.vst3")
    set_tests_properties(OperatorVST3Hosting PROPERTIES TIMEOUT 120)
endif()

# VST3 hosting smoke test: builds a tiny VST3 and loads it through JUCE's VST3 host.
if(APPLE)
    juce_add_plugin(AudioZeroSmokeSynth
        COMPANY_NAME "NOISYLOOP"
        BUNDLE_ID "com.noisyloop.audiozero.smokesynth"
        PLUGIN_MANUFACTURER_CODE Nzlp
        PLUGIN_CODE Azsm
        FORMATS VST3
        JUCE_VST3_CAN_REPLACE_VST2 FALSE
        PRODUCT_NAME "AudioZeroSmokeSynth"
        IS_SYNTH TRUE
        NEEDS_MIDI_INPUT TRUE
        COPY_PLUGIN_AFTER_BUILD FALSE)
    target_sources(AudioZeroSmokeSynth PRIVATE SmokeSynth.cpp)
    target_compile_definitions(AudioZeroSmokeSynth PRIVATE JUCE_VST3_CAN_REPLACE_VST2=0)
    target_link_libraries(AudioZeroSmokeSynth PRIVATE juce::juce_audio_utils juce::juce_recommended_config_flags)

    juce_add_console_app(AudioZeroHostSmoke PRODUCT_NAME "AudioZeroHostSmoke")
    target_sources(AudioZeroHostSmoke PRIVATE HostSmoke.cpp)
    target_compile_definitions(AudioZeroHostSmoke PRIVATE JUCE_PLUGINHOST_VST3=1 JUCE_WEB_BROWSER=0 JUCE_USE_CURL=0)
    target_link_libraries(AudioZeroHostSmoke PRIVATE juce::juce_audio_utils juce::juce_audio_processors juce::juce_recommended_config_flags)

    add_dependencies(AudioZeroHostSmoke AudioZeroSmokeSynth_VST3)
    add_test(NAME VST3HostSmoke COMMAND AudioZeroHostSmoke "$<TARGET_BUNDLE_DIR:AudioZeroSmokeSynth_VST3>")
    set_tests_properties(VST3HostSmoke PROPERTIES TIMEOUT 90)
endif()

juce_add_plugin(AudioZeroSmokeSynth
    COMPANY_NAME "NOISYLOOP"
    BUNDLE_ID "com.noisyloop.audiozero.smokesynth"
    PLUGIN_MANUFACTURER_CODE Nzlp
    PLUGIN_CODE Azsm
    FORMATS VST3
    PRODUCT_NAME "AudioZeroSmokeSynth"
    IS_SYNTH TRUE
    NEEDS_MIDI_INPUT TRUE
    COPY_PLUGIN_AFTER_BUILD FALSE)
target_sources(AudioZeroSmokeSynth PRIVATE SmokeSynth.cpp)
target_link_libraries(AudioZeroSmokeSynth PRIVATE juce::juce_audio_utils juce::juce_recommended_config_flags)

juce_add_console_app(AudioZeroHostSmoke PRODUCT_NAME "AudioZeroHostSmoke")
target_sources(AudioZeroHostSmoke PRIVATE HostSmoke.cpp)
target_compile_definitions(AudioZeroHostSmoke PRIVATE JUCE_PLUGINHOST_VST3=1 JUCE_WEB_BROWSER=0 JUCE_USE_CURL=0)
target_link_libraries(AudioZeroHostSmoke PRIVATE juce::juce_audio_utils juce::juce_audio_processors juce::juce_recommended_config_flags)

add_dependencies(AudioZeroHostSmoke AudioZeroSmokeSynth_VST3)
add_test(NAME VST3HostSmoke COMMAND AudioZeroHostSmoke "$<TARGET_FILE:AudioZeroSmokeSynth_VST3>")
set_tests_properties(VST3HostSmoke PROPERTIES TIMEOUT 90)

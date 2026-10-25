#include <juce_audio_processors/juce_audio_processors.h>
#include <cmath>
#include <iostream>

int main(int argc, char** argv)
{
    juce::ScopedJuceInitialiser_GUI juceInit;
    if (argc != 2)
    {
        std::cerr << "Usage: AudioZeroHostSmoke /path/to/SmokeSynth.vst3\n";
        return 2;
    }
    juce::VST3PluginFormat format;
    juce::OwnedArray<juce::PluginDescription> descriptions;
    format.findAllTypesForFile(descriptions, juce::String::fromUTF8(argv[1]));
    if (descriptions.isEmpty())
    {
        std::cerr << "FAIL: VST3 discovery\n";
        return 3;
    }
    juce::AudioPluginFormatManager manager;
    manager.addDefaultFormats();
    juce::String error;
    auto plugin = manager.createPluginInstance(*descriptions[0], 48000.0, 256, error);
    if (!plugin)
    {
        std::cerr << "FAIL: VST3 load: " << error << "\n";
        return 4;
    }
    plugin->setPlayConfigDetails(0, 2, 48000.0, 256);
    plugin->prepareToPlay(48000.0, 256);
    juce::AudioBuffer<float> audio(2, 256);
    audio.clear();
    juce::MidiBuffer midi;
    midi.addEvent(juce::MidiMessage::noteOn(1, 60, (juce::uint8)100), 0);
    plugin->processBlock(audio, midi);
    float peak = 0.0f;
    for (int ch = 0; ch < audio.getNumChannels(); ++ch)
        for (int i = 0; i < audio.getNumSamples(); ++i)
            peak = std::max(peak, std::abs(audio.getSample(ch, i)));
    if (peak < 0.01f)
    {
        std::cerr << "FAIL: MIDI note produced no audio\n";
        return 5;
    }
    juce::MemoryBlock saved;
    plugin->getStateInformation(saved);
    if (saved.isEmpty())
    {
        std::cerr << "FAIL: empty plugin state\n";
        return 6;
    }
    juce::MidiBuffer noteOff;
    noteOff.addEvent(juce::MidiMessage::noteOff(1, 60), 0);
    plugin->processBlock(audio, noteOff);
    plugin->setStateInformation(saved.getData(), static_cast<int>(saved.getSize()));
    audio.clear();
    juce::MidiBuffer empty;
    plugin->processBlock(audio, empty);
    if (std::abs(audio.getSample(0, 0)) < 0.01f)
    {
        std::cerr << "FAIL: state restore did not restore voice\n";
        return 7;
    }
    plugin->releaseResources();
    std::cout << "PASS: VST3 discovered, loaded, MIDI processed, audio produced, state restored\n";
    return 0;
}

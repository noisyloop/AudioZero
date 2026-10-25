#pragma once
#include "Project.h"

namespace az
{

class AudioEngine;
class PluginManager;

namespace io
{
    // Project files are XML ("*.azproj"). Plugin states are stored as base64. Audio generated in
    // memory is written next to the project in "<name> Samples/".
    bool save(Project& project, const juce::File& file, juce::String& error);

    // Builds tracks from a file. Returns them without touching the project, so the caller can
    // swap them in safely. Problems that do not stop loading (missing plugins/samples) go to warnings.
    struct Loaded
    {
        std::vector<std::shared_ptr<Track>> tracks;
        double bpm = 120.0, loopStart = 0.0, loopEnd = 16.0;
        int beatsPerBar = 4;
        bool loopEnabled = true;
        float masterVolume = 0.85f;
    };
    bool load(const juce::File& file, Project& idSource, PluginManager& plugins, AudioEngine& engine,
              Loaded& result, juce::String& error, juce::StringArray& warnings);

    // Starting points.
    Loaded createEmpty(Project& idSource, PluginManager& plugins, AudioEngine& engine);
    Loaded createDemo(Project& idSource, PluginManager& plugins, AudioEngine& engine);

    // Replaces the project's content (takes the project lock while swapping).
    void install(Project& project, Loaded&& loaded, const juce::File& file);

    // Helpers shared with the UI.
    std::shared_ptr<Track> makeTrack(Project& idSource, const juce::String& name, juce::Colour colour);
    Clip makeMidiClip(Project& idSource, double start, double length, juce::Colour colour, const juce::String& name = "MIDI Clip");
    Clip makeAudioClip(Project& idSource, std::shared_ptr<const AudioSample> sample, double start, double bpm, juce::Colour colour);
    bool importMidiFile(const juce::File& file, Clip& clip, juce::String& error);
}

} // namespace az

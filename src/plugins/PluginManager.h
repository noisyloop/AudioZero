#pragma once
#include "../model/Project.h"

namespace az
{

// Owns plugin formats and the list of known (scanned) plugins, and creates devices.
class PluginManager final : public juce::ChangeBroadcaster, private juce::Timer
{
public:
    PluginManager();
    ~PluginManager() override;

    juce::AudioPluginFormatManager formats;
    juce::KnownPluginList knownPlugins;

    // Scans the standard VST3 folders (plus any extra ones) a file at a time on the message thread.
    void startScan(bool rescanAll = false);
    bool isScanning() const { return scanner != nullptr; }
    juce::String scanStatus() const { return status; }
    juce::StringArray extraFolders;

    std::unique_ptr<Device> createBuiltin(const juce::String& id) const;
    std::unique_ptr<Device> createPlugin(const juce::PluginDescription& description, double sampleRate, int blockSize, juce::String& error);
    // Loads the first plugin found in a .vst3 bundle dropped from the file system.
    std::unique_ptr<Device> createFromFile(const juce::File& file, double sampleRate, int blockSize, juce::String& error);
    const juce::PluginDescription* findByIdentifier(const juce::String& identifier) const;

    static bool isPluginFile(const juce::File& file);
    static juce::File settingsFolder();

private:
    void timerCallback() override;
    void saveList();
    void loadList();

    std::unique_ptr<juce::PluginDirectoryScanner> scanner;
    juce::String status;
    std::vector<juce::PluginDescription> cache;
};

} // namespace az

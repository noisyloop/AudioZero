#include <juce_audio_utils/juce_audio_utils.h>
#include <juce_audio_processors/juce_audio_processors.h>
#include <juce_gui_extra/juce_gui_extra.h>

class PluginWindow final : public juce::DocumentWindow
{
public:
    explicit PluginWindow(juce::AudioProcessor& processor)
        : DocumentWindow("AudioZero — Plugin Editor", juce::Colours::darkgrey, allButtons)
    {
        if (auto* editor = processor.createEditorIfNeeded())
            setContentOwned(editor, true);
        else
            setContentOwned(new juce::GenericAudioProcessorEditor(processor), true);
        centreWithSize(getWidth(), getHeight());
        setResizable(true, true);
        setVisible(true);
    }
    void closeButtonPressed() override { setVisible(false); }
};

class HostComponent final : public juce::Component
{
public:
    HostComponent()
    {
        formatManager.addDefaultFormats();
        auto result = deviceManager.initialise(0, 2, nullptr, true);
        status.setText(result.isEmpty() ? "Audio ready — choose a VST3 plugin" : "Audio device error: " + result,
                       juce::dontSendNotification);
        deviceManager.addAudioCallback(&player);
        deviceManager.addMidiInputDeviceCallback({}, &player);
        for (auto* button : { &loadButton, &editorButton, &saveButton, &restoreButton })
            addAndMakeVisible(button);
        addAndMakeVisible(status);
        loadButton.setButtonText("Load VST3");
        editorButton.setButtonText("Open editor");
        saveButton.setButtonText("Save state");
        restoreButton.setButtonText("Restore state");
        loadButton.onClick = [this] { choosePlugin(); };
        editorButton.onClick = [this] { if (plugin) { if (!pluginWindow) pluginWindow = std::make_unique<PluginWindow>(*plugin); else pluginWindow->setVisible(true); } };
        saveButton.onClick = [this] { saveState(); };
        restoreButton.onClick = [this] { restoreState(); };
        setSize(680, 180);
    }

    ~HostComponent() override
    {
        chooser.reset();
        pluginWindow.reset();
        player.setProcessor(nullptr);
        deviceManager.removeMidiInputDeviceCallback({}, &player);
        deviceManager.removeAudioCallback(&player);
        plugin.reset();
    }

    void resized() override
    {
        auto area = getLocalBounds().reduced(16);
        auto buttons = area.removeFromTop(42);
        for (auto* button : { &loadButton, &editorButton, &saveButton, &restoreButton })
            button->setBounds(buttons.removeFromLeft(160).reduced(4));
        status.setBounds(area.removeFromTop(70));
    }

private:
    void choosePlugin()
    {
        chooser = std::make_unique<juce::FileChooser>("Choose a VST3 bundle", juce::File{}, "*.vst3");
        auto safe = juce::Component::SafePointer<HostComponent>(this);
        chooser->launchAsync(juce::FileBrowserComponent::openMode
                                 | juce::FileBrowserComponent::canSelectFiles
                                 | juce::FileBrowserComponent::canSelectDirectories,
                             [safe](const juce::FileChooser& picker)
                             {
                                 if (safe != nullptr && picker.getResult().exists())
                                     safe->loadPlugin(picker.getResult());
                             });
    }

    void loadPlugin(const juce::File& path)
    {
        juce::VST3PluginFormat vst3;
        juce::OwnedArray<juce::PluginDescription> descriptions;
        vst3.findAllTypesForFile(descriptions, path.getFullPathName());
        if (descriptions.isEmpty())
        {
            status.setText("No VST3 plugin found in: " + path.getFullPathName(), juce::dontSendNotification);
            return;
        }
        juce::String error;
        auto sampleRate = deviceManager.getCurrentAudioDevice() != nullptr
                            ? deviceManager.getCurrentAudioDevice()->getCurrentSampleRate() : 44100.0;
        auto next = formatManager.createPluginInstance(*descriptions[0], sampleRate, 512, error);
        if (!next)
        {
            status.setText("Plugin load failed: " + error, juce::dontSendNotification);
            return;
        }
        pluginWindow.reset();
        player.setProcessor(nullptr);
        plugin = std::move(next);
        plugin->enableAllBuses();
        player.setProcessor(plugin.get());
        status.setText("Loaded: " + plugin->getName() + " | " + path.getFileName(), juce::dontSendNotification);
        pluginWindow = std::make_unique<PluginWindow>(*plugin);
    }

    void saveState()
    {
        if (!plugin) return;
        juce::MemoryBlock state;
        plugin->getStateInformation(state);
        auto target = juce::File::getSpecialLocation(juce::File::userDocumentsDirectory)
                          .getChildFile("AudioZero-plugin-state.bin");
        status.setText(target.replaceWithData(state.getData(), state.getSize())
                           ? "Saved state: " + target.getFullPathName() : "Failed to save state",
                       juce::dontSendNotification);
    }

    void restoreState()
    {
        if (!plugin) return;
        auto source = juce::File::getSpecialLocation(juce::File::userDocumentsDirectory)
                          .getChildFile("AudioZero-plugin-state.bin");
        juce::MemoryBlock state;
        if (!source.loadFileAsData(state))
        {
            status.setText("No saved state found", juce::dontSendNotification);
            return;
        }
        plugin->setStateInformation(state.getData(), static_cast<int>(state.getSize()));
        status.setText("Restored plugin state", juce::dontSendNotification);
    }

    juce::AudioDeviceManager deviceManager;
    juce::AudioProcessorPlayer player;
    juce::AudioPluginFormatManager formatManager;
    std::unique_ptr<juce::AudioPluginInstance> plugin;
    std::unique_ptr<PluginWindow> pluginWindow;
    std::unique_ptr<juce::FileChooser> chooser;
    juce::TextButton loadButton, editorButton, saveButton, restoreButton;
    juce::Label status;
};

class AudioZeroApplication final : public juce::JUCEApplication
{
public:
    const juce::String getApplicationName() override { return "AudioZero"; }
    const juce::String getApplicationVersion() override { return "0.0.1"; }
    void initialise(const juce::String&) override
    {
        mainWindow = std::make_unique<juce::DocumentWindow>("AudioZero — VST3 Host Spike",
                                                            juce::Colours::black,
                                                            juce::DocumentWindow::allButtons);
        mainWindow->setUsingNativeTitleBar(true);
        mainWindow->setContentOwned(new HostComponent(), true);
        mainWindow->centreWithSize(mainWindow->getWidth(), mainWindow->getHeight());
        mainWindow->setVisible(true);
    }
    void shutdown() override { mainWindow.reset(); }
    void systemRequestedQuit() override { quit(); }
private:
    std::unique_ptr<juce::DocumentWindow> mainWindow;
};
START_JUCE_APPLICATION(AudioZeroApplication)

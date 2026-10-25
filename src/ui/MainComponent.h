#pragma once
#include "Session.h"
#include "TransportBar.h"
#include "Browser.h"
#include "ArrangementView.h"
#include "PianoRoll.h"
#include "DevicePanel.h"
#include "MixerView.h"

namespace az
{

class MainComponent final : public juce::Component,
                            public juce::DragAndDropContainer,
                            public juce::MenuBarModel,
                            private juce::Timer,
                            private juce::FocusChangeListener
{
public:
    // startupFile: project to open; empty = last project, or the demo song on first run.
    explicit MainComponent(const juce::File& startupFile = {}, bool forceDemo = false);
    ~MainComponent() override;

    void paint(juce::Graphics&) override;
    void resized() override;
    bool keyPressed(const juce::KeyPress&) override;
    bool keyStateChanged(bool isKeyDown) override;

    juce::StringArray getMenuBarNames() override;
    juce::PopupMenu getMenuForIndex(int index, const juce::String& name) override;
    void menuItemSelected(int itemId, int menuIndex) override;

    enum class Tab { devices, pianoRoll, mixer };
    void showTab(Tab);
    void requestQuit(std::function<void()> quit);
    juce::String windowTitle() const;
    std::function<void()> onTitleChanged;

    Session session;

private:
    class Divider;
    enum Command
    {
        cmdNew = 1, cmdOpen, cmdOpenDemo, cmdSave, cmdSaveAs, cmdExport, cmdQuit,
        cmdUndo, cmdRedo, cmdDelete, cmdDuplicate, cmdCopy, cmdPaste, cmdSplit, cmdSelectAll,
        cmdAddInstrumentTrack, cmdAddAudioTrack, cmdAddMidiClip, cmdImport,
        cmdAudioSettings, cmdScanPlugins, cmdKeyboard, cmdMetronome, cmdLoop,
        cmdShortcuts, cmdAbout
    };
    void perform(int command);
    void timerCallback() override;
    void globalFocusChanged(juce::Component* focused) override;
    void loadProject(const juce::File& file);
    void saveProject(bool saveAs, std::function<void()> then = {});
    void exportAudio();
    void confirmDiscard(std::function<void()> then);
    void setStatus(const juce::String& text);
    void rememberProject(const juce::File& file);
    static juce::PropertiesFile& settings();

    TransportBar transport { session };
    Browser browser { session };
    ArrangementView arrangement { session };
    DevicePanel devicePanel { session };
    PianoRoll pianoRoll { session };
    MixerView mixer { session };
    juce::TextButton devicesTab { "Devices" }, pianoRollTab { "Piano Roll" }, mixerTab { "Mixer" };
    std::unique_ptr<Divider> browserDivider, bottomDivider;
    juce::TooltipWindow tooltips { this, 700 };
    Tab tab = Tab::devices;
    int browserWidth = 240, bottomHeight = 296;
    juce::String statusText;
    juce::Rectangle<int> statusArea, bottomHeader;
    std::unique_ptr<juce::FileChooser> chooser;
    std::set<int> heldKeys;
    int64_t lastVersion = -1;
    bool lastDirty = false;
};

} // namespace az

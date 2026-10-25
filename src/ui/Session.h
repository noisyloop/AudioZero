#pragma once
#include "../engine/AudioEngine.h"
#include "../plugins/PluginManager.h"
#include "../model/ProjectIO.h"
#include <juce_audio_utils/juce_audio_utils.h>

namespace az
{

// Application state shared by all views: the project, engine, devices, selection and the
// editing operations views perform. Lives on the message thread.
class Session final : public juce::ChangeBroadcaster
{
public:
    Session();
    ~Session() override;

    Project project;
    AudioEngine engine { project };
    PluginManager plugins;
    juce::AudioDeviceManager deviceManager;
    juce::String audioError;

    struct Listener
    {
        virtual ~Listener() = default;
        virtual void devicesAboutToChange() {}   // destroy any embedded editors now
        virtual void devicesChanged() {}
    };
    void addListener(Listener* l) { listeners.add(l); }
    void removeListener(Listener* l) { listeners.remove(l); }

    // Selection / view state
    int selectedTrackId = 0;
    int selectedClipId = 0;
    int editClipId = 0;                  // clip shown in the piano roll
    double snapBeats = 1.0;              // arrangement grid; 0 = off
    std::function<void()> onEditClip;     // switch to the piano roll
    std::function<void(const juce::String&)> onStatus;

    void selectTrack(int trackId);
    void selectClip(int clipId);
    void editClip(int clipId);
    std::shared_ptr<Track> selectedTrack() const { return project.findTrack(selectedTrackId); }
    Clip* findClip(int clipId) const;
    double snap(double beat, double grid) const { return grid > 0 ? std::round(beat / grid) * grid : beat; }
    double snap(double beat) const { return snap(beat, snapBeats); }
    void status(const juce::String& text) { if (onStatus) onStatus(text); }

    // Replaces the whole project (new / open / demo).
    void install(io::Loaded&& loaded, const juce::File& file);

    // Tracks
    std::shared_ptr<Track> addTrack(bool withOperator, int insertIndex = -1, const juce::String& name = {});
    void deleteTrack(int trackId);
    void duplicateTrack(int trackId);
    void moveTrack(int trackId, int delta);

    // Clips
    int createMidiClip(int trackId, double beat, double length);
    void deleteClip(int clipId);
    int duplicateClip(int clipId);
    void splitClip(int clipId, double beat);
    void copyClip(int clipId);
    void pasteClip(int trackId, double beat);
    bool hasClipboard() const { return clipboard.has_value(); }

    // Devices
    bool addDevice(int trackId, std::unique_ptr<Device> device, bool asInstrument);
    void removeDevice(int trackId, Device* device);
    void showEditorWindow(Device& device);
    void closeEditorWindows(Device* device = nullptr);
    std::unique_ptr<Device> createDeviceFromItem(const juce::String& item, juce::String& error);
    static bool isInstrumentDevice(const Device& device);

    // Drag & drop / double-click from the browser ("builtin:x", "preset:x", "plugin:x", "file:x").
    // trackId < 0 creates a new track. Returns true if anything happened.
    bool applyItem(const juce::String& item, int trackId, double beat);
    bool importFiles(const juce::StringArray& files, int trackId, double beat);
    static bool isInterestingFile(const juce::String& path);

    // Live notes for auditioning from the piano roll / keyboard.
    void noteOn(int pitch, float velocity) { engine.addLiveMidi(juce::MidiMessage::noteOn(1, pitch, velocity)); }
    void noteOff(int pitch) { engine.addLiveMidi(juce::MidiMessage::noteOff(1, pitch)); }

    // Moves notes captured by the engine into clips. Call regularly from a timer.
    void updateRecording();
    void toggleRecord();

    // Edit helper: checkpoint for undo, mutate under the project lock, notify.
    template <typename Fn>
    void edit(Fn&& fn, bool undoable = true)
    {
        if (undoable)
            project.checkpoint();
        {
            const juce::ScopedLock sl(project.lock);
            fn();
        }
        project.changed();
    }

private:
    struct OpenNote { double start; float velocity; };
    struct Recording { int clipId = 0; std::map<int, OpenNote> open; };
    std::map<int, Recording> recordings; // per track id
    bool wasRecording = false;
    std::vector<AudioEngine::RecordedEvent> recordedEvents;

    class EditorWindow;
    std::vector<std::unique_ptr<EditorWindow>> editorWindows;
    juce::ListenerList<Listener> listeners;
    std::optional<Clip> clipboard;
};

} // namespace az

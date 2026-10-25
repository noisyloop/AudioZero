#pragma once
#include <juce_audio_processors/juce_audio_processors.h>
#include <juce_audio_formats/juce_audio_formats.h>
#include <array>
#include <atomic>
#include <functional>
#include <memory>
#include <vector>

namespace az
{

// A MIDI note inside a clip. Positions are in beats (quarter notes) relative to the clip start.
struct Note
{
    int pitch = 60;
    double start = 0.0;
    double length = 1.0;
    float velocity = 0.8f;
};

// Immutable decoded audio, shared between clips that use the same file.
struct AudioSample
{
    juce::String name;
    juce::File file;                       // empty for audio generated in memory
    juce::AudioBuffer<float> data;
    double sampleRate = 44100.0;
    std::vector<float> peaks;              // max |amplitude| per peakBlock frames, for waveform drawing
    static constexpr int peakBlock = 256;

    double lengthSeconds() const { return data.getNumSamples() / sampleRate; }
    void computePeaks();

    static std::shared_ptr<AudioSample> fromBuffer(const juce::String& name, juce::AudioBuffer<float> buffer, double sampleRate);
    static std::shared_ptr<AudioSample> load(const juce::File& file, juce::String& error);
    static juce::AudioFormatManager& formats();
    static bool isAudioFile(const juce::File& file);
};

enum class ClipKind { midi, audio };

struct Clip
{
    int id = 0;
    ClipKind kind = ClipKind::midi;
    juce::String name;
    juce::Colour colour;
    double start = 0.0;     // beats
    double length = 4.0;    // beats
    bool muted = false;

    std::vector<Note> notes;                 // midi clips

    std::shared_ptr<const AudioSample> sample; // audio clips
    double sampleOffset = 0.0;               // seconds into the sample at clip start
    float gain = 1.0f;

    double end() const { return start + length; }
};

// A processor living in a track's instrument slot or effect chain.
struct Device
{
    std::unique_ptr<juce::AudioProcessor> processor;
    juce::String builtinId;                  // "operator", "reverb", "delay" or empty for plugins
    juce::PluginDescription description;     // valid for plugins
    bool bypassed = false;
    juce::AudioBuffer<float> scratch;        // multi-channel work buffer, allocated in prepare

    bool isBuiltin() const { return builtinId.isNotEmpty(); }
    juce::String displayName() const { return processor != nullptr ? processor->getName() : juce::String("Empty"); }
};

// Everything about a track that the undo system can snapshot.
struct TrackData
{
    juce::String name;
    juce::Colour colour;
    float volume = 0.8f;   // linear gain
    float pan = 0.0f;      // -1 .. 1
    bool mute = false, solo = false, arm = false;
    std::vector<Clip> clips;
};

struct Track
{
    int id = 0;
    TrackData data;
    std::unique_ptr<Device> instrument;
    std::vector<std::unique_ptr<Device>> effects;

    // runtime state owned by the audio engine
    juce::AudioBuffer<float> buffer;
    juce::MidiBuffer midi;
    std::array<bool, 128> clipNotesOn {};
    std::atomic<float> meterL { 0.0f }, meterR { 0.0f };
    double preparedRate = 0.0;
    int preparedBlock = 0;

    Clip* findClip(int clipId);
};

// The whole song. The UI thread is the only writer and must hold `lock` while mutating
// anything the audio thread reads; the audio thread holds it for each processed block.
class Project : public juce::ChangeBroadcaster
{
public:
    Project();

    juce::CriticalSection lock;

    double bpm = 120.0;
    int beatsPerBar = 4;
    bool loopEnabled = true;
    double loopStart = 0.0, loopEnd = 16.0;
    float masterVolume = 0.85f;
    std::atomic<float> masterMeterL { 0.0f }, masterMeterR { 0.0f };

    std::vector<std::shared_ptr<Track>> tracks;

    int newId() { return nextId++; }
    std::shared_ptr<Track> findTrack(int trackId) const;
    std::shared_ptr<Track> trackOwningClip(int clipId) const;
    int indexOfTrack(int trackId) const;
    double songEndBeats() const;
    juce::Colour nextTrackColour();

    // Call after any edit: tells views to refresh and the engine to release hanging notes.
    void changed();
    bool consumeNoteFlush() { return flushNotes.exchange(false); }

    // Undo: call checkpoint() *before* making an edit.
    void checkpoint();
    bool undo();
    bool redo();
    bool canUndo() const { return ! undoStack.empty(); }
    bool canRedo() const { return ! redoStack.empty(); }
    void clearHistory() { undoStack.clear(); redoStack.clear(); }

    int64_t version() const { return editVersion; }
    int64_t savedVersion = 0;
    bool isDirty() const { return editVersion != savedVersion; }

    juce::File file;

    // Set by the engine so tracks resurrected by undo get prepared before they are played.
    std::function<void(Track&)> prepareTrack;

private:
    struct Snapshot
    {
        std::vector<std::pair<std::shared_ptr<Track>, TrackData>> tracks;
        double bpm, loopStart, loopEnd;
        int beatsPerBar;
        bool loopEnabled;
    };
    Snapshot capture() const;
    void restore(const Snapshot&);

    std::vector<Snapshot> undoStack, redoStack;
    int nextId = 1;
    int colourIndex = 0;
    int64_t editVersion = 0;
    std::atomic<bool> flushNotes { false };
};

namespace palette
{
    juce::Colour trackColour(int index);
}

} // namespace az

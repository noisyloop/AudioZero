#pragma once
#include "../model/Project.h"
#include <juce_audio_devices/juce_audio_devices.h>

namespace az
{

// Renders the arrangement: generates MIDI from clips, runs each track's instrument and
// effect chain, mixes, meters and drives the transport. Usable with or without a device.
class AudioEngine final : public juce::AudioIODeviceCallback,
                          public juce::MidiInputCallback,
                          public juce::AudioPlayHead
{
public:
    explicit AudioEngine(Project& project);
    ~AudioEngine() override;

    // Offline / test entry points. prepare() must be called before render().
    void prepare(double sampleRate, int maxBlockSize);
    void render(juce::AudioBuffer<float>& stereoOut, int numSamples);
    bool exportWav(const juce::File& file, double endBeat, juce::String& error);

    // Prepares a single track's devices for the current rate. Takes the project lock.
    void prepareTrack(Track& track);
    void prepareDevice(Device& device, bool isInstrument);

    // Transport (thread safe)
    void play();
    void stop();
    void togglePlay() { isPlaying() ? stop() : play(); }
    bool isPlaying() const { return playing.load(); }
    void setPlayhead(double beats);
    double getPlayhead() const { return position.load(); }
    void setRecording(bool shouldRecord) { recording = shouldRecord; }
    bool isRecording() const { return recording.load(); }
    void setMetronome(bool on) { metronome = on; }
    bool isMetronomeOn() const { return metronome.load(); }

    // Live MIDI from the on-screen/computer keyboard and hardware inputs
    void addLiveMidi(const juce::MidiMessage& message);
    void setLiveTrack(int trackId) { liveTrackId = trackId; }
    void handleIncomingMidiMessage(juce::MidiInput*, const juce::MidiMessage& message) override { addLiveMidi(message); }

    // Notes captured while recording, drained by the UI.
    struct RecordedEvent { int trackId; double beat; int pitch; float velocity; bool noteOn; };
    int popRecordedEvents(std::vector<RecordedEvent>& out);

    // Audition a sample from the browser.
    void previewSample(std::shared_ptr<const AudioSample> sample);

    double getSampleRate() const { return sampleRate; }
    int getBlockSize() const { return blockSize; }

    // AudioIODeviceCallback
    void audioDeviceIOCallbackWithContext(const float* const* inputs, int numInputs,
                                          float* const* outputs, int numOutputs, int numSamples,
                                          const juce::AudioIODeviceCallbackContext&) override;
    void audioDeviceAboutToStart(juce::AudioIODevice* device) override;
    void audioDeviceStopped() override;

    // AudioPlayHead, so hosted plugins can tempo sync
    juce::Optional<PositionInfo> getPosition() const override;

private:
    struct Segment { int offset; int length; double beat; };
    void renderBlock(juce::AudioBuffer<float>& out, int numSamples);
    void renderTrack(Track& track, const Segment* segments, int numSegments, int numSamples, bool stopNotes,
                     const juce::MidiBuffer& live, bool isLiveTarget);
    void addClipMidi(Track& track, const Segment& seg, double samplesPerBeat);
    void releaseClipNotes(Track& track, int offset);
    void renderAudioClips(Track& track, const Segment& seg, double samplesPerBeat);
    void runDevice(Device& device, juce::AudioBuffer<float>& buffer, juce::MidiBuffer& midi, int numSamples);
    void renderMetronome(juce::AudioBuffer<float>& out, const Segment& seg, double samplesPerBeat);
    void renderPreview(juce::AudioBuffer<float>& out, int numSamples);

    Project& project;
    double sampleRate = 44100.0;
    int blockSize = 512;

    std::atomic<bool> playing { false }, recording { false }, metronome { false };
    std::atomic<double> position { 0.0 };
    std::atomic<double> requestedPosition { -1.0 };
    std::atomic<bool> stopRequested { false };
    std::atomic<int> liveTrackId { 0 };
    double blockStartBeat = 0.0;
    bool blockPlaying = false;

    juce::MidiMessageCollector liveCollector;
    juce::MidiBuffer liveBlock;
    juce::AudioBuffer<float> master;

    std::vector<RecordedEvent> recordQueue;
    juce::SpinLock recordLock;

    int clickRemaining = 0;
    double clickPhase = 0.0, clickFreq = 1000.0;

    juce::SpinLock previewLock;
    std::shared_ptr<const AudioSample> preview;
    double previewPos = 0.0;
};

} // namespace az

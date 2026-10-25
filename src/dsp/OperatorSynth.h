#pragma once
#include <juce_audio_processors/juce_audio_processors.h>
#include <array>

namespace az
{

// AudioZero "Operator": a four-operator phase-modulation (FM) synthesizer in the spirit of
// Ableton Operator / DX-style synths. Operators are named A-D; A is always a carrier.
class OperatorSynth final : public juce::AudioProcessor
{
public:
    static constexpr int numOperators = 4;
    static constexpr int numVoices = 16;
    static constexpr int numAlgorithms = 8;

    struct Algorithm
    {
        // modulators[i] is a bit mask of operators that phase-modulate operator i.
        std::array<int, numOperators> modulators;
        int carriers; // bit mask of operators summed to the output
    };
    static const Algorithm& algorithm(int index);

    OperatorSynth();
    ~OperatorSynth() override = default;

    static juce::AudioProcessorValueTreeState::ParameterLayout createLayout();
    juce::AudioProcessorValueTreeState params;

    static juce::StringArray presetNames();
    void applyPreset(const juce::String& name);
    juce::String currentPreset { "Init" };

    // AudioProcessor
    const juce::String getName() const override { return "Operator"; }
    void prepareToPlay(double sampleRate, int blockSize) override;
    void releaseResources() override {}
    bool isBusesLayoutSupported(const BusesLayout& layout) const override;
    void processBlock(juce::AudioBuffer<float>&, juce::MidiBuffer&) override;
    juce::AudioProcessorEditor* createEditor() override;
    bool hasEditor() const override { return true; }
    bool acceptsMidi() const override { return true; }
    bool producesMidi() const override { return false; }
    bool isMidiEffect() const override { return false; }
    double getTailLengthSeconds() const override { return 2.0; }
    int getNumPrograms() override { return presetNames().size(); }
    int getCurrentProgram() override { return std::max(0, presetNames().indexOf(currentPreset)); }
    void setCurrentProgram(int index) override { applyPreset(presetNames()[index]); }
    const juce::String getProgramName(int index) override { return presetNames()[index]; }
    void changeProgramName(int, const juce::String&) override {}
    void getStateInformation(juce::MemoryBlock&) override;
    void setStateInformation(const void* data, int size) override;

    int activeVoiceCount() const { return activeVoices.load(); }

private:
    struct Envelope
    {
        enum class Stage { idle, attack, decay, sustain, release };
        Stage stage = Stage::idle;
        float level = 0.0f;
        float attackStep = 0.0f, decayCoef = 0.0f, releaseCoef = 0.0f, sustain = 1.0f;
        void set(float attackSec, float decaySec, float sustainLevel, float releaseSec, double sr);
        void noteOn() { stage = Stage::attack; }
        void noteOff() { if (stage != Stage::idle) stage = Stage::release; }
        float next();
        bool active() const { return stage != Stage::idle; }
    };

    struct Voice
    {
        int note = -1;
        float velocity = 0.0f;
        bool held = false, sustained = false;
        juce::uint32 age = 0;
        double frequency = 440.0;
        std::array<double, numOperators> phase {};
        std::array<Envelope, numOperators> env;
        Envelope filterEnv;
        float pitchEnv = 0.0f;
        float feedback1 = 0.0f, feedback2 = 0.0f;
        float ic1 = 0.0f, ic2 = 0.0f; // filter state
        juce::uint32 noiseSeed = 1;
        bool active() const { return env[0].active() || env[1].active() || env[2].active() || env[3].active(); }
    };

    struct Snapshot; // parameter values read once per block
    void startVoice(int note, float velocity);
    void stopVoice(int note);
    void renderVoices(float* left, float* right, int start, int end, const Snapshot&);

    double sampleRate = 44100.0;
    std::array<Voice, numVoices> voices;
    juce::uint32 voiceCounter = 0;
    double lfoPhase = 0.0;
    bool sustainPedal = false;
    std::atomic<int> activeVoices { 0 };
};

} // namespace az

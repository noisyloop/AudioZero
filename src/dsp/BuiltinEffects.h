#pragma once
#include <juce_audio_processors/juce_audio_processors.h>
#include <juce_audio_basics/juce_audio_basics.h>

namespace az
{

// Shared boilerplate for the small built-in effects.
class BuiltinEffect : public juce::AudioProcessor
{
public:
    BuiltinEffect(const juce::String& effectName, juce::AudioProcessorValueTreeState::ParameterLayout layout);

    juce::AudioProcessorValueTreeState params;

    const juce::String getName() const override { return name; }
    void releaseResources() override {}
    bool isBusesLayoutSupported(const BusesLayout& layout) const override;
    juce::AudioProcessorEditor* createEditor() override;
    bool hasEditor() const override { return true; }
    bool acceptsMidi() const override { return false; }
    bool producesMidi() const override { return false; }
    bool isMidiEffect() const override { return false; }
    double getTailLengthSeconds() const override { return 4.0; }
    int getNumPrograms() override { return 1; }
    int getCurrentProgram() override { return 0; }
    void setCurrentProgram(int) override {}
    const juce::String getProgramName(int) override { return {}; }
    void changeProgramName(int, const juce::String&) override {}
    void getStateInformation(juce::MemoryBlock&) override;
    void setStateInformation(const void* data, int size) override;

protected:
    float value(const char* id) const { return params.getRawParameterValue(id)->load(); }

private:
    juce::String name;
};

class ReverbEffect final : public BuiltinEffect
{
public:
    ReverbEffect();
    void prepareToPlay(double sampleRate, int blockSize) override;
    void processBlock(juce::AudioBuffer<float>&, juce::MidiBuffer&) override;

private:
    juce::Reverb reverb;
};

class DelayEffect final : public BuiltinEffect
{
public:
    DelayEffect();
    void prepareToPlay(double sampleRate, int blockSize) override;
    void processBlock(juce::AudioBuffer<float>&, juce::MidiBuffer&) override;

private:
    juce::AudioBuffer<float> line;
    int writePos = 0;
    double sampleRate = 44100.0;
    float smoothedDelay = 0.0f;
    std::array<float, 2> toneState {};
};

// Factory for everything AudioZero ships with.
struct Builtins
{
    struct Info { juce::String id, name, description; bool isInstrument; };
    static const std::vector<Info>& all();
    static const Info* find(const juce::String& id);
    static std::unique_ptr<juce::AudioProcessor> create(const juce::String& id);
};

} // namespace az

#pragma once
#include <juce_audio_processors/juce_audio_processors.h>
#include <cmath>

// AudioZero's built-in test instrument; also compiled as a VST3 for host testing.
class SmokeSynth final : public juce::AudioProcessor
{
public:
    SmokeSynth() : AudioProcessor(BusesProperties().withOutput("Output", juce::AudioChannelSet::stereo(), true)) {}
    const juce::String getName() const override { return "SmokeSynth"; }
    void prepareToPlay(double sr, int) override { sampleRate = sr > 0 ? sr : 44100.0; }
    void releaseResources() override {}
    bool isBusesLayoutSupported(const BusesLayout& l) const override
    { return l.getMainOutputChannelSet() == juce::AudioChannelSet::stereo(); }
    void processBlock(juce::AudioBuffer<float>& buffer, juce::MidiBuffer& midi) override
    {
        int pos = 0;
        auto render = [&](int end) {
            for (; pos < end; ++pos) {
                float value = 0.0f;
                if (active) {
                    value = static_cast<float>(std::sin(phase) * 0.15);
                    phase += 2.0 * juce::MathConstants<double>::pi * frequency / sampleRate;
                    if (phase >= 2.0 * juce::MathConstants<double>::pi)
                        phase -= 2.0 * juce::MathConstants<double>::pi;
                }
                for (int ch = 0; ch < buffer.getNumChannels(); ++ch)
                    buffer.setSample(ch, pos, value);
            }
        };
        for (const auto item : midi) {
            const int offset = juce::jlimit(0, buffer.getNumSamples(), item.samplePosition);
            render(offset);
            auto msg = item.getMessage();
            if (msg.isNoteOn()) {
                note = msg.getNoteNumber();
                frequency = juce::MidiMessage::getMidiNoteInHertz(note);
                active = true;
            } else if ((msg.isNoteOff() && msg.getNoteNumber() == note) || msg.isAllNotesOff()) {
                active = false;
            }
        }
        render(buffer.getNumSamples());
    }
    juce::AudioProcessorEditor* createEditor() override { return new juce::GenericAudioProcessorEditor(*this); }
    bool hasEditor() const override { return true; }
    bool acceptsMidi() const override { return true; }
    bool producesMidi() const override { return false; }
    bool isMidiEffect() const override { return false; }
    double getTailLengthSeconds() const override { return 0.0; }
    int getNumPrograms() override { return 1; }
    int getCurrentProgram() override { return 0; }
    void setCurrentProgram(int) override {}
    const juce::String getProgramName(int) override { return {}; }
    void changeProgramName(int, const juce::String&) override {}
    void getStateInformation(juce::MemoryBlock& block) override {
        juce::MemoryOutputStream stream(block, false);
        stream.writeInt(note);
        stream.writeBool(active);
    }
    void setStateInformation(const void* data, int size) override {
        if (size < 5 || data == nullptr) return;
        juce::MemoryInputStream stream(data, static_cast<size_t>(size), false);
        note = juce::jlimit(0, 127, stream.readInt());
        active = stream.readBool();
        frequency = juce::MidiMessage::getMidiNoteInHertz(note);
    }
private:
    double sampleRate = 44100.0, phase = 0.0, frequency = 261.6256;
    int note = 60;
    bool active = false;
};

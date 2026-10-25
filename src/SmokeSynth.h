#pragma once
#include <juce_audio_processors/juce_audio_processors.h>
#include <array>
#include <cmath>

// Four-operator mono FM instrument. Parameters are exposed to both the built-in
// AudioZero device and the VST3 editor/automation system.
class SmokeSynth final : public juce::AudioProcessor
{
public:
    SmokeSynth() : AudioProcessor(BusesProperties().withOutput("Output", juce::AudioChannelSet::stereo(), true))
    {
        auto add = [this](const char* id, const char* name, float low, float high, float def) {
            addParameter(new juce::AudioParameterFloat(juce::ParameterID(id, 1), name,
                juce::NormalisableRange<float>(low, high, 0.001f), def));
        };
        add("gain", "Output Gain", 0.0f, 1.0f, 0.18f);
        add("attack", "Attack", 0.001f, 4.0f, 0.01f);
        add("decay", "Decay", 0.001f, 4.0f, 0.15f);
        add("sustain", "Sustain", 0.0f, 1.0f, 0.75f);
        add("release", "Release", 0.001f, 6.0f, 0.25f);
        add("cutoff", "Lowpass Cutoff", 50.0f, 20000.0f, 12000.0f);
        for (int i = 0; i < 4; ++i)
        {
            auto n = juce::String(i + 1);
            addParameter(new juce::AudioParameterFloat(juce::ParameterID("ratio" + n, 1),
                "Operator " + n + " Ratio", juce::NormalisableRange<float>(0.25f, 16.0f, 0.01f), i == 0 ? 1.0f : static_cast<float>(i + 1)));
            addParameter(new juce::AudioParameterFloat(juce::ParameterID("level" + n, 1),
                "Operator " + n + " Level", juce::NormalisableRange<float>(0.0f, 1.0f, 0.001f), i == 0 ? 1.0f : 0.0f));
        }
        add("fm12", "FM 2 to 1", 0.0f, 10.0f, 0.0f);
        add("fm23", "FM 3 to 2", 0.0f, 10.0f, 0.0f);
        add("fm34", "FM 4 to 3", 0.0f, 10.0f, 0.0f);
    }
    const juce::String getName() const override { return "SmokeSynth"; }
    void prepareToPlay(double sr, int) override
    {
        sampleRate = sr > 0 ? sr : 44100.0;
        envelope.setSampleRate(sampleRate);
        updateEnvelope();
        filterMemory = 0.0;
    }
    void releaseResources() override {}
    bool isBusesLayoutSupported(const BusesLayout& l) const override
    { return l.getMainOutputChannelSet() == juce::AudioChannelSet::stereo(); }
    void processBlock(juce::AudioBuffer<float>& buffer, juce::MidiBuffer& midi) override
    {
        juce::ScopedNoDenormals noDenormals;
        updateEnvelope();
        std::array<double, 4> ratios{}, levels{};
        for (int i = 0; i < 4; ++i) {
            ratios[static_cast<size_t>(i)] = get("ratio" + juce::String(i + 1));
            levels[static_cast<size_t>(i)] = get("level" + juce::String(i + 1));
        }
        const double fm12 = get("fm12"), fm23 = get("fm23"), fm34 = get("fm34");
        const double gain = get("gain");
        const double cutoff = juce::jlimit(50.0, sampleRate * 0.45, static_cast<double>(get("cutoff")));
        const double alpha = 1.0 - std::exp(-2.0 * juce::MathConstants<double>::pi * cutoff / sampleRate);
        const double tau = 2.0 * juce::MathConstants<double>::pi;
        int position = 0;
        auto render = [&](int end) {
            for (; position < end; ++position) {
                double output = 0.0;
                if (envelope.isActive()) {
                    const double op4 = std::sin(phases[3]) * levels[3];
                    const double op3 = std::sin(phases[2] + fm34 * op4) * levels[2];
                    const double op2 = std::sin(phases[1] + fm23 * op3) * levels[1];
                    const double op1 = std::sin(phases[0] + fm12 * op2) * levels[0];
                    output = op1 * envelope.getNextSample() * gain;
                    for (int i = 0; i < 4; ++i) {
                        phases[static_cast<size_t>(i)] += tau * frequency * ratios[static_cast<size_t>(i)] / sampleRate;
                        if (phases[static_cast<size_t>(i)] >= tau)
                            phases[static_cast<size_t>(i)] = std::fmod(phases[static_cast<size_t>(i)], tau);
                    }
                }
                filterMemory += alpha * (output - filterMemory);
                const auto sample = static_cast<float>(filterMemory);
                for (int ch = 0; ch < buffer.getNumChannels(); ++ch)
                    buffer.setSample(ch, position, sample);
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
                envelope.noteOn();
            } else if ((msg.isNoteOff() && msg.getNoteNumber() == note) || msg.isAllNotesOff()) {
                active = false;
                envelope.noteOff();
            }
        }
        render(buffer.getNumSamples());
    }
    juce::AudioProcessorEditor* createEditor() override { return new juce::GenericAudioProcessorEditor(*this); }
    bool hasEditor() const override { return true; }
    bool acceptsMidi() const override { return true; }
    bool producesMidi() const override { return false; }
    bool isMidiEffect() const override { return false; }
    double getTailLengthSeconds() const override { return get("release"); }
    int getNumPrograms() override { return 1; }
    int getCurrentProgram() override { return 0; }
    void setCurrentProgram(int) override {}
    const juce::String getProgramName(int) override { return {}; }
    void changeProgramName(int, const juce::String&) override {}
    void getStateInformation(juce::MemoryBlock& block) override
    {
        juce::MemoryOutputStream stream(block, false);
        stream.writeInt(0x534D4B32); // SMK2
        stream.writeInt(note);
        stream.writeBool(active);
        for (auto* p : getParameters())
            stream.writeFloat(p->getValue());
    }
    void setStateInformation(const void* data, int size) override
    {
        if (data == nullptr || size < 9) return;
        juce::MemoryInputStream stream(data, static_cast<size_t>(size), false);
        if (stream.readInt() != 0x534D4B32) return;
        note = juce::jlimit(0, 127, stream.readInt());
        active = stream.readBool();
        frequency = juce::MidiMessage::getMidiNoteInHertz(note);
        for (auto* p : getParameters()) {
            if (stream.getNumBytesRemaining() < sizeof(float)) break;
            p->setValueNotifyingHost(stream.readFloat());
        }
        if (active) envelope.noteOn();
        else envelope.reset();
    }
private:
    float get(const juce::String& id) const
    {
        for (auto* p : getParameters())
            if (p->getName(128).isNotEmpty() && p->getParameterID() == id)
                return static_cast<juce::AudioParameterFloat*>(p)->get();
        return 0.0f;
    }
    void updateEnvelope()
    {
        juce::ADSR::Parameters p;
        p.attack = get("attack"); p.decay = get("decay");
        p.sustain = get("sustain"); p.release = get("release");
        envelope.setParameters(p);
    }
    juce::ADSR envelope;
    std::array<double, 4> phases{};
    double sampleRate = 44100.0, frequency = 261.6256, filterMemory = 0.0;
    int note = 60;
    bool active = false;
};

#include "BuiltinEffects.h"
#include "OperatorSynth.h"
#include "../ui/KnobEditor.h"

namespace az
{

BuiltinEffect::BuiltinEffect(const juce::String& effectName, juce::AudioProcessorValueTreeState::ParameterLayout layout)
    : AudioProcessor(BusesProperties()
                         .withInput("Input", juce::AudioChannelSet::stereo(), true)
                         .withOutput("Output", juce::AudioChannelSet::stereo(), true)),
      params(*this, nullptr, juce::Identifier(effectName.removeCharacters(" ")), std::move(layout)),
      name(effectName)
{
}

bool BuiltinEffect::isBusesLayoutSupported(const BusesLayout& layout) const
{
    return layout.getMainOutputChannelSet() == juce::AudioChannelSet::stereo()
        && layout.getMainInputChannelSet() == juce::AudioChannelSet::stereo();
}

juce::AudioProcessorEditor* BuiltinEffect::createEditor()
{
    return new KnobEditor(*this, params);
}

void BuiltinEffect::getStateInformation(juce::MemoryBlock& block)
{
    if (auto xml = params.copyState().createXml())
        copyXmlToBinary(*xml, block);
}

void BuiltinEffect::setStateInformation(const void* data, int size)
{
    if (auto xml = getXmlFromBinary(data, size))
        if (xml->hasTagName(params.state.getType()))
            params.replaceState(juce::ValueTree::fromXml(*xml));
}

//==============================================================================
static juce::AudioProcessorValueTreeState::ParameterLayout reverbLayout()
{
    juce::AudioProcessorValueTreeState::ParameterLayout layout;
    auto add = [&](const char* id, const char* name, float def) {
        layout.add(std::make_unique<juce::AudioParameterFloat>(juce::ParameterID(id, 1), name, juce::NormalisableRange<float>(0.0f, 1.0f), def));
    };
    add("size", "Size", 0.6f);
    add("damping", "Damping", 0.45f);
    add("width", "Width", 1.0f);
    add("mix", "Dry/Wet", 0.3f);
    return layout;
}

ReverbEffect::ReverbEffect() : BuiltinEffect("Reverb", reverbLayout()) {}

void ReverbEffect::prepareToPlay(double sampleRate, int)
{
    reverb.setSampleRate(sampleRate);
    reverb.reset();
}

void ReverbEffect::processBlock(juce::AudioBuffer<float>& buffer, juce::MidiBuffer&)
{
    juce::Reverb::Parameters p;
    p.roomSize = value("size");
    p.damping = value("damping");
    p.width = value("width");
    const float mix = value("mix");
    p.wetLevel = mix;
    p.dryLevel = 1.0f - mix * 0.5f;
    reverb.setParameters(p);
    if (buffer.getNumChannels() >= 2)
        reverb.processStereo(buffer.getWritePointer(0), buffer.getWritePointer(1), buffer.getNumSamples());
    else
        reverb.processMono(buffer.getWritePointer(0), buffer.getNumSamples());
}

//==============================================================================
static juce::AudioProcessorValueTreeState::ParameterLayout delayLayout()
{
    using namespace juce;
    AudioProcessorValueTreeState::ParameterLayout layout;
    layout.add(std::make_unique<AudioParameterFloat>(ParameterID("time", 1), "Time", NormalisableRange<float>(10.0f, 2000.0f, 0.0f, 0.5f), 375.0f,
                                                     AudioParameterFloatAttributes().withLabel("ms")));
    layout.add(std::make_unique<AudioParameterFloat>(ParameterID("feedback", 1), "Feedback", NormalisableRange<float>(0.0f, 0.95f), 0.4f));
    layout.add(std::make_unique<AudioParameterFloat>(ParameterID("tone", 1), "Tone", NormalisableRange<float>(0.0f, 1.0f), 0.6f));
    layout.add(std::make_unique<AudioParameterFloat>(ParameterID("mix", 1), "Dry/Wet", NormalisableRange<float>(0.0f, 1.0f), 0.3f));
    layout.add(std::make_unique<AudioParameterBool>(ParameterID("pingpong", 1), "Ping Pong", true));
    return layout;
}

DelayEffect::DelayEffect() : BuiltinEffect("Delay", delayLayout()) {}

void DelayEffect::prepareToPlay(double sr, int)
{
    sampleRate = sr;
    line.setSize(2, static_cast<int>(sr * 2.1) + 1);
    line.clear();
    writePos = 0;
    smoothedDelay = static_cast<float>(value("time") * 0.001 * sr);
    toneState = {};
}

void DelayEffect::processBlock(juce::AudioBuffer<float>& buffer, juce::MidiBuffer&)
{
    const int size = line.getNumSamples();
    if (size < 2 || buffer.getNumChannels() < 2)
        return;
    const float target = static_cast<float>(value("time") * 0.001 * sampleRate);
    const float feedback = value("feedback");
    const float mix = value("mix");
    const bool pingPong = value("pingpong") > 0.5f;
    const float toneCoef = 0.05f + 0.95f * value("tone");
    float* l = buffer.getWritePointer(0);
    float* r = buffer.getWritePointer(1);
    float* dl = line.getWritePointer(0);
    float* dr = line.getWritePointer(1);

    for (int i = 0; i < buffer.getNumSamples(); ++i)
    {
        smoothedDelay += 0.0005f * (target - smoothedDelay);
        float readPos = static_cast<float>(writePos) - smoothedDelay;
        while (readPos < 0)
            readPos += static_cast<float>(size);
        const int i0 = static_cast<int>(readPos) % size;
        const int i1 = (i0 + 1) % size;
        const float frac = readPos - std::floor(readPos);
        float wetL = dl[i0] + frac * (dl[i1] - dl[i0]);
        float wetR = dr[i0] + frac * (dr[i1] - dr[i0]);
        toneState[0] += toneCoef * (wetL - toneState[0]);
        toneState[1] += toneCoef * (wetR - toneState[1]);
        wetL = toneState[0];
        wetR = toneState[1];

        const float inL = l[i], inR = r[i];
        if (pingPong)
        {
            dl[writePos] = 0.5f * (inL + inR) + wetR * feedback;
            dr[writePos] = wetL * feedback;
        }
        else
        {
            dl[writePos] = inL + wetL * feedback;
            dr[writePos] = inR + wetR * feedback;
        }
        l[i] = inL + mix * wetL;
        r[i] = inR + mix * wetR;
        writePos = (writePos + 1) % size;
    }
}

//==============================================================================
const std::vector<Builtins::Info>& Builtins::all()
{
    static const std::vector<Info> list = {
        { "operator", "Operator", "4-operator FM synthesizer", true },
        { "reverb", "Reverb", "Algorithmic stereo reverb", false },
        { "delay", "Delay", "Stereo / ping-pong delay", false },
    };
    return list;
}

const Builtins::Info* Builtins::find(const juce::String& id)
{
    for (auto& info : all())
        if (info.id == id)
            return &info;
    return nullptr;
}

std::unique_ptr<juce::AudioProcessor> Builtins::create(const juce::String& id)
{
    if (id == "operator") return std::make_unique<OperatorSynth>();
    if (id == "reverb") return std::make_unique<ReverbEffect>();
    if (id == "delay") return std::make_unique<DelayEffect>();
    return nullptr;
}

} // namespace az

#include "OperatorSynth.h"
#include "../ui/OperatorEditor.h"
#include <map>

namespace az
{

namespace
{
    const char* opNames[] = { "A", "B", "C", "D" };

    juce::String opId(int op, const char* name) { return juce::String("op") + opNames[op] + name; }

    juce::NormalisableRange<float> timeRange(float maxSeconds)
    {
        return juce::NormalisableRange<float>(0.001f, maxSeconds, 0.0f, 0.3f);
    }

    juce::uint32 nextRandom(juce::uint32& seed)
    {
        seed = seed * 1664525u + 1013904223u;
        return seed;
    }
}

const OperatorSynth::Algorithm& OperatorSynth::algorithm(int index)
{
    // Operator indices: 0 = A ... 3 = D. Modulators always have a higher index than their target,
    // so evaluating D -> A in order is sufficient.
    static const Algorithm algorithms[numAlgorithms] = {
        { { 0b0010, 0b0100, 0b1000, 0 }, 0b0001 }, // 1: D > C > B > A
        { { 0b0010, 0b1100, 0, 0 }, 0b0001 },      // 2: (C + D) > B > A
        { { 0b0110, 0, 0b1000, 0 }, 0b0001 },      // 3: B > A, D > C > A
        { { 0b1110, 0, 0, 0 }, 0b0001 },           // 4: (B + C + D) > A
        { { 0b0010, 0, 0b1000, 0 }, 0b0101 },      // 5: B > A,  D > C  (two stacks)
        { { 0b1000, 0b1000, 0b1000, 0 }, 0b0111 }, // 6: D > (A, B, C)
        { { 0, 0, 0b1000, 0 }, 0b0111 },           // 7: D > C, plus A, B
        { { 0, 0, 0, 0 }, 0b1111 },                // 8: additive
    };
    return algorithms[juce::jlimit(0, numAlgorithms - 1, index)];
}

juce::AudioProcessorValueTreeState::ParameterLayout OperatorSynth::createLayout()
{
    using namespace juce;
    AudioProcessorValueTreeState::ParameterLayout layout;
    auto addFloat = [&layout](const String& id, const String& name, NormalisableRange<float> range, float def, const String& label = {}) {
        layout.add(std::make_unique<AudioParameterFloat>(ParameterID(id, 1), name, range, def, AudioParameterFloatAttributes().withLabel(label)));
    };

    StringArray algorithmNames;
    for (int i = 1; i <= numAlgorithms; ++i)
        algorithmNames.add(String(i));
    layout.add(std::make_unique<AudioParameterChoice>(ParameterID("algorithm", 1), "Algorithm", algorithmNames, 0));
    addFloat("volume", "Volume", { 0.0f, 1.0f }, 0.7f);
    layout.add(std::make_unique<AudioParameterInt>(ParameterID("transpose", 1), "Transpose", -24, 24, 0));
    addFloat("feedback", "Feedback", { 0.0f, 1.0f }, 0.0f);

    layout.add(std::make_unique<AudioParameterBool>(ParameterID("filterOn", 1), "Filter On", false));
    layout.add(std::make_unique<AudioParameterChoice>(ParameterID("filterType", 1), "Filter Type", StringArray { "Low Pass", "High Pass", "Band Pass" }, 0));
    addFloat("cutoff", "Cutoff", NormalisableRange<float>(20.0f, 20000.0f, 0.0f, 0.25f), 18000.0f, "Hz");
    addFloat("resonance", "Resonance", { 0.0f, 1.0f }, 0.1f);
    addFloat("filterEnv", "Filter Env", { -1.0f, 1.0f }, 0.0f);
    addFloat("filterAttack", "Filter Attack", timeRange(10.0f), 0.002f, "s");
    addFloat("filterDecay", "Filter Decay", timeRange(10.0f), 0.4f, "s");
    addFloat("filterSustain", "Filter Sustain", { 0.0f, 1.0f }, 0.0f);
    addFloat("filterRelease", "Filter Release", timeRange(10.0f), 0.3f, "s");

    addFloat("pitchEnvAmount", "Pitch Env", { -48.0f, 48.0f }, 0.0f, "st");
    addFloat("pitchEnvDecay", "Pitch Decay", timeRange(2.0f), 0.1f, "s");

    addFloat("lfoRate", "LFO Rate", NormalisableRange<float>(0.05f, 20.0f, 0.0f, 0.4f), 4.0f, "Hz");
    addFloat("lfoAmount", "LFO Amount", { 0.0f, 1.0f }, 0.0f);
    layout.add(std::make_unique<AudioParameterChoice>(ParameterID("lfoDest", 1), "LFO Dest", StringArray { "Pitch", "Filter", "Volume", "FM" }, 0));

    for (int op = 0; op < numOperators; ++op)
    {
        const String n = String("Op ") + opNames[op] + " ";
        layout.add(std::make_unique<AudioParameterBool>(ParameterID(opId(op, "On"), 1), n + "On", true));
        layout.add(std::make_unique<AudioParameterChoice>(ParameterID(opId(op, "Wave"), 1), n + "Wave",
                                                          StringArray { "Sine", "Triangle", "Saw", "Square", "Noise" }, 0));
        layout.add(std::make_unique<AudioParameterInt>(ParameterID(opId(op, "Coarse"), 1), n + "Coarse", 0, 32, 1));
        addFloat(opId(op, "Fine"), n + "Fine", { 0.0f, 1000.0f, 1.0f }, 0.0f);
        addFloat(opId(op, "Level"), n + "Level", { 0.0f, 1.0f }, op == 0 ? 1.0f : 0.0f);
        addFloat(opId(op, "Attack"), n + "Attack", timeRange(10.0f), 0.002f, "s");
        addFloat(opId(op, "Decay"), n + "Decay", timeRange(10.0f), 0.6f, "s");
        addFloat(opId(op, "Sustain"), n + "Sustain", { 0.0f, 1.0f }, 1.0f);
        addFloat(opId(op, "Release"), n + "Release", timeRange(10.0f), 0.3f, "s");
        addFloat(opId(op, "Vel"), n + "Velocity", { 0.0f, 1.0f }, 0.5f);
    }
    return layout;
}

OperatorSynth::OperatorSynth()
    : AudioProcessor(BusesProperties().withOutput("Output", juce::AudioChannelSet::stereo(), true)),
      params(*this, nullptr, "Operator", createLayout())
{
}

bool OperatorSynth::isBusesLayoutSupported(const BusesLayout& layout) const
{
    const auto& out = layout.getMainOutputChannelSet();
    return out == juce::AudioChannelSet::stereo() || out == juce::AudioChannelSet::mono();
}

//==============================================================================
juce::StringArray OperatorSynth::presetNames()
{
    return { "Init", "Electric Piano", "FM Bass", "Glass Bell", "Warm Pad", "Kick", "Pluck", "Brass", "Wobble Bass", "Hi-Hat" };
}

void OperatorSynth::applyPreset(const juce::String& name)
{
    using Values = std::map<juce::String, float>;
    auto op = [](int index, Values v) {
        Values out;
        for (auto& [key, value] : v)
            out[opId(index, key.toRawUTF8())] = value;
        return out;
    };
    auto merge = [](std::initializer_list<Values> parts) {
        Values out;
        for (auto& part : parts)
            out.insert(part.begin(), part.end());
        return out;
    };
    // Algorithm values are zero based indices of the choice parameter.
    Values values;
    if (name == "Electric Piano")
        values = merge({ { { "algorithm", 4 }, { "volume", 0.6f } },
                         op(0, { { "Level", 0.85f }, { "Decay", 1.8f }, { "Sustain", 0.25f }, { "Release", 0.5f } }),
                         op(1, { { "Level", 0.45f }, { "Decay", 0.9f }, { "Sustain", 0.1f }, { "Vel", 0.9f } }),
                         op(2, { { "Level", 0.5f }, { "Fine", 4 }, { "Decay", 2.2f }, { "Sustain", 0.2f }, { "Release", 0.5f } }),
                         op(3, { { "Coarse", 14 }, { "Level", 0.18f }, { "Decay", 0.25f }, { "Sustain", 0.0f } }) });
    else if (name == "FM Bass")
        values = merge({ { { "algorithm", 0 }, { "feedback", 0.2f }, { "filterOn", 1 }, { "cutoff", 1600 }, { "resonance", 0.3f },
                           { "filterEnv", 0.45f }, { "filterDecay", 0.25f } },
                         op(0, { { "Attack", 0.001f }, { "Decay", 0.4f }, { "Sustain", 0.7f }, { "Release", 0.08f } }),
                         op(1, { { "Level", 0.6f }, { "Decay", 0.3f }, { "Sustain", 0.25f } }),
                         op(2, { { "Coarse", 2 }, { "Level", 0.3f }, { "Decay", 0.2f }, { "Sustain", 0.0f } }) });
    else if (name == "Glass Bell")
        values = merge({ { { "algorithm", 4 }, { "volume", 0.55f } },
                         op(0, { { "Decay", 3.0f }, { "Sustain", 0.0f }, { "Release", 2.0f } }),
                         op(1, { { "Coarse", 3 }, { "Fine", 500 }, { "Level", 0.5f }, { "Decay", 2.0f }, { "Sustain", 0.0f } }),
                         op(2, { { "Coarse", 2 }, { "Level", 0.45f }, { "Decay", 2.5f }, { "Sustain", 0.0f }, { "Release", 2.0f } }),
                         op(3, { { "Coarse", 7 }, { "Level", 0.35f }, { "Decay", 1.5f }, { "Sustain", 0.0f } }) });
    else if (name == "Warm Pad")
        values = merge({ { { "algorithm", 5 }, { "volume", 0.5f }, { "filterOn", 1 }, { "cutoff", 2400 }, { "resonance", 0.2f },
                           { "lfoDest", 1 }, { "lfoRate", 0.3f }, { "lfoAmount", 0.3f } },
                         op(0, { { "Wave", 2 }, { "Attack", 0.6f }, { "Decay", 1.0f }, { "Sustain", 0.8f }, { "Release", 1.5f } }),
                         op(1, { { "Wave", 2 }, { "Fine", 4 }, { "Level", 0.8f }, { "Attack", 0.7f }, { "Sustain", 0.8f }, { "Release", 1.5f } }),
                         op(2, { { "Coarse", 2 }, { "Fine", 2 }, { "Level", 0.35f }, { "Attack", 0.9f }, { "Sustain", 0.8f }, { "Release", 1.5f } }),
                         op(3, { { "Level", 0.2f }, { "Attack", 1.0f }, { "Sustain", 0.6f } }) });
    else if (name == "Kick")
        values = merge({ { { "algorithm", 0 }, { "pitchEnvAmount", 36 }, { "pitchEnvDecay", 0.06f }, { "volume", 0.85f } },
                         op(0, { { "Attack", 0.0005f }, { "Decay", 0.45f }, { "Sustain", 0.0f }, { "Release", 0.1f }, { "Vel", 0.3f } }),
                         op(1, { { "Level", 0.25f }, { "Decay", 0.04f }, { "Sustain", 0.0f } }) });
    else if (name == "Pluck")
        values = merge({ { { "algorithm", 1 }, { "filterOn", 1 }, { "cutoff", 2500 }, { "filterEnv", 0.5f }, { "filterDecay", 0.2f }, { "volume", 0.6f } },
                         op(0, { { "Decay", 0.35f }, { "Sustain", 0.0f }, { "Release", 0.3f } }),
                         op(1, { { "Coarse", 2 }, { "Level", 0.5f }, { "Decay", 0.2f }, { "Sustain", 0.0f } }),
                         op(2, { { "Coarse", 3 }, { "Level", 0.2f }, { "Decay", 0.15f }, { "Sustain", 0.0f } }) });
    else if (name == "Brass")
        values = merge({ { { "algorithm", 0 }, { "feedback", 0.3f }, { "lfoDest", 0 }, { "lfoRate", 5.0f }, { "lfoAmount", 0.08f }, { "volume", 0.55f } },
                         op(0, { { "Attack", 0.04f }, { "Decay", 0.3f }, { "Sustain", 0.8f }, { "Release", 0.2f } }),
                         op(1, { { "Level", 0.55f }, { "Attack", 0.08f }, { "Decay", 0.3f }, { "Sustain", 0.6f }, { "Release", 0.2f } }) });
    else if (name == "Wobble Bass")
        values = merge({ { { "algorithm", 0 }, { "filterOn", 1 }, { "cutoff", 500 }, { "resonance", 0.55f }, { "lfoDest", 1 },
                           { "lfoRate", 2.0f }, { "lfoAmount", 0.8f }, { "feedback", 0.4f } },
                         op(0, { { "Wave", 2 }, { "Sustain", 1.0f }, { "Release", 0.1f } }),
                         op(1, { { "Level", 0.7f }, { "Sustain", 0.8f } }) });
    else if (name == "Hi-Hat")
        values = merge({ { { "algorithm", 7 }, { "filterOn", 1 }, { "filterType", 1 }, { "cutoff", 7000 }, { "resonance", 0.3f }, { "volume", 0.5f } },
                         op(0, { { "Wave", 4 }, { "Level", 0.6f }, { "Attack", 0.0005f }, { "Decay", 0.07f }, { "Sustain", 0.0f }, { "Release", 0.05f } }) });

    for (auto* p : getParameters())
        if (auto* ranged = dynamic_cast<juce::RangedAudioParameter*>(p))
        {
            auto it = values.find(ranged->getParameterID());
            const float target = it != values.end() ? ranged->convertTo0to1(it->second) : ranged->getDefaultValue();
            ranged->setValueNotifyingHost(target);
        }
    currentPreset = presetNames().contains(name) ? name : "Init";
    updateHostDisplay();
}

void OperatorSynth::getStateInformation(juce::MemoryBlock& block)
{
    auto state = params.copyState();
    state.setProperty("preset", currentPreset, nullptr);
    if (auto xml = state.createXml())
        copyXmlToBinary(*xml, block);
}

void OperatorSynth::setStateInformation(const void* data, int size)
{
    if (auto xml = getXmlFromBinary(data, size))
        if (xml->hasTagName(params.state.getType()))
        {
            auto state = juce::ValueTree::fromXml(*xml);
            currentPreset = state.getProperty("preset", "Init").toString();
            params.replaceState(state);
        }
}

juce::AudioProcessorEditor* OperatorSynth::createEditor()
{
    return new OperatorEditor(*this);
}

//==============================================================================
void OperatorSynth::Envelope::set(float a, float d, float s, float r, double sr)
{
    attackStep = static_cast<float>(1.0 / (std::max(0.0005f, a) * sr));
    decayCoef = static_cast<float>(std::exp(-6.9 / (std::max(0.001f, d) * sr)));
    releaseCoef = static_cast<float>(std::exp(-6.9 / (std::max(0.001f, r) * sr)));
    sustain = s;
}

float OperatorSynth::Envelope::next()
{
    switch (stage)
    {
        case Stage::idle:
            return 0.0f;
        case Stage::attack:
            level += attackStep;
            if (level >= 1.0f)
            {
                level = 1.0f;
                stage = Stage::decay;
            }
            break;
        case Stage::decay:
            level = sustain + (level - sustain) * decayCoef;
            if (std::abs(level - sustain) < 1.0e-4f)
            {
                level = sustain;
                stage = sustain <= 1.0e-4f ? Stage::idle : Stage::sustain;
            }
            break;
        case Stage::sustain:
            level = sustain;
            break;
        case Stage::release:
            level *= releaseCoef;
            if (level < 1.0e-4f)
            {
                level = 0.0f;
                stage = Stage::idle;
            }
            break;
    }
    return level;
}

//==============================================================================
struct OperatorSynth::Snapshot
{
    const Algorithm* algorithm = nullptr;
    std::array<bool, numOperators> on {};
    std::array<int, numOperators> wave {};
    std::array<double, numOperators> ratio {};
    std::array<float, numOperators> level {}, vel {};
    float volume = 0, feedback = 0, transpose = 0;
    bool filterOn = false;
    int filterType = 0;
    float cutoff = 0, resonance = 0, filterEnv = 0;
    float pitchEnvAmount = 0, pitchEnvCoef = 0;
    float lfoAmount = 0;
    int lfoDest = 0;
    double lfoInc = 0;
};

void OperatorSynth::prepareToPlay(double sr, int)
{
    sampleRate = sr > 0 ? sr : 44100.0;
    for (auto& v : voices)
        v = Voice {};
    lfoPhase = 0.0;
}

void OperatorSynth::startVoice(int note, float velocity)
{
    Voice* target = nullptr;
    for (auto& v : voices)
        if (v.note == note && v.active())
            target = &v;
    if (target == nullptr)
        for (auto& v : voices)
            if (! v.active())
            {
                target = &v;
                break;
            }
    if (target == nullptr)
    {
        // Steal the oldest voice, preferring released ones.
        for (auto& v : voices)
            if (target == nullptr || (! v.held && target->held) || (v.held == target->held && v.age < target->age))
                target = &v;
    }

    auto& v = *target;
    const bool wasSilent = ! v.active();
    v.note = note;
    v.velocity = velocity;
    v.held = true;
    v.sustained = false;
    v.age = ++voiceCounter;
    v.frequency = juce::MidiMessage::getMidiNoteInHertz(note);
    v.pitchEnv = 1.0f;
    v.noiseSeed = 0x9e3779b9u ^ v.age;
    if (wasSilent)
    {
        v.phase = {};
        v.feedback1 = v.feedback2 = 0.0f;
        v.ic1 = v.ic2 = 0.0f;
    }
    for (int op = 0; op < numOperators; ++op)
    {
        auto get = [&](const char* id) { return params.getRawParameterValue(opId(op, id))->load(); };
        v.env[static_cast<size_t>(op)].set(get("Attack"), get("Decay"), get("Sustain"), get("Release"), sampleRate);
        v.env[static_cast<size_t>(op)].noteOn();
    }
    auto get = [&](const char* id) { return params.getRawParameterValue(id)->load(); };
    v.filterEnv.set(get("filterAttack"), get("filterDecay"), get("filterSustain"), get("filterRelease"), sampleRate);
    v.filterEnv.noteOn();
}

void OperatorSynth::stopVoice(int note)
{
    for (auto& v : voices)
        if (v.note == note && v.held)
        {
            v.held = false;
            if (sustainPedal)
            {
                v.sustained = true;
                continue;
            }
            for (auto& e : v.env)
                e.noteOff();
            v.filterEnv.noteOff();
        }
}

void OperatorSynth::processBlock(juce::AudioBuffer<float>& buffer, juce::MidiBuffer& midi)
{
    juce::ScopedNoDenormals noDenormals;
    buffer.clear();
    auto value = [this](const juce::String& id) { return params.getRawParameterValue(id)->load(); };

    Snapshot s;
    s.algorithm = &algorithm(static_cast<int>(value("algorithm")));
    for (int op = 0; op < numOperators; ++op)
    {
        const auto i = static_cast<size_t>(op);
        s.on[i] = value(opId(op, "On")) > 0.5f;
        s.wave[i] = static_cast<int>(value(opId(op, "Wave")));
        const int coarse = static_cast<int>(value(opId(op, "Coarse")));
        s.ratio[i] = (coarse == 0 ? 0.5 : coarse) * (1.0 + value(opId(op, "Fine")) / 1000.0);
        s.level[i] = value(opId(op, "Level"));
        s.vel[i] = value(opId(op, "Vel"));
    }
    s.volume = value("volume");
    s.feedback = value("feedback");
    s.transpose = value("transpose");
    s.filterOn = value("filterOn") > 0.5f;
    s.filterType = static_cast<int>(value("filterType"));
    s.cutoff = value("cutoff");
    s.resonance = value("resonance");
    s.filterEnv = value("filterEnv");
    s.pitchEnvAmount = value("pitchEnvAmount");
    s.pitchEnvCoef = static_cast<float>(std::exp(-4.6 / (std::max(0.001f, value("pitchEnvDecay")) * sampleRate)));
    s.lfoAmount = value("lfoAmount");
    s.lfoDest = static_cast<int>(value("lfoDest"));
    s.lfoInc = value("lfoRate") / sampleRate;

    float* left = buffer.getWritePointer(0);
    float* right = buffer.getNumChannels() > 1 ? buffer.getWritePointer(1) : nullptr;
    int position = 0;
    for (const auto meta : midi)
    {
        const int offset = juce::jlimit(0, buffer.getNumSamples(), meta.samplePosition);
        renderVoices(left, right, position, offset, s);
        position = offset;
        const auto msg = meta.getMessage();
        if (msg.isNoteOn())
            startVoice(msg.getNoteNumber(), msg.getFloatVelocity());
        else if (msg.isNoteOff())
            stopVoice(msg.getNoteNumber());
        else if (msg.isSustainPedalOn())
            sustainPedal = true;
        else if (msg.isSustainPedalOff())
        {
            sustainPedal = false;
            for (auto& v : voices)
                if (v.sustained)
                {
                    v.sustained = false;
                    for (auto& e : v.env)
                        e.noteOff();
                    v.filterEnv.noteOff();
                }
        }
        else if (msg.isAllNotesOff() || msg.isAllSoundOff())
        {
            for (auto& v : voices)
                if (v.held)
                    stopVoice(v.note);
        }
    }
    renderVoices(left, right, position, buffer.getNumSamples(), s);

    int count = 0;
    for (auto& v : voices)
        count += v.active() ? 1 : 0;
    activeVoices = count;
}

void OperatorSynth::renderVoices(float* left, float* right, int start, int end, const Snapshot& s)
{
    if (end <= start)
        return;
    constexpr int controlRate = 16;
    constexpr double twoPi = juce::MathConstants<double>::twoPi;
    constexpr float modDepth = 4.0f;
    const auto& alg = *s.algorithm;
    int numCarriers = 0;
    for (int op = 0; op < numOperators; ++op)
        numCarriers += (alg.carriers >> op) & 1;
    const float carrierGain = 1.0f / (1.0f + 0.4f * static_cast<float>(numCarriers - 1));
    const float outputGain = s.volume * s.volume * 0.7f;
    const float k = 2.0f - 1.94f * s.resonance;

    // Global LFO is evaluated per sample in the loop below via lfoPhase.
    const double lfoStart = lfoPhase;

    for (auto& v : voices)
    {
        if (! v.active())
            continue;
        double lfo = lfoStart;
        std::array<double, numOperators> inc {};
        std::array<float, numOperators> velScale {};
        float fmScale = 1.0f, ampScale = 1.0f;
        float a1 = 0, a2 = 0, a3 = 0;
        for (int op = 0; op < numOperators; ++op)
            velScale[static_cast<size_t>(op)] = 1.0f - s.vel[static_cast<size_t>(op)] * (1.0f - v.velocity);

        for (int i = start; i < end; ++i)
        {
            if ((i - start) % controlRate == 0)
            {
                const float lfoValue = static_cast<float>(std::sin(lfo * twoPi)) * s.lfoAmount;
                float semis = s.transpose + v.pitchEnv * s.pitchEnvAmount;
                if (s.lfoDest == 0)
                    semis += lfoValue * 2.0f;
                fmScale = s.lfoDest == 3 ? 1.0f + lfoValue : 1.0f;
                ampScale = s.lfoDest == 2 ? 1.0f - 0.5f * (s.lfoAmount + lfoValue) : 1.0f;
                const double base = v.frequency * std::pow(2.0, semis / 12.0) / sampleRate;
                for (int op = 0; op < numOperators; ++op)
                    inc[static_cast<size_t>(op)] = base * s.ratio[static_cast<size_t>(op)];

                if (s.filterOn)
                {
                    float octaves = s.filterEnv * v.filterEnv.level * 6.0f;
                    if (s.lfoDest == 1)
                        octaves += lfoValue * 3.0f;
                    const double fc = juce::jlimit(20.0, sampleRate * 0.45, static_cast<double>(s.cutoff) * std::pow(2.0, octaves));
                    const float g = static_cast<float>(std::tan(juce::MathConstants<double>::pi * fc / sampleRate));
                    a1 = 1.0f / (1.0f + g * (g + k));
                    a2 = g * a1;
                    a3 = g * a2;
                }
            }

            std::array<float, numOperators> out {};
            for (int op = numOperators - 1; op >= 0; --op)
            {
                const auto o = static_cast<size_t>(op);
                const float envelope = v.env[o].next();
                if (! s.on[o])
                    continue;
                double pm = 0.0;
                for (int m = op + 1; m < numOperators; ++m)
                    if ((alg.modulators[o] >> m) & 1)
                        pm += out[static_cast<size_t>(m)] * modDepth * fmScale;
                if (op == numOperators - 1)
                    pm += s.feedback * (v.feedback1 + v.feedback2) * 0.5f * 3.0f;

                double p = v.phase[o] + pm / twoPi;
                p -= std::floor(p);
                float w = 0.0f;
                switch (s.wave[o])
                {
                    case 0: w = static_cast<float>(std::sin(p * twoPi)); break;
                    case 1: w = static_cast<float>(p < 0.5 ? 4.0 * p - 1.0 : 3.0 - 4.0 * p); break;
                    case 2: w = static_cast<float>(2.0 * p - 1.0); break;
                    case 3: w = p < 0.5 ? 1.0f : -1.0f; break;
                    default: w = static_cast<float>(nextRandom(v.noiseSeed) >> 8) / 8388608.0f - 1.0f; break;
                }
                out[o] = w * envelope * s.level[o] * velScale[o];
                v.phase[o] += inc[o];
                if (v.phase[o] >= 1.0)
                    v.phase[o] -= std::floor(v.phase[o]);
            }
            v.feedback2 = v.feedback1;
            v.feedback1 = out[3];

            float sample = 0.0f;
            for (int op = 0; op < numOperators; ++op)
                if ((alg.carriers >> op) & 1)
                    sample += out[static_cast<size_t>(op)];
            sample *= carrierGain;

            v.filterEnv.next();
            if (s.filterOn)
            {
                const float v3 = sample - v.ic2;
                const float v1 = a1 * v.ic1 + a2 * v3;
                const float v2 = v.ic2 + a2 * v.ic1 + a3 * v3;
                v.ic1 = 2.0f * v1 - v.ic1;
                v.ic2 = 2.0f * v2 - v.ic2;
                sample = s.filterType == 0 ? v2 : (s.filterType == 1 ? sample - k * v1 - v2 : v1 * k);
            }
            v.pitchEnv *= s.pitchEnvCoef;
            lfo += s.lfoInc;

            sample *= outputGain * ampScale;
            left[i] += sample;
            if (right != nullptr)
                right[i] += sample;
        }

        // A voice is finished once all of its carriers are silent.
        bool carrierActive = false;
        for (int op = 0; op < numOperators; ++op)
            if (((alg.carriers >> op) & 1) && s.on[static_cast<size_t>(op)] && v.env[static_cast<size_t>(op)].active())
                carrierActive = true;
        if (! carrierActive)
        {
            for (auto& e : v.env)
                e.stage = Envelope::Stage::idle;
            v.note = -1;
            v.held = false;
        }
    }

    lfoPhase = lfoStart + s.lfoInc * (end - start);
    lfoPhase -= std::floor(lfoPhase);
}

} // namespace az

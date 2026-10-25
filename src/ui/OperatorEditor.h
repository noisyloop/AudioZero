#pragma once
#include "Theme.h"

namespace az
{

class OperatorSynth;

class OperatorEditor final : public juce::AudioProcessorEditor, private juce::Timer
{
public:
    explicit OperatorEditor(OperatorSynth&);
    ~OperatorEditor() override;
    void paint(juce::Graphics&) override;
    void resized() override;

    static juce::Colour operatorColour(int op);

private:
    void timerCallback() override;
    class AlgorithmView;
    class EnvelopeView;
    struct OperatorPanel;

    juce::SharedResourcePointer<Theme> lookAndFeel;
    OperatorSynth& synth;
    juce::ComboBox presetBox;
    juce::Label title;
    std::unique_ptr<AlgorithmView> algorithmView;
    juce::TextButton prevAlgorithm { "<" }, nextAlgorithm { ">" };
    std::unique_ptr<Knob> volume, transpose, feedback;
    std::vector<std::unique_ptr<OperatorPanel>> operators;
    std::unique_ptr<ParamToggle> filterOn;
    std::unique_ptr<ParamChoice> filterType, lfoDest;
    std::unique_ptr<Knob> cutoff, resonance, filterEnv, filterAttack, filterDecay, filterSustain, filterRelease;
    std::unique_ptr<Knob> lfoRate, lfoAmount, pitchAmount, pitchDecay;
    std::unique_ptr<EnvelopeView> filterEnvView;
    juce::Rectangle<int> algorithmArea, filterArea, modArea;
    std::vector<juce::Rectangle<int>> operatorAreas;
    juce::String shownPreset;
};

} // namespace az

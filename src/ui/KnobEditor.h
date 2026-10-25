#pragma once
#include "Theme.h"

namespace az
{

// Compact editor for the built-in effects: one control per parameter in a single row.
class KnobEditor final : public juce::AudioProcessorEditor
{
public:
    KnobEditor(juce::AudioProcessor& processor, juce::AudioProcessorValueTreeState& state)
        : AudioProcessorEditor(processor)
    {
        setLookAndFeel(&lookAndFeel.get());
        for (auto* p : processor.getParameters())
        {
            auto* ranged = dynamic_cast<juce::RangedAudioParameter*>(p);
            if (ranged == nullptr)
                continue;
            std::unique_ptr<juce::Component> control;
            if (dynamic_cast<juce::AudioParameterBool*>(ranged) != nullptr)
                control = std::make_unique<ParamToggle>(*ranged, ranged->getName(12));
            else if (dynamic_cast<juce::AudioParameterChoice*>(ranged) != nullptr)
                control = std::make_unique<ParamChoice>(*ranged);
            else
                control = std::make_unique<Knob>(*ranged, ranged->getName(12), theme::accent2);
            addAndMakeVisible(*control);
            controls.push_back(std::move(control));
        }
        juce::ignoreUnused(state);
        setSize(std::max(220, columns() * 66 + 12), 120);
    }

    ~KnobEditor() override { setLookAndFeel(nullptr); }

    void paint(juce::Graphics& g) override { g.fillAll(theme::bg1); }

    void resized() override
    {
        auto area = getLocalBounds().reduced(6);
        const int rows = (static_cast<int>(controls.size()) + columns() - 1) / columns();
        const int rowHeight = std::min(70, area.getHeight() / std::max(1, rows));
        for (size_t i = 0; i < controls.size(); ++i)
        {
            const int col = static_cast<int>(i) % columns(), row = static_cast<int>(i) / columns();
            auto cell = juce::Rectangle<int>(area.getX() + col * 66, area.getY() + row * rowHeight, 66, rowHeight).reduced(4);
            if (dynamic_cast<Knob*>(controls[i].get()) == nullptr)
                cell = cell.withSizeKeepingCentre(cell.getWidth(), 24);
            controls[i]->setBounds(cell);
        }
    }

private:
    int columns() const { return std::min(5, std::max(1, static_cast<int>(controls.size()))); }
    juce::SharedResourcePointer<Theme> lookAndFeel;
    std::vector<std::unique_ptr<juce::Component>> controls;
};

} // namespace az

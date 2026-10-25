#pragma once
#include <juce_gui_basics/juce_gui_basics.h>
#include <juce_audio_processors/juce_audio_processors.h>

namespace az
{

namespace theme
{
    const juce::Colour bg0 { 0xff121418 };   // window background / timeline
    const juce::Colour bg1 { 0xff1a1d22 };   // panels
    const juce::Colour bg2 { 0xff22262d };   // raised controls
    const juce::Colour bg3 { 0xff2b3039 };   // hover
    const juce::Colour line { 0xff333944 };
    const juce::Colour grid { 0xff20242b };
    const juce::Colour text { 0xffe1e5ec };
    const juce::Colour textDim { 0xff8b94a5 };
    const juce::Colour accent { 0xffff8a3d };
    const juce::Colour accent2 { 0xff38d9c6 };
    const juce::Colour danger { 0xffff5d6c };
    const juce::Colour record { 0xffff4458 };

    inline juce::Font font(float size, bool bold = false)
    {
        return juce::Font(juce::FontOptions(size, bold ? juce::Font::bold : juce::Font::plain));
    }
}

class Theme final : public juce::LookAndFeel_V4
{
public:
    Theme();
    void drawRotarySlider(juce::Graphics&, int x, int y, int w, int h, float pos, float start, float end, juce::Slider&) override;
    void drawLinearSlider(juce::Graphics&, int x, int y, int w, int h, float pos, float min, float max,
                          juce::Slider::SliderStyle, juce::Slider&) override;
    void drawButtonBackground(juce::Graphics&, juce::Button&, const juce::Colour&, bool highlighted, bool down) override;
    void drawButtonText(juce::Graphics&, juce::TextButton&, bool highlighted, bool down) override;
    void drawToggleButton(juce::Graphics&, juce::ToggleButton&, bool highlighted, bool down) override;
    void drawComboBox(juce::Graphics&, int w, int h, bool down, int bx, int by, int bw, int bh, juce::ComboBox&) override;
    juce::Font getComboBoxFont(juce::ComboBox&) override { return theme::font(13.0f); }
    juce::Font getTextButtonFont(juce::TextButton&, int) override { return theme::font(13.0f); }
    juce::Font getPopupMenuFont() override { return theme::font(14.0f); }
    juce::Font getLabelFont(juce::Label&) override { return theme::font(13.0f); }
    void drawScrollbar(juce::Graphics&, juce::ScrollBar&, int x, int y, int w, int h, bool vertical, int thumbStart,
                       int thumbSize, bool over, bool down) override;
    int getDefaultScrollbarWidth() override { return 10; }
    void drawTabButton(juce::TabBarButton&, juce::Graphics&, bool over, bool down) override;
    int getTabButtonBestWidth(juce::TabBarButton&, int) override { return 110; }
    void drawTabAreaBehindFrontButton(juce::TabbedButtonBar&, juce::Graphics&, int, int) override {}
};

// Rotary knob bound to a plugin parameter, with its name drawn underneath.
class Knob final : public juce::Component
{
public:
    Knob(juce::RangedAudioParameter& parameter, const juce::String& label = {}, juce::Colour colour = theme::accent);
    void resized() override;
    void paint(juce::Graphics&) override;
    juce::Slider slider;

private:
    juce::String label;
    std::unique_ptr<juce::SliderParameterAttachment> attachment;
};

// Small labelled toggle bound to a bool parameter.
class ParamToggle final : public juce::Component
{
public:
    ParamToggle(juce::RangedAudioParameter& parameter, const juce::String& label);
    void resized() override { button.setBounds(getLocalBounds()); }
    juce::TextButton button;

private:
    std::unique_ptr<juce::ButtonParameterAttachment> attachment;
};

// Combo box bound to a choice parameter.
class ParamChoice final : public juce::Component
{
public:
    explicit ParamChoice(juce::RangedAudioParameter& parameter);
    void resized() override { box.setBounds(getLocalBounds()); }
    juce::ComboBox box;

private:
    std::unique_ptr<juce::ComboBoxParameterAttachment> attachment;
};

} // namespace az

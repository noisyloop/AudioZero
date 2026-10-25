#pragma once
#include "Session.h"
#include "Theme.h"

namespace az
{

// Button drawn with a vector glyph instead of text.
class IconButton final : public juce::Button
{
public:
    enum class Glyph { play, stop, record, loop, metronome, toStart, keyboard, follow };
    IconButton(Glyph g, const juce::String& tooltip, juce::Colour onColour = theme::accent);
    void paintButton(juce::Graphics&, bool highlighted, bool down) override;

private:
    Glyph glyph;
    juce::Colour onColour;
};

// Numeric field you can drag vertically or double-click to type into.
class DragNumber final : public juce::Component, public juce::SettableTooltipClient
{
public:
    DragNumber(double minValue, double maxValue, int decimals, const juce::String& suffix = {});
    void setValue(double v);
    double getValue() const { return value; }
    std::function<void(double)> onChange;
    std::function<void()> onDragStart;

    void paint(juce::Graphics&) override;
    void resized() override { editor.setBounds(getLocalBounds()); }
    void mouseDown(const juce::MouseEvent&) override;
    void mouseDrag(const juce::MouseEvent&) override;
    void mouseDoubleClick(const juce::MouseEvent&) override;

private:
    double value = 0, minimum, maximum, dragStart = 0;
    int decimals;
    juce::String suffix;
    juce::Label editor;
};

class TransportBar final : public juce::Component
{
public:
    explicit TransportBar(Session&);
    void paint(juce::Graphics&) override;
    void resized() override;
    void refresh(); // called by the main timer
    std::function<void()> onOpenSettings;
    bool follow() const { return followButton.getToggleState(); }
    bool keyboardEnabled() const { return keyboardButton.getToggleState(); }
    int octave = 4;
    float keyboardVelocity = 0.8f;

private:
    Session& session;
    juce::Image logo;
    IconButton toStart { IconButton::Glyph::toStart, "Go to start (Home)" };
    IconButton playButton { IconButton::Glyph::play, "Play / Pause (Space)", theme::accent2 };
    IconButton stopButton { IconButton::Glyph::stop, "Stop. Press again to return to the start" };
    IconButton recordButton { IconButton::Glyph::record, "Record MIDI into the selected or armed track (R)", theme::record };
    IconButton loopButton { IconButton::Glyph::loop, "Loop on/off (Cmd/Ctrl+L)" };
    IconButton metronomeButton { IconButton::Glyph::metronome, "Metronome" };
    IconButton followButton { IconButton::Glyph::follow, "Follow playhead" };
    IconButton keyboardButton { IconButton::Glyph::keyboard, "Computer MIDI keyboard: play notes with A-K, Z/X octave" };
    DragNumber bpm { 20.0, 999.0, 2, " BPM" };
    DragNumber signature { 1.0, 16.0, 0, " / 4" };
    juce::ComboBox snapBox;
    juce::String positionText, timeText, cpuText, octaveText;
    float meterL = 0, meterR = 0;
    juce::Rectangle<int> positionArea, meterArea, cpuArea, logoArea, octaveArea;
};

} // namespace az

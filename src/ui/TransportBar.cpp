#include "TransportBar.h"
#include "BinaryData.h"

namespace az
{

IconButton::IconButton(Glyph g, const juce::String& tooltip, juce::Colour on)
    : Button(tooltip), glyph(g), onColour(on)
{
    setTooltip(tooltip);
}

void IconButton::paintButton(juce::Graphics& g, bool highlighted, bool down)
{
    auto bounds = getLocalBounds().toFloat().reduced(1.0f);
    const bool on = getToggleState();
    g.setColour(on ? onColour.withAlpha(0.2f) : (highlighted ? theme::bg3 : theme::bg2));
    g.fillRoundedRectangle(bounds, 5.0f);
    g.setColour(on ? onColour : theme::line);
    g.drawRoundedRectangle(bounds, 5.0f, 1.0f);

    auto c = bounds.getCentre();
    const float s = std::min(bounds.getWidth(), bounds.getHeight()) * 0.28f;
    g.setColour(on ? onColour : (down ? theme::accent : theme::text));
    juce::Path p;
    switch (glyph)
    {
        case Glyph::play:
            if (on)
            {
                g.fillRect(juce::Rectangle<float>(s * 0.55f, s * 2.0f).withCentre({ c.x - s * 0.45f, c.y }));
                g.fillRect(juce::Rectangle<float>(s * 0.55f, s * 2.0f).withCentre({ c.x + s * 0.45f, c.y }));
            }
            else
            {
                p.addTriangle(c.x - s * 0.7f, c.y - s, c.x - s * 0.7f, c.y + s, c.x + s, c.y);
                g.fillPath(p);
            }
            break;
        case Glyph::stop:
            g.fillRoundedRectangle(juce::Rectangle<float>(s * 1.7f, s * 1.7f).withCentre(c), 2.0f);
            break;
        case Glyph::record:
            g.setColour(on ? onColour : theme::record.withAlpha(0.85f));
            g.fillEllipse(juce::Rectangle<float>(s * 1.9f, s * 1.9f).withCentre(c));
            break;
        case Glyph::toStart:
            g.fillRect(juce::Rectangle<float>(s * 0.35f, s * 1.8f).withCentre({ c.x - s * 0.8f, c.y }));
            p.addTriangle(c.x + s * 0.8f, c.y - s * 0.9f, c.x + s * 0.8f, c.y + s * 0.9f, c.x - s * 0.5f, c.y);
            g.fillPath(p);
            break;
        case Glyph::loop:
        {
            p.addCentredArc(c.x, c.y, s * 1.1f, s * 0.8f, 0.0f, 0.4f, juce::MathConstants<float>::twoPi - 0.4f, true);
            g.strokePath(p, juce::PathStrokeType(1.8f));
            juce::Path head;
            const auto tip = juce::Point<float>(c.x + std::sin(0.4f) * s * 1.1f, c.y - std::cos(0.4f) * s * 0.8f);
            head.addTriangle(tip.x - 1, tip.y - 4, tip.x - 1, tip.y + 4, tip.x + 5, tip.y);
            g.fillPath(head);
            break;
        }
        case Glyph::metronome:
            p.addTriangle(c.x - s * 0.8f, c.y + s, c.x + s * 0.8f, c.y + s, c.x, c.y - s);
            g.strokePath(p, juce::PathStrokeType(1.6f));
            g.drawLine(c.x, c.y + s * 0.6f, c.x + s * 0.8f, c.y - s * 0.9f, 1.6f);
            break;
        case Glyph::keyboard:
        {
            auto r = juce::Rectangle<float>(s * 2.6f, s * 1.7f).withCentre(c);
            g.drawRoundedRectangle(r, 2.0f, 1.4f);
            for (int i = 1; i < 4; ++i)
                g.drawLine(r.getX() + r.getWidth() * i / 4.0f, r.getY(), r.getX() + r.getWidth() * i / 4.0f, r.getBottom(), 1.0f);
            for (int i : { 1, 2 })
                g.fillRect(juce::Rectangle<float>(3.0f, r.getHeight() * 0.55f).withPosition(r.getX() + r.getWidth() * i / 4.0f - 1.5f, r.getY()));
            break;
        }
        case Glyph::follow:
            p.startNewSubPath(c.x - s, c.y);
            p.lineTo(c.x + s, c.y);
            p.startNewSubPath(c.x + s * 0.3f, c.y - s * 0.7f);
            p.lineTo(c.x + s, c.y);
            p.lineTo(c.x + s * 0.3f, c.y + s * 0.7f);
            g.strokePath(p, juce::PathStrokeType(1.8f));
            g.drawLine(c.x - s, c.y - s, c.x - s, c.y + s, 1.8f);
            break;
    }
}

//==============================================================================
DragNumber::DragNumber(double minValue, double maxValue, int dp, const juce::String& sfx)
    : minimum(minValue), maximum(maxValue), decimals(dp), suffix(sfx)
{
    editor.setEditable(false, true, false);
    editor.setJustificationType(juce::Justification::centred);
    editor.setFont(theme::font(14.0f, true));
    editor.setVisible(false);
    editor.onTextChange = [this] {
        setValue(editor.getText().getDoubleValue());
        if (onChange)
            onChange(value);
    };
    editor.onEditorHide = [this] { editor.setVisible(false); };
    addChildComponent(editor);
    setMouseCursor(juce::MouseCursor::UpDownResizeCursor);
    setTooltip("Drag up/down or double-click to type");
}

void DragNumber::setValue(double v)
{
    v = juce::jlimit(minimum, maximum, v);
    if (v != value)
    {
        value = v;
        repaint();
    }
}

void DragNumber::paint(juce::Graphics& g)
{
    auto bounds = getLocalBounds().toFloat().reduced(0.5f);
    g.setColour(theme::bg0);
    g.fillRoundedRectangle(bounds, 4.0f);
    g.setColour(theme::line);
    g.drawRoundedRectangle(bounds, 4.0f, 1.0f);
    g.setColour(theme::text);
    g.setFont(theme::font(14.0f, true));
    g.drawText(juce::String(value, decimals) + suffix, getLocalBounds(), juce::Justification::centred);
}

void DragNumber::mouseDown(const juce::MouseEvent&)
{
    dragStart = value;
    if (onDragStart)
        onDragStart();
}

void DragNumber::mouseDrag(const juce::MouseEvent& e)
{
    const double step = decimals > 0 ? (e.mods.isShiftDown() ? 0.01 : 0.5) : 0.1;
    const double next = decimals > 0 ? dragStart - e.getDistanceFromDragStartY() * step
                                     : std::round(dragStart - e.getDistanceFromDragStartY() * step);
    const double old = value;
    setValue(decimals > 0 ? std::round(next * 100.0) / 100.0 : next);
    if (value != old && onChange)
        onChange(value);
}

void DragNumber::mouseDoubleClick(const juce::MouseEvent&)
{
    editor.setText(juce::String(value, decimals), juce::dontSendNotification);
    editor.setVisible(true);
    editor.showEditor();
}

//==============================================================================
TransportBar::TransportBar(Session& s) : session(s)
{
    logo = juce::ImageCache::getFromMemory(BinaryData::icon_png, BinaryData::icon_pngSize);
    for (auto* b : { &toStart, &playButton, &stopButton, &recordButton, &loopButton, &metronomeButton, &followButton, &keyboardButton })
    {
        b->setWantsKeyboardFocus(false);
        addAndMakeVisible(b);
    }
    followButton.setClickingTogglesState(true);
    followButton.setToggleState(true, juce::dontSendNotification);
    keyboardButton.setClickingTogglesState(true);
    keyboardButton.setToggleState(true, juce::dontSendNotification);

    toStart.onClick = [this] { session.engine.setPlayhead(session.project.loopEnabled && session.engine.isPlaying() ? session.project.loopStart : 0.0); };
    playButton.onClick = [this] { session.engine.togglePlay(); };
    stopButton.onClick = [this] { session.engine.stop(); };
    recordButton.onClick = [this] { session.toggleRecord(); };
    loopButton.onClick = [this] {
        session.edit([this] { session.project.loopEnabled = ! session.project.loopEnabled; });
    };
    metronomeButton.onClick = [this] { session.engine.setMetronome(! session.engine.isMetronomeOn()); };

    bpm.onDragStart = [this] { session.project.checkpoint(); };
    bpm.onChange = [this](double v) { session.edit([this, v] { session.project.bpm = v; }, false); };
    signature.onDragStart = [this] { session.project.checkpoint(); };
    signature.onChange = [this](double v) { session.edit([this, v] { session.project.beatsPerBar = static_cast<int>(v); }, false); };
    addAndMakeVisible(bpm);
    addAndMakeVisible(signature);

    snapBox.addItemList({ "Grid: Bar", "Grid: 1/2", "Grid: Beat", "Grid: 1/8", "Grid: 1/16", "Grid: Off" }, 1);
    snapBox.setSelectedId(3, juce::dontSendNotification);
    snapBox.setTooltip("Snap clips to the grid");
    snapBox.onChange = [this] {
        static const double values[] = { -1.0, 2.0, 1.0, 0.5, 0.25, 0.0 };
        const int index = snapBox.getSelectedItemIndex();
        session.snapBeats = index == 0 ? session.project.beatsPerBar : values[index];
        session.sendChangeMessage();
    };
    snapBox.setWantsKeyboardFocus(false);
    addAndMakeVisible(snapBox);
    refresh();
}

void TransportBar::refresh()
{
    auto& p = session.project;
    playButton.setToggleState(session.engine.isPlaying(), juce::dontSendNotification);
    recordButton.setToggleState(session.engine.isRecording(), juce::dontSendNotification);
    loopButton.setToggleState(p.loopEnabled, juce::dontSendNotification);
    metronomeButton.setToggleState(session.engine.isMetronomeOn(), juce::dontSendNotification);
    bpm.setValue(p.bpm);
    signature.setValue(p.beatsPerBar);

    const double beat = session.engine.getPlayhead();
    const int bar = static_cast<int>(beat / p.beatsPerBar) + 1;
    const double inBar = std::fmod(beat, static_cast<double>(p.beatsPerBar));
    const int beatInBar = static_cast<int>(inBar) + 1;
    const int sixteenth = static_cast<int>((inBar - std::floor(inBar)) * 4.0) + 1;
    const double seconds = beat * 60.0 / p.bpm;
    auto newPosition = juce::String(bar).paddedLeft(' ', 3) + " . " + juce::String(beatInBar) + " . " + juce::String(sixteenth);
    auto newTime = juce::String(static_cast<int>(seconds) / 60) + ":" + juce::String(std::fmod(seconds, 60.0), 2).paddedLeft('0', 5);
    auto newCpu = "CPU " + juce::String(juce::roundToInt(session.deviceManager.getCpuUsage() * 100.0)) + "%";
    auto newOctave = keyboardButton.getToggleState() ? "C" + juce::String(octave) : juce::String("off");
    const float l = p.masterMeterL.load(), r = p.masterMeterR.load();
    if (newPosition != positionText || newTime != timeText || newCpu != cpuText || newOctave != octaveText
        || std::abs(l - meterL) > 0.003f || std::abs(r - meterR) > 0.003f)
    {
        positionText = newPosition;
        timeText = newTime;
        cpuText = newCpu;
        octaveText = newOctave;
        meterL = l;
        meterR = r;
        repaint(positionArea);
        repaint(meterArea);
        repaint(cpuArea);
        repaint(octaveArea);
    }
}

void TransportBar::paint(juce::Graphics& g)
{
    g.setGradientFill(juce::ColourGradient(theme::bg2, 0, 0, theme::bg1, 0, static_cast<float>(getHeight()), false));
    g.fillAll();
    g.setColour(theme::line);
    g.drawLine(0, static_cast<float>(getHeight()) - 0.5f, static_cast<float>(getWidth()), static_cast<float>(getHeight()) - 0.5f);

    auto logoBounds = logoArea;
    if (logo.isValid())
        g.drawImage(logo, logoBounds.removeFromLeft(logoBounds.getHeight()).toFloat().reduced(1.0f), juce::RectanglePlacement::centred);
    auto text = logoBounds.withTrimmedLeft(6);
    g.setColour(theme::text);
    g.setFont(theme::font(17.0f, true));
    g.drawText("AudioZero", text, juce::Justification::centredLeft);

    auto pos = positionArea.toFloat();
    g.setColour(theme::bg0);
    g.fillRoundedRectangle(pos, 5.0f);
    g.setColour(theme::line);
    g.drawRoundedRectangle(pos.reduced(0.5f), 5.0f, 1.0f);
    g.setColour(theme::accent);
    g.setFont(juce::Font(juce::FontOptions(juce::Font::getDefaultMonospacedFontName(), 17.0f, juce::Font::bold)));
    g.drawText(positionText, positionArea.withTrimmedRight(positionArea.getWidth() / 2 - 8), juce::Justification::centred);
    g.setColour(theme::textDim);
    g.setFont(juce::Font(juce::FontOptions(juce::Font::getDefaultMonospacedFontName(), 14.0f, juce::Font::plain)));
    g.drawText(timeText, positionArea.withTrimmedLeft(positionArea.getWidth() / 2), juce::Justification::centred);

    g.setFont(theme::font(11.0f));
    g.setColour(theme::textDim);
    g.drawText(cpuText, cpuArea, juce::Justification::centredRight);
    g.drawText("Oct " + octaveText, octaveArea, juce::Justification::centredLeft);

    // Master meter
    auto m = meterArea.toFloat();
    g.setColour(theme::bg0);
    g.fillRoundedRectangle(m, 3.0f);
    auto drawBar = [&](juce::Rectangle<float> r, float level) {
        const float db = juce::Decibels::gainToDecibels(level, -60.0f);
        const float fill = juce::jlimit(0.0f, 1.0f, (db + 60.0f) / 63.0f);
        g.setColour(level > 0.99f ? theme::danger : theme::accent2);
        g.fillRoundedRectangle(r.withWidth(r.getWidth() * fill), 2.0f);
    };
    auto top = m.reduced(3.0f).removeFromTop(m.getHeight() * 0.5f - 3.5f);
    drawBar(top, meterL);
    drawBar(top.translated(0, top.getHeight() + 1.0f), meterR);
}

void TransportBar::resized()
{
    auto area = getLocalBounds().reduced(8, 7);
    logoArea = area.removeFromLeft(140);
    area.removeFromLeft(10);
    const int b = area.getHeight();
    for (auto* button : { &toStart, &playButton, &stopButton, &recordButton })
    {
        button->setBounds(area.removeFromLeft(b + 8));
        area.removeFromLeft(4);
    }
    area.removeFromLeft(8);
    bpm.setBounds(area.removeFromLeft(108));
    area.removeFromLeft(6);
    signature.setBounds(area.removeFromLeft(62));
    area.removeFromLeft(10);
    positionArea = area.removeFromLeft(230);
    area.removeFromLeft(10);
    for (auto* button : { &loopButton, &metronomeButton, &followButton })
    {
        button->setBounds(area.removeFromLeft(b + 4));
        area.removeFromLeft(4);
    }
    area.removeFromLeft(6);
    snapBox.setBounds(area.removeFromLeft(110).reduced(0, 3));
    area.removeFromLeft(10);
    keyboardButton.setBounds(area.removeFromLeft(b + 8));
    octaveArea = area.removeFromLeft(56).withTrimmedLeft(6);

    meterArea = area.removeFromRight(120).reduced(0, 8);
    area.removeFromRight(8);
    cpuArea = area.removeFromRight(70);
}

} // namespace az

#include "MixerView.h"

namespace az
{

static constexpr int stripWidth = 92;

class MixerView::Strip final : public juce::Component
{
public:
    // trackId 0 = master
    Strip(Session& s, int id) : session(s), trackId(id)
    {
        fader.setSliderStyle(juce::Slider::LinearVertical);
        fader.setTextBoxStyle(juce::Slider::NoTextBox, false, 0, 0);
        fader.setRange(0.0, 1.25);
        fader.setSkewFactor(0.6);
        fader.setDoubleClickReturnValue(true, 0.8);
        fader.setPopupDisplayEnabled(true, true, nullptr);
        fader.textFromValueFunction = [](double v) { return juce::Decibels::toString(juce::Decibels::gainToDecibels(v)); };
        fader.onDragStart = [this] { session.project.checkpoint(); };
        fader.onValueChange = [this] {
            const float v = static_cast<float>(fader.getValue());
            if (trackId == 0)
                session.edit([this, v] { session.project.masterVolume = v; }, false);
            else if (auto t = track())
                session.edit([t, v] { t->data.volume = v; }, false);
        };
        addAndMakeVisible(fader);

        if (trackId != 0)
        {
            pan.setSliderStyle(juce::Slider::RotaryHorizontalVerticalDrag);
            pan.setTextBoxStyle(juce::Slider::NoTextBox, false, 0, 0);
            pan.setRange(-1.0, 1.0);
            pan.setDoubleClickReturnValue(true, 0.0);
            pan.setPopupDisplayEnabled(true, true, nullptr);
            pan.setTooltip("Pan");
            pan.onDragStart = [this] { session.project.checkpoint(); };
            pan.onValueChange = [this] {
                const float v = static_cast<float>(pan.getValue());
                if (auto t = track())
                    session.edit([t, v] { t->data.pan = v; }, false);
            };
            addAndMakeVisible(pan);
            for (auto [b, text, colour] : { std::tuple { &mute, "M", theme::accent }, std::tuple { &solo, "S", juce::Colour(0xff5fb3ff) } })
            {
                b->setButtonText(text);
                b->setClickingTogglesState(true);
                b->setColour(juce::TextButton::buttonOnColourId, colour);
                b->setWantsKeyboardFocus(false);
                addAndMakeVisible(b);
            }
            mute.onClick = [this] { if (auto t = track()) session.edit([t, this] { t->data.mute = mute.getToggleState(); }); };
            solo.onClick = [this] { if (auto t = track()) session.edit([t, this] { t->data.solo = solo.getToggleState(); }); };
        }
        fader.setWantsKeyboardFocus(false);
        pan.setWantsKeyboardFocus(false);
        refresh();
    }

    std::shared_ptr<Track> track() const { return session.project.findTrack(trackId); }

    void refresh()
    {
        if (trackId == 0)
        {
            fader.setValue(session.project.masterVolume, juce::dontSendNotification);
            name = "Master";
            colour = theme::text;
        }
        else if (auto t = track())
        {
            fader.setValue(t->data.volume, juce::dontSendNotification);
            pan.setValue(t->data.pan, juce::dontSendNotification);
            mute.setToggleState(t->data.mute, juce::dontSendNotification);
            solo.setToggleState(t->data.solo, juce::dontSendNotification);
            name = t->data.name;
            colour = t->data.colour;
            fader.setColour(juce::Slider::trackColourId, colour);
            pan.setColour(juce::Slider::rotarySliderFillColourId, colour);
        }
        repaint();
    }

    void refreshMeter()
    {
        float l = 0, r = 0;
        if (trackId == 0)
        {
            l = session.project.masterMeterL.load();
            r = session.project.masterMeterR.load();
        }
        else if (auto t = track())
        {
            l = t->meterL.load();
            r = t->meterR.load();
        }
        if (std::abs(l - meterL) > 0.002f || std::abs(r - meterR) > 0.002f)
        {
            meterL = l;
            meterR = r;
            repaint(meterArea);
        }
    }

    void paint(juce::Graphics& g) override
    {
        const bool selected = trackId != 0 && session.selectedTrackId == trackId;
        auto bounds = getLocalBounds().toFloat().reduced(2.0f);
        g.setColour(selected ? theme::bg2 : theme::bg1);
        g.fillRoundedRectangle(bounds, 5.0f);
        g.setColour(colour);
        g.fillRoundedRectangle(bounds.removeFromTop(4.0f), 2.0f);
        g.setColour(theme::text);
        g.setFont(theme::font(12.0f, true));
        g.drawFittedText(name, getLocalBounds().withHeight(26).withTrimmedTop(6).reduced(4, 0), juce::Justification::centred, 1);

        auto m = meterArea.toFloat();
        g.setColour(theme::bg0);
        g.fillRoundedRectangle(m, 2.0f);
        auto bar = [&](juce::Rectangle<float> b, float level) {
            const float fill = juce::jlimit(0.0f, 1.0f, (juce::Decibels::gainToDecibels(level, -60.0f) + 60.0f) / 63.0f);
            juce::ColourGradient grad(theme::accent2, 0, b.getBottom(), theme::danger, 0, b.getY(), false);
            grad.addColour(0.8, juce::Colour(0xffffc94a));
            g.setGradientFill(grad);
            g.fillRect(b.withTop(b.getBottom() - b.getHeight() * fill));
        };
        auto inner = m.reduced(1.0f);
        bar(inner.removeFromLeft(inner.getWidth() * 0.5f - 0.5f), meterL);
        bar(inner.withTrimmedLeft(1.0f), meterR);

        g.setColour(theme::textDim);
        g.setFont(theme::font(10.0f));
        g.drawText(juce::Decibels::toString(juce::Decibels::gainToDecibels(fader.getValue()), 1),
                   getLocalBounds().removeFromBottom(18), juce::Justification::centred);
    }

    void resized() override
    {
        auto area = getLocalBounds().reduced(6);
        area.removeFromTop(24);
        if (trackId != 0)
        {
            pan.setBounds(area.removeFromTop(38).withSizeKeepingCentre(38, 38));
            auto buttons = area.removeFromTop(22);
            mute.setBounds(buttons.removeFromLeft(buttons.getWidth() / 2).reduced(2, 0));
            solo.setBounds(buttons.reduced(2, 0));
            area.removeFromTop(6);
        }
        area.removeFromBottom(14);
        meterArea = area.removeFromRight(14);
        fader.setBounds(area.reduced(4, 0));
    }

    void mouseDown(const juce::MouseEvent&) override
    {
        if (trackId != 0)
            session.selectTrack(trackId);
    }

    int trackId;

private:
    Session& session;
    juce::Slider fader, pan;
    juce::TextButton mute, solo;
    juce::String name;
    juce::Colour colour;
    juce::Rectangle<int> meterArea;
    float meterL = 0, meterR = 0;
};

MixerView::MixerView(Session& s) : session(s)
{
    viewport.setViewedComponent(&content, false);
    viewport.setScrollBarsShown(false, true);
    addAndMakeVisible(viewport);
    session.project.addChangeListener(this);
    session.addChangeListener(this);
    rebuild();
}

MixerView::~MixerView()
{
    session.project.removeChangeListener(this);
    session.removeChangeListener(this);
}

void MixerView::changeListenerCallback(juce::ChangeBroadcaster*)
{
    std::vector<int> ids;
    for (auto& t : session.project.tracks)
        ids.push_back(t->id);
    if (ids != shownIds)
        rebuild();
    else
        for (auto& s : strips)
            s->refresh();
}

void MixerView::rebuild()
{
    strips.clear();
    shownIds.clear();
    for (auto& t : session.project.tracks)
    {
        shownIds.push_back(t->id);
        strips.push_back(std::make_unique<Strip>(session, t->id));
    }
    strips.push_back(std::make_unique<Strip>(session, 0));
    for (auto& s : strips)
        content.addAndMakeVisible(*s);
    resized();
}

void MixerView::timerTick()
{
    if (! isShowing())
        return;
    for (auto& s : strips)
        s->refreshMeter();
}

void MixerView::paint(juce::Graphics& g)
{
    g.fillAll(theme::bg0);
}

void MixerView::resized()
{
    viewport.setBounds(getLocalBounds());
    const int h = getHeight() - 14;
    int x = 6;
    for (size_t i = 0; i < strips.size(); ++i)
    {
        if (i + 1 == strips.size())
            x += 12; // gap before master
        strips[i]->setBounds(x, 4, stripWidth, h);
        x += stripWidth + 4;
    }
    content.setSize(std::max(x, getWidth()), h + 8);
}

} // namespace az

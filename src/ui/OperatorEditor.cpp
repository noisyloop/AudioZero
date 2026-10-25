#include "OperatorEditor.h"
#include "../dsp/OperatorSynth.h"

namespace az
{

namespace
{
    const char* opLetters[] = { "A", "B", "C", "D" };
    juce::String opParam(int op, const char* name) { return juce::String("op") + opLetters[op] + name; }

    juce::RangedAudioParameter& param(OperatorSynth& synth, const juce::String& id)
    {
        auto* p = synth.params.getParameter(id);
        jassert(p != nullptr);
        return *p;
    }

    void drawSectionTitle(juce::Graphics& g, juce::Rectangle<int> area, const juce::String& text, juce::Colour colour)
    {
        g.setColour(theme::bg2);
        g.fillRoundedRectangle(area.toFloat(), 5.0f);
        g.setColour(colour);
        g.setFont(theme::font(11.0f, true));
        g.drawText(text, area.removeFromTop(18).reduced(8, 0), juce::Justification::centredLeft);
    }
}

juce::Colour OperatorEditor::operatorColour(int op)
{
    static const juce::Colour colours[] = { juce::Colour(0xffff8a3d), juce::Colour(0xffffc94a), juce::Colour(0xff38d9c6), juce::Colour(0xff8c7bff) };
    return colours[juce::jlimit(0, 3, op)];
}

//==============================================================================
class OperatorEditor::AlgorithmView final : public juce::Component, public juce::SettableTooltipClient
{
public:
    explicit AlgorithmView(OperatorSynth& s) : synth(s) { setTooltip("Algorithm: how operators modulate each other. Click to cycle."); }

    void paint(juce::Graphics& g) override
    {
        // Hand-placed (column, row) positions for each algorithm's operators A-D.
        static const float layout[OperatorSynth::numAlgorithms][4][2] = {
            { { 0, 3 }, { 0, 2 }, { 0, 1 }, { 0, 0 } },
            { { 0.5f, 3 }, { 0.5f, 2 }, { 0, 1 }, { 1, 1 } },
            { { 0.5f, 3 }, { 0, 2 }, { 1, 2 }, { 1, 1 } },
            { { 1, 3 }, { 0, 2 }, { 1, 2 }, { 2, 2 } },
            { { 0, 3 }, { 0, 2 }, { 1, 3 }, { 1, 2 } },
            { { 0, 3 }, { 1, 3 }, { 2, 3 }, { 1, 2 } },
            { { 0, 3 }, { 1, 3 }, { 2, 3 }, { 2, 2 } },
            { { 0, 3 }, { 1, 3 }, { 2, 3 }, { 3, 3 } },
        };
        const int index = currentIndex();
        const auto& alg = OperatorSynth::algorithm(index);
        auto area = getLocalBounds().toFloat().reduced(4.0f);
        g.setColour(theme::bg0);
        g.fillRoundedRectangle(area, 5.0f);

        float maxCol = 0, minRow = 3;
        for (auto& p : layout[index])
        {
            maxCol = std::max(maxCol, p[0]);
            minRow = std::min(minRow, p[1]);
        }
        const float box = 20.0f;
        const float rows = 3.0f - minRow;
        const float step = rows > 0 ? juce::jlimit(box + 6.0f, 34.0f, (area.getHeight() - 30.0f - box) / rows) : 30.0f;
        const float width = maxCol * step + box;
        const float height = rows * step + box + 10.0f;
        const float originX = area.getCentreX() - width * 0.5f;
        const float originY = area.getCentreY() + 6.0f - height * 0.5f - minRow * step;
        auto rectFor = [&](int op) {
            return juce::Rectangle<float>(originX + layout[index][op][0] * step, originY + layout[index][op][1] * step, box, box);
        };

        g.setColour(theme::textDim);
        for (int target = 0; target < 4; ++target)
            for (int mod = 0; mod < 4; ++mod)
                if ((alg.modulators[static_cast<size_t>(target)] >> mod) & 1)
                {
                    const auto from = rectFor(mod), to = rectFor(target);
                    g.drawLine(from.getCentreX(), from.getBottom(), to.getCentreX(), to.getY(), 1.5f);
                }
        const float outY = originY + 3 * step + box + 8.0f;
        float outLeft = 1.0e9f, outRight = -1.0e9f;
        for (int op = 0; op < 4; ++op)
            if ((alg.carriers >> op) & 1)
            {
                const auto r = rectFor(op);
                g.drawLine(r.getCentreX(), r.getBottom(), r.getCentreX(), outY, 1.5f);
                outLeft = std::min(outLeft, r.getCentreX());
                outRight = std::max(outRight, r.getCentreX());
            }
        g.setColour(theme::accent);
        g.drawLine(outLeft - 4.0f, outY, outRight + 4.0f, outY, 2.5f);

        // Feedback loop on D
        const auto d = rectFor(3);
        g.setColour(theme::textDim);
        juce::Path loop;
        loop.startNewSubPath(d.getRight(), d.getCentreY() + 4.0f);
        loop.lineTo(d.getRight() + 6.0f, d.getCentreY() + 4.0f);
        loop.lineTo(d.getRight() + 6.0f, d.getY() - 4.0f);
        loop.lineTo(d.getCentreX(), d.getY() - 4.0f);
        loop.lineTo(d.getCentreX(), d.getY());
        g.strokePath(loop, juce::PathStrokeType(1.2f));

        for (int op = 0; op < 4; ++op)
        {
            const auto r = rectFor(op);
            g.setColour(operatorColour(op));
            g.fillRoundedRectangle(r, 4.0f);
            g.setColour(juce::Colours::black.withAlpha(0.85f));
            g.setFont(theme::font(12.0f, true));
            g.drawText(opLetters[op], r, juce::Justification::centred);
        }
        g.setColour(theme::textDim);
        g.setFont(theme::font(11.0f));
        g.drawText("ALG " + juce::String(index + 1), area.removeFromTop(14.0f).reduced(6.0f, 0), juce::Justification::topLeft);
    }

    void mouseDown(const juce::MouseEvent& e) override
    {
        cycle(e.mods.isRightButtonDown() || e.mods.isShiftDown() ? -1 : 1);
    }

    void cycle(int delta)
    {
        auto& p = param(synth, "algorithm");
        const int next = (currentIndex() + delta + OperatorSynth::numAlgorithms) % OperatorSynth::numAlgorithms;
        p.beginChangeGesture();
        p.setValueNotifyingHost(p.convertTo0to1(static_cast<float>(next)));
        p.endChangeGesture();
        repaint();
    }

    int currentIndex() const { return static_cast<int>(synth.params.getRawParameterValue("algorithm")->load()); }

private:
    OperatorSynth& synth;
};

//==============================================================================
class OperatorEditor::EnvelopeView final : public juce::Component
{
public:
    EnvelopeView(OperatorSynth& s, juce::String a, juce::String d, juce::String su, juce::String r, juce::Colour c)
        : synth(s), ids { a, d, su, r }, colour(c) {}

    bool refresh()
    {
        std::array<float, 4> now {};
        for (size_t i = 0; i < 4; ++i)
            now[i] = synth.params.getRawParameterValue(ids[i])->load();
        if (now == values)
            return false;
        values = now;
        repaint();
        return true;
    }

    void paint(juce::Graphics& g) override
    {
        auto area = getLocalBounds().toFloat();
        g.setColour(theme::bg0);
        g.fillRoundedRectangle(area, 4.0f);
        area = area.reduced(4.0f, 5.0f);
        // Display times on a compressed scale so short and long envelopes both read well.
        auto t = [](float seconds) { return std::sqrt(std::max(0.0f, seconds)); };
        const float a = t(values[0]), d = t(values[1]), s = values[2], r = t(values[3]);
        const float hold = 0.6f;
        const float total = a + d + hold + r;
        const float scale = area.getWidth() / std::max(total, 0.001f);
        const float x0 = area.getX(), bottom = area.getBottom(), top = area.getY();
        const float xa = x0 + a * scale, xd = xa + d * scale, xs = xd + hold * scale, xr = xs + r * scale;
        const float ys = bottom - s * area.getHeight();
        juce::Path p;
        p.startNewSubPath(x0, bottom);
        p.lineTo(xa, top);
        p.quadraticTo(xa + (xd - xa) * 0.25f, ys, xd, ys);
        p.lineTo(xs, ys);
        p.quadraticTo(xs + (xr - xs) * 0.25f, bottom, xr, bottom);
        juce::Path fill(p);
        fill.lineTo(x0, bottom);
        fill.closeSubPath();
        g.setColour(colour.withAlpha(0.18f));
        g.fillPath(fill);
        g.setColour(colour);
        g.strokePath(p, juce::PathStrokeType(1.6f));
    }

private:
    OperatorSynth& synth;
    std::array<juce::String, 4> ids;
    std::array<float, 4> values { -1, -1, -1, -1 };
    juce::Colour colour;
};

//==============================================================================
struct OperatorEditor::OperatorPanel
{
    OperatorPanel(OperatorEditor& editor, OperatorSynth& synth, int op)
        : index(op),
          on(param(synth, opParam(op, "On")), opLetters[op]),
          wave(param(synth, opParam(op, "Wave"))),
          coarse(param(synth, opParam(op, "Coarse")), "Coarse", operatorColour(op)),
          fine(param(synth, opParam(op, "Fine")), "Fine", operatorColour(op)),
          level(param(synth, opParam(op, "Level")), "Level", operatorColour(op)),
          vel(param(synth, opParam(op, "Vel")), "Vel", operatorColour(op)),
          attack(param(synth, opParam(op, "Attack")), "Attack", operatorColour(op)),
          decay(param(synth, opParam(op, "Decay")), "Decay", operatorColour(op)),
          sustain(param(synth, opParam(op, "Sustain")), "Sustain", operatorColour(op)),
          release(param(synth, opParam(op, "Release")), "Release", operatorColour(op)),
          envelope(synth, opParam(op, "Attack"), opParam(op, "Decay"), opParam(op, "Sustain"), opParam(op, "Release"), operatorColour(op))
    {
        on.button.setColour(juce::TextButton::buttonOnColourId, operatorColour(op));
        on.button.setTooltip("Enable operator " + juce::String(opLetters[op]));
        for (juce::Component* c : std::initializer_list<juce::Component*> { &on, &wave, &coarse, &fine, &level, &vel, &attack, &decay, &sustain, &release, &envelope })
            editor.addAndMakeVisible(c);
    }

    void layout(juce::Rectangle<int> area)
    {
        area = area.reduced(6, 4);
        area.removeFromTop(16);
        auto header = area.removeFromTop(22);
        on.setBounds(header.removeFromLeft(30));
        header.removeFromLeft(6);
        wave.setBounds(header);
        area.removeFromTop(5);
        envelope.setBounds(area.removeFromTop(40));
        area.removeFromTop(4);
        const int knobWidth = area.getWidth() / 4;
        auto row1 = area.removeFromTop(area.getHeight() / 2);
        for (auto* k : { &coarse, &fine, &level, &vel })
            k->setBounds(row1.removeFromLeft(knobWidth));
        for (auto* k : { &attack, &decay, &sustain, &release })
            k->setBounds(area.removeFromLeft(knobWidth));
    }

    int index;
    ParamToggle on;
    ParamChoice wave;
    Knob coarse, fine, level, vel, attack, decay, sustain, release;
    EnvelopeView envelope;
};

//==============================================================================
OperatorEditor::OperatorEditor(OperatorSynth& s) : AudioProcessorEditor(s), synth(s)
{
    setLookAndFeel(&lookAndFeel.get());
    title.setText("OPERATOR", juce::dontSendNotification);
    title.setFont(theme::font(13.0f, true));
    title.setColour(juce::Label::textColourId, theme::accent);
    addAndMakeVisible(title);

    presetBox.addItemList(OperatorSynth::presetNames(), 1);
    presetBox.setTooltip("Preset");
    presetBox.onChange = [this] {
        const auto name = presetBox.getText();
        if (name.isNotEmpty() && name != synth.currentPreset)
            synth.applyPreset(name);
        shownPreset = synth.currentPreset;
    };
    addAndMakeVisible(presetBox);

    algorithmView = std::make_unique<AlgorithmView>(synth);
    addAndMakeVisible(*algorithmView);
    prevAlgorithm.onClick = [this] { algorithmView->cycle(-1); };
    nextAlgorithm.onClick = [this] { algorithmView->cycle(1); };
    addAndMakeVisible(prevAlgorithm);
    addAndMakeVisible(nextAlgorithm);

    auto knob = [this](const char* id, const char* label, juce::Colour c = theme::accent) {
        auto k = std::make_unique<Knob>(param(synth, id), label, c);
        addAndMakeVisible(*k);
        return k;
    };
    volume = knob("volume", "Volume");
    transpose = knob("transpose", "Transp.");
    feedback = knob("feedback", "Feedback", operatorColour(3));

    for (int op = 0; op < OperatorSynth::numOperators; ++op)
        operators.push_back(std::make_unique<OperatorPanel>(*this, synth, op));

    filterOn = std::make_unique<ParamToggle>(param(synth, "filterOn"), "Filter");
    filterOn->button.setColour(juce::TextButton::buttonOnColourId, theme::accent2);
    addAndMakeVisible(*filterOn);
    filterType = std::make_unique<ParamChoice>(param(synth, "filterType"));
    addAndMakeVisible(*filterType);
    cutoff = knob("cutoff", "Cutoff", theme::accent2);
    resonance = knob("resonance", "Res", theme::accent2);
    filterEnv = knob("filterEnv", "Env Amt", theme::accent2);
    filterAttack = knob("filterAttack", "Attack", theme::accent2);
    filterDecay = knob("filterDecay", "Decay", theme::accent2);
    filterSustain = knob("filterSustain", "Sustain", theme::accent2);
    filterRelease = knob("filterRelease", "Release", theme::accent2);
    filterEnvView = std::make_unique<EnvelopeView>(synth, "filterAttack", "filterDecay", "filterSustain", "filterRelease", theme::accent2);
    addAndMakeVisible(*filterEnvView);

    lfoRate = knob("lfoRate", "LFO Rate", juce::Colour(0xff8c7bff));
    lfoAmount = knob("lfoAmount", "LFO Amt", juce::Colour(0xff8c7bff));
    lfoDest = std::make_unique<ParamChoice>(param(synth, "lfoDest"));
    addAndMakeVisible(*lfoDest);
    pitchAmount = knob("pitchEnvAmount", "Pitch Env", juce::Colour(0xffff5d8f));
    pitchDecay = knob("pitchEnvDecay", "P. Decay", juce::Colour(0xffff5d8f));

    setSize(1290, 214);
    timerCallback();
    startTimerHz(15);
}

OperatorEditor::~OperatorEditor()
{
    stopTimer();
    setLookAndFeel(nullptr);
}

void OperatorEditor::timerCallback()
{
    for (auto& op : operators)
        op->envelope.refresh();
    filterEnvView->refresh();
    algorithmView->repaint();
    if (shownPreset != synth.currentPreset)
    {
        shownPreset = synth.currentPreset;
        presetBox.setText(shownPreset, juce::dontSendNotification);
    }
}

void OperatorEditor::paint(juce::Graphics& g)
{
    g.fillAll(theme::bg1);
    drawSectionTitle(g, algorithmArea, "ALGORITHM", theme::textDim);
    for (size_t i = 0; i < operatorAreas.size(); ++i)
        drawSectionTitle(g, operatorAreas[i], "OPERATOR " + juce::String(opLetters[i]), operatorColour(static_cast<int>(i)));
    drawSectionTitle(g, filterArea, "FILTER", theme::accent2);
    drawSectionTitle(g, modArea, "LFO / PITCH", juce::Colour(0xff8c7bff));
}

void OperatorEditor::resized()
{
    auto area = getLocalBounds().reduced(4);
    auto header = area.removeFromTop(24);
    title.setBounds(header.removeFromLeft(90));
    presetBox.setBounds(header.removeFromLeft(160).reduced(0, 1));
    area.removeFromTop(4);

    algorithmArea = area.removeFromLeft(196).reduced(3, 0);
    {
        auto a = algorithmArea.reduced(6, 4);
        a.removeFromTop(16);
        auto knobs = a.removeFromRight(52);
        const int h = std::min(52, knobs.getHeight() / 3);
        volume->setBounds(knobs.removeFromTop(h));
        transpose->setBounds(knobs.removeFromTop(h));
        feedback->setBounds(knobs.removeFromTop(h));
        auto arrows = a.removeFromBottom(20);
        prevAlgorithm.setBounds(arrows.removeFromLeft(24));
        nextAlgorithm.setBounds(arrows.removeFromRight(24));
        algorithmView->setBounds(a.withTrimmedBottom(2));
    }

    modArea = area.removeFromRight(150).reduced(3, 0);
    filterArea = area.removeFromRight(212).reduced(3, 0);
    {
        auto a = filterArea.reduced(6, 4);
        a.removeFromTop(16);
        auto filterHeader = a.removeFromTop(22);
        filterOn->setBounds(filterHeader.removeFromLeft(60));
        filterHeader.removeFromLeft(6);
        filterType->setBounds(filterHeader);
        a.removeFromTop(5);
        auto row1 = a.removeFromTop(52);
        const int w = row1.getWidth() / 4;
        cutoff->setBounds(row1.removeFromLeft(w));
        resonance->setBounds(row1.removeFromLeft(w));
        filterEnv->setBounds(row1.removeFromLeft(w));
        filterEnvView->setBounds(row1.reduced(2, 8));
        a.removeFromTop(2);
        auto row2 = a.removeFromTop(52);
        for (auto* k : { filterAttack.get(), filterDecay.get(), filterSustain.get(), filterRelease.get() })
            k->setBounds(row2.removeFromLeft(w));
    }
    {
        auto a = modArea.reduced(6, 4);
        a.removeFromTop(16);
        lfoDest->setBounds(a.removeFromTop(22));
        a.removeFromTop(5);
        auto row1 = a.removeFromTop(52);
        lfoRate->setBounds(row1.removeFromLeft(row1.getWidth() / 2));
        lfoAmount->setBounds(row1);
        a.removeFromTop(2);
        auto row2 = a.removeFromTop(52);
        pitchAmount->setBounds(row2.removeFromLeft(row2.getWidth() / 2));
        pitchDecay->setBounds(row2);
    }

    operatorAreas.clear();
    const int opWidth = area.getWidth() / OperatorSynth::numOperators;
    for (auto& op : operators)
    {
        auto r = area.removeFromLeft(opWidth).reduced(3, 0);
        operatorAreas.push_back(r);
        op->layout(r);
    }
}

} // namespace az

#include "PianoRoll.h"

namespace az
{

static bool isBlackKey(int pitch)
{
    const int n = pitch % 12;
    return n == 1 || n == 3 || n == 6 || n == 8 || n == 10;
}

PianoRoll::PianoRoll(Session& s) : session(s)
{
    gridBox.addItemList({ "Grid 1/4", "Grid 1/8", "Grid 1/16", "Grid 1/32", "Grid 1/8T", "Grid 1/16T", "Grid Off" }, 1);
    gridBox.setSelectedId(3, juce::dontSendNotification);
    gridBox.onChange = [this] { repaint(); };
    gridBox.setWantsKeyboardFocus(false);
    addAndMakeVisible(gridBox);

    quantizeButton.setTooltip("Snap selected notes (or all notes) to the grid (Q)");
    quantizeButton.onClick = [this] { quantize(); };
    quantizeButton.setWantsKeyboardFocus(false);
    addAndMakeVisible(quantizeButton);

    lengthBars.onDragStart = [this] { session.project.checkpoint(); };
    lengthBars.onChange = [this](double bars) {
        if (auto* c = clip())
        {
            const double beats = bars * session.project.beatsPerBar;
            session.edit([c, beats] { c->length = beats; }, false);
        }
    };
    addAndMakeVisible(lengthBars);

    for (auto* bar : { &vbar, &hbar })
    {
        bar->addListener(this);
        bar->setAutoHide(false);
        addAndMakeVisible(bar);
    }
    scrollY = (127 - 84) * keyHeight;
    session.project.addChangeListener(this);
    session.addChangeListener(this);
}

PianoRoll::~PianoRoll()
{
    stopAudition();
    session.project.removeChangeListener(this);
    session.removeChangeListener(this);
}

Clip* PianoRoll::clip() const
{
    auto* c = session.findClip(clipId);
    return c != nullptr && c->kind == ClipKind::midi ? c : nullptr;
}

void PianoRoll::changeListenerCallback(juce::ChangeBroadcaster*)
{
    if (session.editClipId != clipId)
    {
        clipId = session.editClipId;
        selection.clear();
        fitToClip();
    }
    if (auto* c = clip())
        lengthBars.setValue(c->length / session.project.beatsPerBar);
    updateScrollBars();
    repaint();
}

void PianoRoll::fitToClip()
{
    auto* c = clip();
    needsFit = c != nullptr && gridArea.getWidth() <= 0;
    if (c == nullptr || needsFit)
        return;
    scrollBeats = 0.0;
    pixelsPerBeat = juce::jlimit(8.0, 400.0, (gridArea.getWidth() - 20) / std::max(1.0, c->length));
    if (! c->notes.empty())
    {
        int lo = 127, hi = 0;
        for (auto& n : c->notes)
        {
            lo = std::min(lo, n.pitch);
            hi = std::max(hi, n.pitch);
        }
        const int centre = (lo + hi) / 2;
        scrollY = juce::jlimit(0, std::max(0, 128 * keyHeight - gridArea.getHeight()),
                               (127 - centre) * keyHeight - gridArea.getHeight() / 2);
    }
}

double PianoRoll::grid() const
{
    static const double values[] = { 1.0, 0.5, 0.25, 0.125, 1.0 / 3.0, 1.0 / 6.0, 0.0 };
    return values[juce::jlimit(0, 6, gridBox.getSelectedItemIndex())];
}

void PianoRoll::updateScrollBars()
{
    vbar.setRangeLimits(0.0, 128.0 * keyHeight, juce::dontSendNotification);
    vbar.setCurrentRange(scrollY, gridArea.getHeight(), juce::dontSendNotification);
    const double visible = gridArea.getWidth() / pixelsPerBeat;
    const double total = std::max((clip() != nullptr ? clip()->length : 4.0) + 8.0, scrollBeats + visible);
    hbar.setRangeLimits(0.0, total, juce::dontSendNotification);
    hbar.setCurrentRange(scrollBeats, visible, juce::dontSendNotification);
}

void PianoRoll::scrollBarMoved(juce::ScrollBar* bar, double start)
{
    if (bar == &vbar)
        scrollY = juce::roundToInt(start);
    else
        scrollBeats = std::max(0.0, start);
    repaint();
}

juce::Rectangle<float> PianoRoll::noteBounds(const Note& n) const
{
    const float x = beatToX(n.start);
    return { x, pitchToY(n.pitch) + 1.0f, std::max(3.0f, static_cast<float>(n.length * pixelsPerBeat) - 1.0f), keyHeight - 1.0f };
}

int PianoRoll::noteAt(juce::Point<float> p) const
{
    auto* c = clip();
    if (c == nullptr)
        return -1;
    for (int i = static_cast<int>(c->notes.size()) - 1; i >= 0; --i)
        if (noteBounds(c->notes[static_cast<size_t>(i)]).contains(p))
            return i;
    return -1;
}

void PianoRoll::audition(int pitch)
{
    if (auditioning == pitch)
        return;
    stopAudition();
    auditioning = pitch;
    session.noteOn(pitch, 0.8f);
}

void PianoRoll::stopAudition()
{
    if (auditioning >= 0)
        session.noteOff(auditioning);
    auditioning = -1;
}

//==============================================================================
void PianoRoll::paint(juce::Graphics& g)
{
    g.fillAll(theme::bg0);
    auto* c = clip();

    // Toolbar
    g.setColour(theme::bg1);
    g.fillRect(toolbar);
    g.setColour(theme::textDim);
    g.setFont(theme::font(11.0f));
    g.drawText("Click: add  |  Drag: move  |  Edge: resize  |  Right/double-click: delete  |  Shift-drag: select  |  Alt-drag: copy",
               toolbar.withTrimmedLeft(lengthBars.getRight() + 8).reduced(6, 0), juce::Justification::centredLeft, true);

    if (c == nullptr)
    {
        g.setColour(theme::textDim);
        g.setFont(theme::font(14.0f));
        g.drawText("Double-click a MIDI clip in the arrangement (or an empty lane to create one) to edit its notes here",
                   getLocalBounds().withTrimmedTop(toolbar.getHeight()), juce::Justification::centred);
        return;
    }
    if (auto track = session.project.trackOwningClip(clipId))
    {
        g.setColour(c->colour);
        g.fillRoundedRectangle(toolbar.withWidth(6).withX(6).reduced(0, 7).toFloat(), 2.0f);
        g.setColour(theme::text);
        g.setFont(theme::font(13.0f, true));
        g.drawText(track->data.name + "  /  " + c->name, toolbar.withX(16).withWidth(170), juce::Justification::centredLeft, true);
    }

    // Grid rows
    {
        juce::Graphics::ScopedSaveState state(g);
        g.reduceClipRegion(gridArea);
        for (int pitch = 0; pitch < 128; ++pitch)
        {
            const float y = pitchToY(pitch);
            if (y > gridArea.getBottom() || y + keyHeight < gridArea.getY())
                continue;
            g.setColour(isBlackKey(pitch) ? theme::bg0 : theme::bg1.withAlpha(0.7f));
            g.fillRect(static_cast<float>(gridArea.getX()), y, static_cast<float>(gridArea.getWidth()), static_cast<float>(keyHeight));
            if (pitch % 12 == 0)
            {
                g.setColour(theme::line);
                g.drawHorizontalLine(juce::roundToInt(y + keyHeight) - 1, static_cast<float>(gridArea.getX()), static_cast<float>(gridArea.getRight()));
            }
        }
        const double step = grid() > 0 ? grid() : 0.25;
        const double bar = session.project.beatsPerBar;
        double drawStep = step;
        while (drawStep * pixelsPerBeat < 6.0)
            drawStep *= 2.0;
        for (double b = std::floor(scrollBeats / drawStep) * drawStep; beatToX(b) < gridArea.getRight(); b += drawStep)
        {
            const bool isBar = std::fmod(b + 1e-9, bar) < 1e-6;
            const bool isBeat = std::fmod(b + 1e-9, 1.0) < 1e-6;
            g.setColour(isBar ? theme::line.brighter(0.2f) : isBeat ? theme::line : theme::grid.brighter(0.05f));
            g.drawVerticalLine(juce::roundToInt(beatToX(b)), static_cast<float>(gridArea.getY()), static_cast<float>(gridArea.getBottom()));
        }
        // Outside the clip
        const float endX = beatToX(c->length);
        g.setColour(juce::Colours::black.withAlpha(0.45f));
        g.fillRect(endX, static_cast<float>(gridArea.getY()), static_cast<float>(gridArea.getRight()) - endX, static_cast<float>(gridArea.getHeight()));
        g.setColour(c->colour);
        g.drawVerticalLine(juce::roundToInt(endX), static_cast<float>(gridArea.getY()), static_cast<float>(gridArea.getBottom()));

        for (size_t i = 0; i < c->notes.size(); ++i)
        {
            const auto& n = c->notes[i];
            auto r = noteBounds(n);
            const bool selected = selection.count(i) != 0;
            const auto base = c->colour.withMultipliedBrightness(0.55f + 0.45f * n.velocity);
            g.setColour(selected ? base.brighter(0.5f) : base);
            g.fillRoundedRectangle(r, 2.5f);
            g.setColour(selected ? theme::text : juce::Colours::black.withAlpha(0.5f));
            g.drawRoundedRectangle(r, 2.5f, selected ? 1.5f : 1.0f);
            if (r.getWidth() > 26)
            {
                g.setColour(juce::Colours::black.withAlpha(0.7f));
                g.setFont(theme::font(9.5f));
                g.drawText(juce::MidiMessage::getMidiNoteName(n.pitch, true, true, 4), r.reduced(3, 0), juce::Justification::centredLeft, false);
            }
        }
        if (mode == Mode::select)
        {
            g.setColour(theme::accent.withAlpha(0.15f));
            g.fillRect(rubberBand);
            g.setColour(theme::accent);
            g.drawRect(rubberBand, 1.0f);
        }
        // Playhead (only while it is inside this clip)
        const double rel = session.engine.getPlayhead() - c->start;
        if (rel >= 0 && rel <= c->length)
        {
            g.setColour(theme::text.withAlpha(0.8f));
            g.drawVerticalLine(juce::roundToInt(beatToX(rel)), static_cast<float>(gridArea.getY()), static_cast<float>(gridArea.getBottom()));
        }
    }

    // Keyboard
    {
        juce::Graphics::ScopedSaveState state(g);
        g.reduceClipRegion(keyboardArea);
        for (int pitch = 0; pitch < 128; ++pitch)
        {
            const float y = pitchToY(pitch);
            if (y > keyboardArea.getBottom() || y + keyHeight < keyboardArea.getY())
                continue;
            const bool black = isBlackKey(pitch);
            auto r = juce::Rectangle<float>(static_cast<float>(keyboardArea.getX()), y, static_cast<float>(keyboardArea.getWidth()), static_cast<float>(keyHeight));
            g.setColour(pitch == auditioning ? theme::accent : black ? juce::Colour(0xff2a2d33) : juce::Colour(0xffd9dde4));
            g.fillRect(black ? r.withWidth(r.getWidth() * 0.62f) : r);
            if (black)
            {
                g.setColour(juce::Colour(0xffd9dde4));
                g.fillRect(r.withTrimmedLeft(r.getWidth() * 0.62f));
            }
            g.setColour(juce::Colours::black.withAlpha(0.25f));
            g.drawHorizontalLine(juce::roundToInt(y + keyHeight) - 1, r.getX(), r.getRight());
            if (pitch % 12 == 0)
            {
                g.setColour(juce::Colour(0xff40444c));
                g.setFont(theme::font(10.0f, true));
                g.drawText("C" + juce::String(pitch / 12 - 1), r.withTrimmedLeft(r.getWidth() - 26), juce::Justification::centred);
            }
        }
    }

    // Velocity lane
    g.setColour(theme::bg1);
    g.fillRect(velocityArea);
    g.setColour(theme::line);
    g.drawHorizontalLine(velocityArea.getY(), 0.0f, static_cast<float>(getWidth()));
    g.setColour(theme::textDim);
    g.setFont(theme::font(10.0f, true));
    g.drawText("VEL", velocityArea.withWidth(keyboardWidth), juce::Justification::centred);
    {
        juce::Graphics::ScopedSaveState state(g);
        g.reduceClipRegion(velocityArea.withTrimmedLeft(keyboardWidth));
        for (size_t i = 0; i < c->notes.size(); ++i)
        {
            const auto& n = c->notes[i];
            const float x = beatToX(n.start);
            const float h = (velocityArea.getHeight() - 8) * n.velocity;
            g.setColour(selection.count(i) != 0 ? theme::text : c->colour);
            g.fillRect(x, static_cast<float>(velocityArea.getBottom()) - 4.0f - h, 3.0f, h);
            g.fillEllipse(x - 2.0f, static_cast<float>(velocityArea.getBottom()) - 6.0f - h, 7.0f, 7.0f);
        }
    }
}

void PianoRoll::resized()
{
    auto area = getLocalBounds();
    toolbar = area.removeFromTop(30);
    {
        auto t = toolbar.reduced(4, 4);
        t.removeFromLeft(186);
        gridBox.setBounds(t.removeFromLeft(110));
        t.removeFromLeft(6);
        quantizeButton.setBounds(t.removeFromLeft(76));
        t.removeFromLeft(6);
        lengthBars.setBounds(t.removeFromLeft(90));
    }
    hbar.setBounds(area.removeFromBottom(10).withTrimmedLeft(keyboardWidth).withTrimmedRight(10));
    velocityArea = area.removeFromBottom(velocityHeight);
    vbar.setBounds(area.removeFromRight(10));
    velocityArea.setWidth(area.getWidth());
    keyboardArea = area.removeFromLeft(keyboardWidth);
    gridArea = area;
    if (needsFit)
        fitToClip();
    updateScrollBars();
}

void PianoRoll::timerTick()
{
    auto* c = clip();
    if (c == nullptr)
        return;
    const float x = beatToX(session.engine.getPlayhead() - c->start);
    if (std::abs(x - lastPlayheadX) >= 0.5f)
    {
        for (float px : { lastPlayheadX, x })
            repaint(juce::roundToInt(px) - 2, gridArea.getY(), 5, gridArea.getHeight());
        lastPlayheadX = x;
    }
}

//==============================================================================
void PianoRoll::mouseMove(const juce::MouseEvent& e)
{
    const int hit = gridArea.contains(e.getPosition()) ? noteAt(e.position) : -1;
    if (hit >= 0 && clip() != nullptr && noteBounds(clip()->notes[static_cast<size_t>(hit)]).getRight() - e.position.x < 6.0f)
        setMouseCursor(juce::MouseCursor::LeftRightResizeCursor);
    else
        setMouseCursor(hit >= 0 ? juce::MouseCursor::DraggingHandCursor : juce::MouseCursor::NormalCursor);
}

void PianoRoll::mouseDown(const juce::MouseEvent& e)
{
    auto* c = clip();
    mode = Mode::none;
    started = false;
    if (c == nullptr)
        return;
    session.selectTrack(session.project.trackOwningClip(clipId)->id);

    if (keyboardArea.contains(e.getPosition()))
    {
        mode = Mode::keyboard;
        audition(yToPitch(e.position.y));
        repaint(keyboardArea);
        return;
    }

    if (velocityArea.contains(e.getPosition()))
    {
        // Pick the note whose stem is closest to the click.
        int best = -1;
        float bestDistance = 8.0f;
        for (size_t i = 0; i < c->notes.size(); ++i)
        {
            const float d = std::abs(beatToX(c->notes[i].start) - e.position.x);
            if (d < bestDistance)
            {
                bestDistance = d;
                best = static_cast<int>(i);
            }
        }
        if (best < 0)
            return;
        if (selection.count(static_cast<size_t>(best)) == 0)
            selection = { static_cast<size_t>(best) };
        mode = Mode::velocity;
        dragIndices.assign(selection.begin(), selection.end());
        originals.clear();
        for (auto i : dragIndices)
            originals.push_back(c->notes[i]);
        session.project.checkpoint();
        mouseDrag(e);
        return;
    }

    if (! gridArea.contains(e.getPosition()))
        return;

    const int hit = noteAt(e.position);
    if (e.mods.isPopupMenu())
    {
        if (hit >= 0)
        {
            session.edit([c, hit] { c->notes.erase(c->notes.begin() + hit); });
            selection.clear();
        }
        return;
    }

    downBeat = xToBeat(e.position.x);
    downPitch = yToPitch(e.position.y);

    if (hit < 0)
    {
        if (e.mods.isShiftDown() || e.mods.isCommandDown())
        {
            mode = Mode::select;
            rubberBand = { e.position, e.position };
            return;
        }
        // Draw a new note; dragging right sets its length.
        selection.clear();
        const double g = grid();
        Note note;
        note.pitch = juce::jlimit(0, 127, downPitch);
        note.start = std::max(0.0, g > 0 ? std::floor(downBeat / g) * g : downBeat);
        note.length = lastLength;
        note.velocity = 0.8f;
        session.edit([c, note] { c->notes.push_back(note); });
        selection = { c->notes.size() - 1 };
        mode = Mode::resize;
        started = true;
        audition(note.pitch);
    }
    else
    {
        const auto index = static_cast<size_t>(hit);
        if (e.mods.isShiftDown())
        {
            if (selection.count(index) != 0)
                selection.erase(index);
            else
                selection.insert(index);
        }
        else if (selection.count(index) == 0)
            selection = { index };
        const auto& n = c->notes[index];
        mode = noteBounds(n).getRight() - e.position.x < 6.0f ? Mode::resize : Mode::move;
        audition(n.pitch);
        if (e.mods.isAltDown() && mode == Mode::move)
        {
            // Alt-drag duplicates the selection and drags the copies.
            session.project.checkpoint();
            started = true;
            std::vector<Note> copies;
            for (auto i : selection)
                copies.push_back(c->notes[i]);
            std::set<size_t> fresh;
            {
                const juce::ScopedLock sl(session.project.lock);
                for (auto& n2 : copies)
                {
                    c->notes.push_back(n2);
                    fresh.insert(c->notes.size() - 1);
                }
            }
            selection = fresh;
        }
    }
    dragIndices.assign(selection.begin(), selection.end());
    originals.clear();
    for (auto i : dragIndices)
        originals.push_back(c->notes[i]);
    repaint();
}

void PianoRoll::mouseDrag(const juce::MouseEvent& e)
{
    auto* c = clip();
    if (c == nullptr || mode == Mode::none)
        return;
    if (mode == Mode::keyboard)
    {
        audition(yToPitch(e.position.y));
        repaint(keyboardArea);
        return;
    }
    if (mode == Mode::select)
    {
        rubberBand = juce::Rectangle<float>(e.mouseDownPosition, e.position);
        selection.clear();
        for (size_t i = 0; i < c->notes.size(); ++i)
            if (noteBounds(c->notes[i]).intersects(rubberBand))
                selection.insert(i);
        repaint();
        return;
    }
    if (mode == Mode::velocity)
    {
        const float v = juce::jlimit(0.02f, 1.0f, (velocityArea.getBottom() - 4.0f - e.position.y) / (velocityArea.getHeight() - 8.0f));
        const juce::ScopedLock sl(session.project.lock);
        for (auto i : dragIndices)
            if (i < c->notes.size())
                c->notes[i].velocity = v;
        session.project.changed();
        repaint();
        return;
    }
    if (! started && e.getDistanceFromDragStart() < 3)
        return;
    if (! started)
    {
        session.project.checkpoint();
        started = true;
    }

    const double g = grid();
    const double rawDelta = xToBeat(e.position.x) - downBeat;
    const double delta = g > 0 ? std::round(rawDelta / g) * g : rawDelta;
    const int pitchDelta = yToPitch(e.position.y) - downPitch;
    {
        const juce::ScopedLock sl(session.project.lock);
        for (size_t k = 0; k < dragIndices.size(); ++k)
        {
            const auto i = dragIndices[k];
            if (i >= c->notes.size())
                continue;
            auto& n = c->notes[i];
            const auto& o = originals[k];
            if (mode == Mode::move)
            {
                n.start = std::max(0.0, o.start + delta);
                n.pitch = juce::jlimit(0, 127, o.pitch + pitchDelta);
            }
            else
            {
                const double minLength = g > 0 ? g : 1.0 / 32.0;
                n.length = std::max(minLength, o.length + delta);
                lastLength = n.length;
            }
        }
    }
    if (mode == Mode::move && ! dragIndices.empty() && dragIndices[0] < c->notes.size())
        audition(c->notes[dragIndices[0]].pitch);
    session.project.changed();
    repaint();
}

void PianoRoll::mouseUp(const juce::MouseEvent&)
{
    stopAudition();
    if (mode == Mode::move && ! dragIndices.empty())
        if (auto* c = clip(); c != nullptr && dragIndices[0] < c->notes.size())
            lastLength = c->notes[dragIndices[0]].length;
    mode = Mode::none;
    repaint();
}

void PianoRoll::mouseDoubleClick(const juce::MouseEvent& e)
{
    auto* c = clip();
    if (c == nullptr || ! gridArea.contains(e.getPosition()))
        return;
    const int hit = noteAt(e.position);
    if (hit >= 0)
    {
        session.edit([c, hit] { c->notes.erase(c->notes.begin() + hit); });
        selection.clear();
        repaint();
    }
}

void PianoRoll::mouseWheelMove(const juce::MouseEvent& e, const juce::MouseWheelDetails& w)
{
    if (e.mods.isCommandDown() || e.mods.isCtrlDown())
    {
        const double anchor = xToBeat(e.position.x);
        pixelsPerBeat = juce::jlimit(8.0, 600.0, pixelsPerBeat * std::pow(1.15, (w.deltaY + w.deltaX) * 4.0));
        scrollBeats = std::max(0.0, anchor - (e.position.x - gridArea.getX()) / pixelsPerBeat);
    }
    else if (e.mods.isShiftDown() || std::abs(w.deltaX) > std::abs(w.deltaY))
        scrollBeats = std::max(0.0, scrollBeats - (std::abs(w.deltaX) > 0 ? w.deltaX : w.deltaY) * 300.0 / pixelsPerBeat);
    else
        scrollY = juce::jlimit(0, std::max(0, 128 * keyHeight - gridArea.getHeight()), scrollY - juce::roundToInt(w.deltaY * 250.0));
    updateScrollBars();
    repaint();
}

//==============================================================================
void PianoRoll::deleteSelected()
{
    auto* c = clip();
    if (c == nullptr || selection.empty())
        return;
    session.edit([&] {
        for (auto it = selection.rbegin(); it != selection.rend(); ++it)
            if (*it < c->notes.size())
                c->notes.erase(c->notes.begin() + static_cast<std::ptrdiff_t>(*it));
    });
    selection.clear();
    repaint();
}

void PianoRoll::selectAll()
{
    if (auto* c = clip())
    {
        selection.clear();
        for (size_t i = 0; i < c->notes.size(); ++i)
            selection.insert(i);
        repaint();
    }
}

void PianoRoll::transposeSelected(int semitones)
{
    auto* c = clip();
    if (c == nullptr || selection.empty())
        return;
    session.edit([&] {
        for (auto i : selection)
            if (i < c->notes.size())
                c->notes[i].pitch = juce::jlimit(0, 127, c->notes[i].pitch + semitones);
    });
    repaint();
}

void PianoRoll::quantize()
{
    auto* c = clip();
    const double g = grid();
    if (c == nullptr || g <= 0)
        return;
    session.edit([&] {
        for (size_t i = 0; i < c->notes.size(); ++i)
            if (selection.empty() || selection.count(i) != 0)
                c->notes[i].start = std::max(0.0, std::round(c->notes[i].start / g) * g);
    });
    session.status("Quantized to grid");
    repaint();
}

} // namespace az

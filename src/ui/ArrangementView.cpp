#include "ArrangementView.h"

namespace az
{

namespace
{
    bool isBrowserItem(const juce::var& description)
    {
        const auto s = description.toString();
        return s.startsWith("builtin:") || s.startsWith("preset:") || s.startsWith("plugin:") || s.startsWith("file:");
    }

    void drawMeter(juce::Graphics& g, juce::Rectangle<float> r, float left, float right)
    {
        g.setColour(theme::bg0);
        g.fillRoundedRectangle(r, 2.0f);
        auto bar = [&](juce::Rectangle<float> b, float level) {
            const float fill = juce::jlimit(0.0f, 1.0f, (juce::Decibels::gainToDecibels(level, -60.0f) + 60.0f) / 63.0f);
            g.setColour(level > 0.99f ? theme::danger : theme::accent2.withAlpha(0.9f));
            g.fillRect(b.withWidth(b.getWidth() * fill));
        };
        auto inner = r.reduced(1.0f);
        bar(inner.removeFromTop(inner.getHeight() * 0.5f - 0.5f), left);
        bar(inner.withTrimmedTop(1.0f), right);
    }
}

double ArrangementView::gridStep() const
{
    // Visible grid adapts to zoom so lines never get too dense.
    const double bar = session.project.beatsPerBar;
    for (double step : { 0.25, 0.5, 1.0 })
        if (step * pixelsPerBeat >= 14.0)
            return step;
    for (double bars = 1.0;; bars *= 2.0)
        if (bars * bar * pixelsPerBeat >= 14.0)
            return bars * bar;
}

//==============================================================================
class ArrangementView::Ruler final : public juce::Component
{
public:
    explicit Ruler(ArrangementView& v) : view(v) {}

    void paint(juce::Graphics& g) override
    {
        auto& p = view.session.project;
        g.fillAll(theme::bg1);
        const float h = static_cast<float>(getHeight());
        const double bar = p.beatsPerBar;

        // Loop brace
        const float lx = static_cast<float>(view.beatToX(p.loopStart));
        const float lw = static_cast<float>((p.loopEnd - p.loopStart) * view.pixelsPerBeat);
        g.setColour(p.loopEnabled ? theme::accent.withAlpha(0.85f) : theme::textDim.withAlpha(0.35f));
        g.fillRoundedRectangle(lx, 2.0f, lw, 9.0f, 2.0f);

        // Labels every N bars depending on zoom
        int labelEvery = 1;
        while (labelEvery * bar * view.pixelsPerBeat < 36.0)
            labelEvery *= 2;
        const double step = view.gridStep();
        const double first = std::floor(view.xToBeat(0) / step) * step;
        g.setFont(theme::font(11.0f));
        for (double b = first; view.beatToX(b) < getWidth(); b += step)
        {
            const float x = static_cast<float>(view.beatToX(b));
            const bool isBar = std::fmod(b, bar) < 1e-6;
            const int barNumber = static_cast<int>(std::round(b / bar));
            if (isBar && barNumber % labelEvery == 0)
            {
                g.setColour(theme::textDim);
                g.drawVerticalLine(juce::roundToInt(x), 13.0f, h);
                g.setColour(theme::text);
                g.drawText(juce::String(barNumber + 1), juce::roundToInt(x) + 4, 13, 40, static_cast<int>(h) - 14, juce::Justification::centredLeft);
            }
            else
            {
                g.setColour(theme::line);
                g.drawVerticalLine(juce::roundToInt(x), h - 6.0f, h);
            }
        }

        const float px = static_cast<float>(view.beatToX(view.session.engine.getPlayhead()));
        juce::Path tri;
        tri.addTriangle(px - 6.0f, h - 9.0f, px + 6.0f, h - 9.0f, px, h);
        g.setColour(theme::text);
        g.fillPath(tri);
        g.setColour(theme::line);
        g.drawHorizontalLine(getHeight() - 1, 0.0f, static_cast<float>(getWidth()));
    }

    void mouseDown(const juce::MouseEvent& e) override
    {
        auto& p = view.session.project;
        const double beat = view.xToBeat(e.x);
        mode = Mode::playhead;
        if (e.y < 13)
        {
            const double lx = view.beatToX(p.loopStart), rx = view.beatToX(p.loopEnd);
            p.checkpoint();
            originalStart = p.loopStart;
            originalEnd = p.loopEnd;
            downBeat = beat;
            if (std::abs(e.x - lx) < 6)
                mode = Mode::loopStart;
            else if (std::abs(e.x - rx) < 6)
                mode = Mode::loopEnd;
            else if (e.x > lx && e.x < rx)
                mode = Mode::loopMove;
            else
            {
                mode = Mode::loopEnd;
                const double start = std::floor(beat / p.beatsPerBar) * p.beatsPerBar;
                setLoop(start, start + p.beatsPerBar);
                originalStart = start;
            }
            return;
        }
        view.session.engine.setPlayhead(std::max(0.0, view.session.snap(beat, view.session.snapBeats > 0 ? view.session.snapBeats : 0.0)));
        repaint();
    }

    void mouseDrag(const juce::MouseEvent& e) override
    {
        auto& s = view.session;
        const double grid = s.snapBeats > 0 ? s.snapBeats : 0.25;
        const double beat = std::max(0.0, view.xToBeat(e.x));
        switch (mode)
        {
            case Mode::playhead: s.engine.setPlayhead(s.snap(beat, s.snapBeats)); break;
            case Mode::loopStart: setLoop(std::min(s.snap(beat, grid), originalEnd - grid), originalEnd); break;
            case Mode::loopEnd: setLoop(originalStart, std::max(s.snap(beat, grid), originalStart + grid)); break;
            case Mode::loopMove:
            {
                const double delta = s.snap(beat - downBeat, grid);
                const double start = std::max(0.0, originalStart + delta);
                setLoop(start, start + (originalEnd - originalStart));
                break;
            }
        }
        repaint();
    }

    void mouseDoubleClick(const juce::MouseEvent& e) override
    {
        if (e.y < 13)
            view.session.edit([this] { view.session.project.loopEnabled = ! view.session.project.loopEnabled; });
    }

    void mouseMove(const juce::MouseEvent& e) override
    {
        auto& p = view.session.project;
        const bool edge = e.y < 13 && (std::abs(e.x - view.beatToX(p.loopStart)) < 6 || std::abs(e.x - view.beatToX(p.loopEnd)) < 6);
        setMouseCursor(edge ? juce::MouseCursor::LeftRightResizeCursor : juce::MouseCursor::NormalCursor);
    }

    void mouseWheelMove(const juce::MouseEvent& e, const juce::MouseWheelDetails& w) override
    {
        view.zoom(std::pow(1.15, (w.deltaY + w.deltaX) * 4.0), e.x);
    }

private:
    void setLoop(double start, double end)
    {
        view.session.edit([&] {
            auto& p = view.session.project;
            p.loopStart = start;
            p.loopEnd = end;
            p.loopEnabled = true;
        }, false);
    }

    enum class Mode { playhead, loopStart, loopEnd, loopMove };
    ArrangementView& view;
    Mode mode = Mode::playhead;
    double originalStart = 0, originalEnd = 0, downBeat = 0;
};

//==============================================================================
class ArrangementView::Lanes final : public juce::Component,
                                     public juce::DragAndDropTarget,
                                     public juce::FileDragAndDropTarget
{
public:
    explicit Lanes(ArrangementView& v) : view(v) {}

    void paint(juce::Graphics& g) override
    {
        auto& p = view.session.project;
        const int w = getWidth(), h = getHeight();
        g.fillAll(theme::bg0);
        const int count = static_cast<int>(p.tracks.size());

        for (int i = 0; i < count; ++i)
        {
            const int y = view.laneTop(i);
            if (y > h || y + trackHeight < 0)
                continue;
            if (p.tracks[static_cast<size_t>(i)]->id == view.session.selectedTrackId)
            {
                g.setColour(juce::Colours::white.withAlpha(0.025f));
                g.fillRect(0, y, w, trackHeight);
            }
            g.setColour(theme::grid.darker(0.2f));
            g.drawHorizontalLine(y + trackHeight - 1, 0.0f, static_cast<float>(w));
        }

        // Grid
        const double step = view.gridStep();
        const double bar = p.beatsPerBar;
        for (double b = std::floor(view.xToBeat(0) / step) * step; view.beatToX(b) < w; b += step)
        {
            const bool isBar = std::fmod(b, bar) < 1e-6;
            g.setColour(isBar ? theme::line.withAlpha(0.7f) : theme::grid);
            g.drawVerticalLine(juce::roundToInt(view.beatToX(b)), 0.0f, static_cast<float>(std::min(h, view.laneTop(count))));
        }

        if (p.loopEnabled)
        {
            const float lx = static_cast<float>(view.beatToX(p.loopStart));
            const float lw = static_cast<float>((p.loopEnd - p.loopStart) * view.pixelsPerBeat);
            g.setColour(theme::accent.withAlpha(0.045f));
            g.fillRect(lx, 0.0f, lw, static_cast<float>(h));
        }

        for (int i = 0; i < count; ++i)
        {
            const int y = view.laneTop(i);
            if (y > h || y + trackHeight < 0)
                continue;
            for (auto& clip : p.tracks[static_cast<size_t>(i)]->data.clips)
                drawClip(g, clip, clipBounds(clip, i));
        }

        // Empty area below the tracks
        const int bottom = view.laneTop(count);
        if (bottom < h)
        {
            auto hint = juce::Rectangle<int>(0, bottom, w, h - bottom).reduced(16, 12).withHeight(std::min(90, h - bottom - 24));
            g.setColour(dropTrack == count ? theme::accent.withAlpha(0.12f) : theme::bg1.withAlpha(0.5f));
            g.fillRoundedRectangle(hint.toFloat(), 8.0f);
            g.setColour(dropTrack == count ? theme::accent : theme::line);
            g.drawRoundedRectangle(hint.toFloat(), 8.0f, 1.0f);
            g.setColour(theme::textDim);
            g.setFont(theme::font(13.0f));
            g.drawFittedText("Drop instruments, samples, MIDI files or VST3 plugins here to create a track\n"
                             "Double-click an empty lane to create a MIDI clip",
                             hint, juce::Justification::centred, 2);
        }

        if (dropTrack >= 0 && dropTrack < count)
        {
            const int y = view.laneTop(dropTrack);
            g.setColour(theme::accent.withAlpha(0.15f));
            g.fillRect(0, y, w, trackHeight);
            g.setColour(theme::accent);
            g.drawVerticalLine(juce::roundToInt(view.beatToX(dropBeat)), static_cast<float>(y), static_cast<float>(y + trackHeight));
        }

        const float px = static_cast<float>(view.beatToX(view.session.engine.getPlayhead()));
        g.setColour(theme::text.withAlpha(0.9f));
        g.drawVerticalLine(juce::roundToInt(px), 0.0f, static_cast<float>(h));
    }

    juce::Rectangle<int> clipBounds(const Clip& clip, int trackIndex) const
    {
        const int x = juce::roundToInt(view.beatToX(clip.start));
        const int r = juce::roundToInt(view.beatToX(clip.end()));
        return { x, view.laneTop(trackIndex) + 2, std::max(4, r - x), trackHeight - 5 };
    }

    void drawClip(juce::Graphics& g, const Clip& clip, juce::Rectangle<int> bounds)
    {
        if (bounds.getRight() < 0 || bounds.getX() > getWidth())
            return;
        const bool selected = clip.id == view.session.selectedClipId;
        const auto colour = clip.muted ? theme::textDim : clip.colour;
        auto r = bounds.toFloat();
        g.setColour(colour.withMultipliedBrightness(0.42f).withAlpha(0.95f));
        g.fillRoundedRectangle(r, 4.0f);
        auto header = r.removeFromTop(15.0f);
        g.setColour(colour);
        g.fillRoundedRectangle(header.withHeight(18.0f), 4.0f);
        g.setColour(colour.withMultipliedBrightness(0.42f));
        g.fillRect(header.withTop(header.getBottom()).withHeight(3.0f));
        g.setColour(juce::Colours::black.withAlpha(0.85f));
        g.setFont(theme::font(11.0f, true));
        g.drawText(clip.name, header.reduced(5.0f, 0.0f), juce::Justification::centredLeft, true);

        auto body = r.reduced(3.0f, 4.0f);
        g.saveState();
        g.reduceClipRegion(body.toNearestInt());
        if (clip.kind == ClipKind::midi && ! clip.notes.empty())
        {
            int lo = 127, hi = 0;
            for (auto& n : clip.notes)
            {
                lo = std::min(lo, n.pitch);
                hi = std::max(hi, n.pitch);
            }
            const float range = static_cast<float>(std::max(8, hi - lo + 1));
            const float noteH = std::max(1.5f, body.getHeight() / range);
            g.setColour(colour.brighter(0.4f));
            for (auto& n : clip.notes)
            {
                if (n.start >= clip.length || n.start < 0)
                    continue;
                const float x = body.getX() + static_cast<float>(n.start * view.pixelsPerBeat) - 3.0f;
                const float nw = std::max(1.5f, static_cast<float>(std::min(n.length, clip.length - n.start) * view.pixelsPerBeat) - 1.0f);
                const float y = body.getBottom() - (n.pitch - lo + 1 + (range - (hi - lo + 1)) * 0.5f) * noteH;
                g.fillRect(x, y, nw, std::max(1.5f, noteH - 0.5f));
            }
        }
        else if (clip.kind == ClipKind::audio && clip.sample != nullptr)
        {
            const auto& s = *clip.sample;
            const double secondsPerPixel = 60.0 / view.session.project.bpm / view.pixelsPerBeat;
            const float mid = body.getCentreY(), half = body.getHeight() * 0.5f;
            g.setColour(colour.brighter(0.3f));
            const int x0 = std::max(static_cast<int>(body.getX()), 0);
            const int x1 = std::min(static_cast<int>(body.getRight()), getWidth());
            for (int x = x0; x < x1; ++x)
            {
                const double t0 = clip.sampleOffset + (x - bounds.getX()) * secondsPerPixel;
                const int i0 = static_cast<int>(t0 * s.sampleRate) / AudioSample::peakBlock;
                const int i1 = std::max(i0 + 1, static_cast<int>((t0 + secondsPerPixel) * s.sampleRate) / AudioSample::peakBlock);
                float peak = 0.0f;
                for (int i = std::max(0, i0); i < std::min(i1, static_cast<int>(s.peaks.size())); ++i)
                    peak = std::max(peak, s.peaks[static_cast<size_t>(i)]);
                const float ph = std::min(1.0f, peak * clip.gain) * half;
                g.drawVerticalLine(x, mid - ph, mid + ph + 1.0f);
            }
        }
        g.restoreState();

        if (selected)
        {
            g.setColour(theme::text);
            g.drawRoundedRectangle(bounds.toFloat().reduced(0.5f), 4.0f, 1.5f);
        }
    }

    struct Hit { std::shared_ptr<Track> track; Clip* clip = nullptr; int trackIndex = -1; juce::Rectangle<int> bounds; };

    Hit hitTestClip(juce::Point<int> pos)
    {
        Hit hit;
        auto& p = view.session.project;
        hit.trackIndex = view.trackIndexAt(pos.y);
        if (pos.y + view.scrollY < 0 || hit.trackIndex < 0 || hit.trackIndex >= static_cast<int>(p.tracks.size()))
        {
            hit.trackIndex = -1;
            return hit;
        }
        hit.track = p.tracks[static_cast<size_t>(hit.trackIndex)];
        auto& clips = hit.track->data.clips;
        for (auto it = clips.rbegin(); it != clips.rend(); ++it)
        {
            auto b = clipBounds(*it, hit.trackIndex);
            if (b.contains(pos))
            {
                hit.clip = &*it;
                hit.bounds = b;
                break;
            }
        }
        return hit;
    }

    void mouseMove(const juce::MouseEvent& e) override
    {
        auto hit = hitTestClip(e.getPosition());
        auto cursor = juce::MouseCursor::NormalCursor;
        if (hit.clip != nullptr)
        {
            const bool wide = hit.bounds.getWidth() > 18;
            if (wide && (e.x - hit.bounds.getX() < 6 || hit.bounds.getRight() - e.x < 6))
                cursor = juce::MouseCursor::LeftRightResizeCursor;
            else
                cursor = juce::MouseCursor::DraggingHandCursor;
        }
        setMouseCursor(cursor);
    }

    void mouseDown(const juce::MouseEvent& e) override
    {
        auto& s = view.session;
        drag = {};
        auto hit = hitTestClip(e.getPosition());
        if (hit.clip == nullptr)
        {
            if (hit.track != nullptr)
                s.selectTrack(hit.track->id);
            s.selectedClipId = 0;
            if (e.mods.isPopupMenu())
            {
                showLaneMenu(hit.track, view.xToBeat(e.x));
                return;
            }
            s.engine.setPlayhead(std::max(0.0, s.snap(view.xToBeat(e.x))));
            s.sendChangeMessage();
            repaint();
            return;
        }
        s.selectClip(hit.clip->id);
        if (e.mods.isPopupMenu())
        {
            showClipMenu(hit.clip->id);
            return;
        }
        drag.clipId = hit.clip->id;
        drag.original = *hit.clip;
        drag.trackId = hit.track->id;
        drag.downBeat = view.xToBeat(e.x);
        drag.copy = e.mods.isAltDown();
        const bool wide = hit.bounds.getWidth() > 18;
        drag.mode = wide && e.x - hit.bounds.getX() < 6 ? Drag::Mode::left
                  : wide && hit.bounds.getRight() - e.x < 6 ? Drag::Mode::right
                                                            : Drag::Mode::move;
        repaint();
    }

    void mouseDrag(const juce::MouseEvent& e) override
    {
        if (drag.mode == Drag::Mode::none || e.getDistanceFromDragStart() < 3)
            return;
        auto& s = view.session;
        auto& p = s.project;
        if (! drag.started)
        {
            drag.started = true;
            p.checkpoint();
            if (drag.copy)
                if (auto track = p.findTrack(drag.trackId))
                {
                    Clip copy = drag.original;
                    copy.id = p.newId();
                    const juce::ScopedLock sl(p.lock);
                    track->data.clips.push_back(copy);
                }
        }

        const double delta = view.xToBeat(e.x) - drag.downBeat;
        const double minLength = s.snapBeats > 0 ? std::min(s.snapBeats, 0.25) : 0.0625;
        auto track = p.trackOwningClip(drag.clipId);
        if (track == nullptr)
            return;

        const juce::ScopedLock sl(p.lock);
        auto* clip = track->findClip(drag.clipId);
        const auto& o = drag.original;
        switch (drag.mode)
        {
            case Drag::Mode::move:
            {
                clip->start = std::max(0.0, s.snap(o.start + delta));
                const int target = juce::jlimit(0, static_cast<int>(p.tracks.size()) - 1, view.trackIndexAt(e.y));
                auto targetTrack = p.tracks[static_cast<size_t>(target)];
                if (targetTrack != track)
                {
                    Clip moved = *clip;
                    auto& clips = track->data.clips;
                    clips.erase(std::remove_if(clips.begin(), clips.end(), [&](const Clip& c) { return c.id == moved.id; }), clips.end());
                    targetTrack->data.clips.push_back(moved);
                    s.selectedTrackId = targetTrack->id;
                    s.engine.setLiveTrack(targetTrack->id);
                }
                break;
            }
            case Drag::Mode::right:
            {
                double length = std::max(minLength, s.snap(o.end() + delta) - o.start);
                if (o.kind == ClipKind::audio && o.sample != nullptr)
                    length = std::min(length, (o.sample->lengthSeconds() - o.sampleOffset) * p.bpm / 60.0);
                clip->length = std::max(minLength, length);
                break;
            }
            case Drag::Mode::left:
            {
                double start = juce::jlimit(0.0, o.end() - minLength, s.snap(o.start + delta));
                if (o.kind == ClipKind::audio)
                    start = std::max(start, o.start - o.sampleOffset * p.bpm / 60.0);
                const double d = start - o.start;
                clip->start = start;
                clip->length = o.length - d;
                if (o.kind == ClipKind::audio)
                    clip->sampleOffset = o.sampleOffset + d * 60.0 / p.bpm;
                else
                {
                    clip->notes = o.notes;
                    for (auto& n : clip->notes)
                        n.start -= d;
                }
                break;
            }
            case Drag::Mode::none: break;
        }
        p.changed();
        repaint();
    }

    void mouseUp(const juce::MouseEvent&) override
    {
        if (drag.started)
            view.session.project.changed();
        drag = {};
    }

    void mouseDoubleClick(const juce::MouseEvent& e) override
    {
        auto& s = view.session;
        auto hit = hitTestClip(e.getPosition());
        if (hit.clip != nullptr)
        {
            s.editClip(hit.clip->id);
            return;
        }
        if (hit.track == nullptr)
        {
            s.addTrack(true);
            return;
        }
        const double bar = s.project.beatsPerBar;
        const double grid = s.snapBeats > 0 ? s.snapBeats : 1.0;
        const double start = std::max(0.0, std::floor(view.xToBeat(e.x) / grid) * grid);
        const int id = s.createMidiClip(hit.track->id, start, bar);
        s.editClip(id);
    }

    void mouseWheelMove(const juce::MouseEvent& e, const juce::MouseWheelDetails& w) override
    {
        if (e.mods.isCommandDown() || e.mods.isCtrlDown())
            view.zoom(std::pow(1.15, (w.deltaY + w.deltaX) * 4.0), e.x);
        else if (e.mods.isShiftDown() || std::abs(w.deltaX) > std::abs(w.deltaY))
            view.scrollBy(-(std::abs(w.deltaX) > 0 ? w.deltaX : w.deltaY) * 400.0 / view.pixelsPerBeat, 0);
        else
            view.scrollBy(0.0, juce::roundToInt(-w.deltaY * 300.0));
    }

    void mouseMagnify(const juce::MouseEvent& e, float scale) override { view.zoom(scale, e.x); }

    // Drag and drop from the browser
    bool isInterestedInDragSource(const SourceDetails& d) override { return isBrowserItem(d.description); }
    void itemDragMove(const SourceDetails& d) override { setDropTarget(d.localPosition.toInt()); }
    void itemDragExit(const SourceDetails&) override { clearDropTarget(); }
    void itemDropped(const SourceDetails& d) override
    {
        auto [trackId, beat] = dropLocation(d.localPosition.toInt());
        clearDropTarget();
        view.session.applyItem(d.description.toString(), trackId, beat);
    }

    // Drag and drop from Finder / Explorer
    bool isInterestedInFileDrag(const juce::StringArray& files) override
    {
        for (auto& f : files)
            if (Session::isInterestingFile(f))
                return true;
        return false;
    }
    void fileDragMove(const juce::StringArray&, int x, int y) override { setDropTarget({ x, y }); }
    void fileDragExit(const juce::StringArray&) override { clearDropTarget(); }
    void filesDropped(const juce::StringArray& files, int x, int y) override
    {
        auto [trackId, beat] = dropLocation({ x, y });
        clearDropTarget();
        view.session.importFiles(files, trackId, beat);
    }

    int dropTrack = -1;

private:
    std::pair<int, double> dropLocation(juce::Point<int> pos)
    {
        auto& p = view.session.project;
        const int index = view.trackIndexAt(pos.y);
        const int trackId = index >= 0 && index < static_cast<int>(p.tracks.size()) ? p.tracks[static_cast<size_t>(index)]->id : -1;
        return { trackId, std::max(0.0, view.session.snap(view.xToBeat(pos.x))) };
    }

    void setDropTarget(juce::Point<int> pos)
    {
        dropTrack = std::min(view.trackIndexAt(pos.y), static_cast<int>(view.session.project.tracks.size()));
        dropBeat = std::max(0.0, view.session.snap(view.xToBeat(pos.x)));
        repaint();
    }

    void clearDropTarget()
    {
        dropTrack = -1;
        repaint();
    }

    void showClipMenu(int clipId)
    {
        auto& s = view.session;
        auto* clip = s.findClip(clipId);
        if (clip == nullptr)
            return;
        juce::PopupMenu menu, colours;
        if (clip->kind == ClipKind::midi)
            menu.addItem(1, "Edit in Piano Roll");
        menu.addItem(2, "Rename...");
        menu.addItem(3, "Duplicate", true, false);
        menu.addItem(4, "Split at Playhead");
        menu.addItem(5, clip->muted ? "Unmute" : "Mute");
        for (int i = 0; i < 10; ++i)
            colours.addColouredItem(100 + i, "Colour " + juce::String(i + 1), palette::trackColour(i));
        menu.addSubMenu("Colour", colours);
        menu.addSeparator();
        menu.addItem(6, "Copy");
        menu.addItem(7, "Delete");
        menu.showMenuAsync(juce::PopupMenu::Options(), [this, clipId](int result) {
            auto& session = view.session;
            auto* c = session.findClip(clipId);
            if (c == nullptr)
                return;
            switch (result)
            {
                case 1: session.editClip(clipId); break;
                case 2: rename(clipId); break;
                case 3: session.duplicateClip(clipId); break;
                case 4: session.splitClip(clipId, session.engine.getPlayhead()); break;
                case 5: session.edit([c] { c->muted = ! c->muted; }); break;
                case 6: session.copyClip(clipId); break;
                case 7: session.deleteClip(clipId); break;
                default:
                    if (result >= 100)
                        session.edit([c, result] { c->colour = palette::trackColour(result - 100); });
            }
        });
    }

    void rename(int clipId)
    {
        auto* clip = view.session.findClip(clipId);
        auto* window = new juce::AlertWindow("Rename clip", {}, juce::MessageBoxIconType::NoIcon);
        window->addTextEditor("name", clip->name);
        window->addButton("OK", 1, juce::KeyPress(juce::KeyPress::returnKey));
        window->addButton("Cancel", 0, juce::KeyPress(juce::KeyPress::escapeKey));
        window->enterModalState(true, juce::ModalCallbackFunction::create([this, clipId, window](int result) {
            if (result == 1)
                if (auto* c = view.session.findClip(clipId))
                {
                    const auto name = window->getTextEditorContents("name");
                    view.session.edit([c, name] { c->name = name; });
                }
        }), true);
    }

    void showLaneMenu(std::shared_ptr<Track> track, double beat)
    {
        juce::PopupMenu menu;
        menu.addItem(1, "Create MIDI Clip", track != nullptr);
        menu.addItem(2, "Paste", track != nullptr && view.session.hasClipboard());
        menu.addSeparator();
        menu.addItem(3, "Add Instrument Track");
        menu.addItem(4, "Add Audio Track");
        const int trackId = track != nullptr ? track->id : 0;
        menu.showMenuAsync(juce::PopupMenu::Options(), [this, trackId, beat](int result) {
            auto& s = view.session;
            const double start = std::max(0.0, std::floor(beat / s.project.beatsPerBar) * s.project.beatsPerBar);
            if (result == 1)
                s.editClip(s.createMidiClip(trackId, start, s.project.beatsPerBar));
            else if (result == 2)
                s.pasteClip(trackId, s.snap(beat));
            else if (result == 3)
                s.addTrack(true);
            else if (result == 4)
                s.addTrack(false);
        });
    }

    struct Drag
    {
        enum class Mode { none, move, left, right } mode = Mode::none;
        int clipId = 0, trackId = 0;
        Clip original;
        double downBeat = 0;
        bool copy = false, started = false;
    };

    ArrangementView& view;
    Drag drag;
    double dropBeat = 0;
};

//==============================================================================
class ArrangementView::TrackHeader final : public juce::Component,
                                           public juce::DragAndDropTarget,
                                           public juce::FileDragAndDropTarget
{
public:
    TrackHeader(ArrangementView& v, int id) : view(v), trackId(id)
    {
        name.setFont(theme::font(13.0f, true));
        name.setEditable(false, true, false);
        name.setInterceptsMouseClicks(false, false);
        name.onTextChange = [this] {
            if (auto t = track())
            {
                const auto text = name.getText();
                view.session.edit([t, text] { t->data.name = text; });
            }
        };
        addAndMakeVisible(name);

        auto setup = [this](juce::TextButton& b, const char* text, juce::Colour on, const char* tip) {
            b.setButtonText(text);
            b.setClickingTogglesState(true);
            b.setColour(juce::TextButton::buttonOnColourId, on);
            b.setTooltip(tip);
            b.setWantsKeyboardFocus(false);
            addAndMakeVisible(b);
        };
        setup(mute, "M", theme::accent, "Mute");
        setup(solo, "S", juce::Colour(0xff5fb3ff), "Solo");
        setup(arm, "R", theme::record, "Arm for recording / MIDI input");
        mute.onClick = [this] { set([this](TrackData& d) { d.mute = mute.getToggleState(); }); };
        solo.onClick = [this] { set([this](TrackData& d) { d.solo = solo.getToggleState(); }); };
        arm.onClick = [this] { set([this](TrackData& d) { d.arm = arm.getToggleState(); }); };

        volume.setSliderStyle(juce::Slider::LinearHorizontal);
        volume.setTextBoxStyle(juce::Slider::NoTextBox, false, 0, 0);
        volume.setRange(0.0, 1.25);
        volume.setDoubleClickReturnValue(true, 0.8);
        volume.setWantsKeyboardFocus(false);
        volume.setPopupDisplayEnabled(true, false, nullptr);
        volume.textFromValueFunction = [](double v) { return juce::Decibels::toString(juce::Decibels::gainToDecibels(v)); };
        volume.onDragStart = [this] { view.session.project.checkpoint(); };
        volume.onValueChange = [this] {
            if (auto t = track())
            {
                const float v = static_cast<float>(volume.getValue());
                view.session.edit([t, v] { t->data.volume = v; }, false);
            }
        };
        addAndMakeVisible(volume);
        refresh();
    }

    std::shared_ptr<Track> track() const { return view.session.project.findTrack(trackId); }

    void refresh()
    {
        auto t = track();
        if (t == nullptr)
            return;
        name.setText(t->data.name, juce::dontSendNotification);
        mute.setToggleState(t->data.mute, juce::dontSendNotification);
        solo.setToggleState(t->data.solo, juce::dontSendNotification);
        arm.setToggleState(t->data.arm, juce::dontSendNotification);
        volume.setValue(t->data.volume, juce::dontSendNotification);
        volume.setColour(juce::Slider::trackColourId, t->data.colour);
        subtitle = t->instrument != nullptr ? t->instrument->displayName() : juce::String("Audio");
        if (! t->effects.empty())
            subtitle << "  +" << static_cast<int>(t->effects.size()) << " fx";
        repaint();
    }

    void refreshMeter()
    {
        if (auto t = track())
        {
            const float l = t->meterL.load(), r = t->meterR.load();
            if (std::abs(l - meterL) > 0.002f || std::abs(r - meterR) > 0.002f)
            {
                meterL = l;
                meterR = r;
                repaint(meterArea);
            }
        }
    }

    void paint(juce::Graphics& g) override
    {
        auto t = track();
        if (t == nullptr)
            return;
        const bool selected = view.session.selectedTrackId == trackId;
        g.fillAll(selected ? theme::bg2 : theme::bg1);
        if (dropHover)
        {
            g.setColour(theme::accent.withAlpha(0.18f));
            g.fillAll();
        }
        g.setColour(t->data.colour);
        g.fillRect(0, 0, 5, getHeight() - 1);
        g.setColour(theme::line);
        g.drawHorizontalLine(getHeight() - 1, 0.0f, static_cast<float>(getWidth()));
        g.setColour(theme::textDim);
        g.setFont(theme::font(11.0f));
        g.drawText(subtitle, 14, 25, getWidth() - 24, 14, juce::Justification::centredLeft, true);
        drawMeter(g, meterArea.toFloat(), meterL, meterR);
    }

    void resized() override
    {
        auto row = juce::Rectangle<int>(10, 5, getWidth() - 16, 20);
        for (auto* b : { &arm, &solo, &mute })
        {
            b->setBounds(row.removeFromRight(22).reduced(1));
            row.removeFromRight(2);
        }
        name.setBounds(row);
        auto bottom = juce::Rectangle<int>(10, 44, getWidth() - 18, 16);
        meterArea = bottom.removeFromRight(46).reduced(0, 3);
        bottom.removeFromRight(6);
        volume.setBounds(bottom);
    }

    void mouseDown(const juce::MouseEvent& e) override
    {
        view.session.selectTrack(trackId);
        if (e.mods.isPopupMenu())
            showMenu();
    }

    void mouseDoubleClick(const juce::MouseEvent&) override { name.showEditor(); }

    bool isInterestedInDragSource(const SourceDetails& d) override { return isBrowserItem(d.description); }
    void itemDragEnter(const SourceDetails&) override { dropHover = true; repaint(); }
    void itemDragExit(const SourceDetails&) override { dropHover = false; repaint(); }
    void itemDropped(const SourceDetails& d) override
    {
        dropHover = false;
        view.session.applyItem(d.description.toString(), trackId, view.session.engine.getPlayhead());
    }
    bool isInterestedInFileDrag(const juce::StringArray& files) override
    {
        for (auto& f : files)
            if (Session::isInterestingFile(f))
                return true;
        return false;
    }
    void fileDragEnter(const juce::StringArray&, int, int) override { dropHover = true; repaint(); }
    void fileDragExit(const juce::StringArray&) override { dropHover = false; repaint(); }
    void filesDropped(const juce::StringArray& files, int, int) override
    {
        dropHover = false;
        view.session.importFiles(files, trackId, view.session.engine.getPlayhead());
    }


private:
    template <typename Fn>
    void set(Fn&& fn)
    {
        if (auto t = track())
            view.session.edit([&] { fn(t->data); });
    }

    void showMenu()
    {
        juce::PopupMenu menu, colours;
        menu.addItem(1, "Rename");
        menu.addItem(2, "Duplicate Track");
        menu.addItem(3, "Move Up");
        menu.addItem(4, "Move Down");
        for (int i = 0; i < 10; ++i)
            colours.addColouredItem(100 + i, "Colour " + juce::String(i + 1), palette::trackColour(i));
        menu.addSubMenu("Colour", colours);
        menu.addSeparator();
        menu.addItem(5, "Delete Track");
        menu.showMenuAsync(juce::PopupMenu::Options(), [this](int result) {
            auto& s = view.session;
            const int id = trackId;
            switch (result)
            {
                case 1: name.showEditor(); break;
                case 2: s.duplicateTrack(id); break;
                case 3: s.moveTrack(id, -1); break;
                case 4: s.moveTrack(id, 1); break;
                case 5: juce::MessageManager::callAsync([&s, id] { s.deleteTrack(id); }); break;
                default:
                    if (result >= 100)
                        if (auto t = track())
                            s.edit([t, result] {
                                t->data.colour = palette::trackColour(result - 100);
                                for (auto& c : t->data.clips)
                                    c.colour = t->data.colour;
                            });
            }
        });
    }

    ArrangementView& view;
    int trackId;
    juce::Label name;
    juce::TextButton mute, solo, arm;
    juce::Slider volume;
    juce::String subtitle;
    juce::Rectangle<int> meterArea;
    float meterL = 0, meterR = 0;
    bool dropHover = false;
};

//==============================================================================
class ArrangementView::HeaderList final : public juce::Component
{
public:
    explicit HeaderList(ArrangementView& v) : view(v)
    {
        addInstrument.setButtonText("+ Instrument");
        addAudio.setButtonText("+ Audio");
        addInstrument.setTooltip("Add a track with the Operator synth (Cmd/Ctrl+T)");
        addAudio.setTooltip("Add an empty audio track (Cmd/Ctrl+Shift+T)");
        addInstrument.onClick = [this] { view.session.addTrack(true); };
        addAudio.onClick = [this] { view.session.addTrack(false); };
        for (auto* b : { &addInstrument, &addAudio })
        {
            b->setWantsKeyboardFocus(false);
            addAndMakeVisible(b);
        }
    }

    void rebuild()
    {
        headers.clear();
        for (auto& t : view.session.project.tracks)
        {
            auto h = std::make_unique<TrackHeader>(view, t->id);
            addAndMakeVisible(*h);
            headers.push_back(std::move(h));
        }
        resized();
    }

    void refresh()
    {
        for (auto& h : headers)
            h->refresh();
        resized();
    }

    void refreshMeters()
    {
        for (auto& h : headers)
            h->refreshMeter();
    }

    void paint(juce::Graphics& g) override
    {
        g.fillAll(theme::bg1);
        g.setColour(theme::line);
        g.drawVerticalLine(getWidth() - 1, 0.0f, static_cast<float>(getHeight()));
    }

    void resized() override
    {
        int y = -view.scrollY;
        for (auto& h : headers)
        {
            h->setBounds(0, y, getWidth() - 1, trackHeight);
            y += trackHeight;
        }
        auto buttons = juce::Rectangle<int>(10, y + 12, getWidth() - 20, 26);
        addInstrument.setBounds(buttons.removeFromLeft(buttons.getWidth() / 2 - 3));
        buttons.removeFromLeft(6);
        addAudio.setBounds(buttons);
    }

private:
    ArrangementView& view;
    std::vector<std::unique_ptr<TrackHeader>> headers;
    juce::TextButton addInstrument, addAudio;
};

//==============================================================================
ArrangementView::ArrangementView(Session& s) : session(s)
{
    ruler = std::make_unique<Ruler>(*this);
    lanes = std::make_unique<Lanes>(*this);
    headers = std::make_unique<HeaderList>(*this);
    addAndMakeVisible(*ruler);
    addAndMakeVisible(*lanes);
    addAndMakeVisible(*headers);
    for (auto* bar : { &hbar, &vbar })
    {
        bar->addListener(this);
        bar->setAutoHide(false);
        addAndMakeVisible(bar);
    }
    session.project.addChangeListener(this);
    session.addChangeListener(this);
    refresh();
}

ArrangementView::~ArrangementView()
{
    session.project.removeChangeListener(this);
    session.removeChangeListener(this);
}

void ArrangementView::changeListenerCallback(juce::ChangeBroadcaster*)
{
    refresh();
}

void ArrangementView::refresh()
{
    std::vector<int> ids;
    for (auto& t : session.project.tracks)
        ids.push_back(t->id);
    if (ids != shownTrackIds)
    {
        // A whole new project (no tracks in common): frame the song.
        if (std::none_of(ids.begin(), ids.end(), [this](int id) { return std::find(shownTrackIds.begin(), shownTrackIds.end(), id) != shownTrackIds.end(); }))
            needsFit = true;
        shownTrackIds = ids;
        headers->rebuild();
    }
    else
        headers->refresh();
    if (needsFit)
        zoomToFit();
    updateScrollBars();
    repaint();
}

void ArrangementView::zoomToFit()
{
    if (lanes->getWidth() <= 0)
        return;
    needsFit = false;
    const double bar = session.project.beatsPerBar;
    const double beats = std::max(16.0, std::ceil(session.project.songEndBeats() / bar) * bar + bar);
    pixelsPerBeat = juce::jlimit(1.5, 400.0, lanes->getWidth() / beats);
    scrollBeats = 0.0;
    scrollY = 0;
    headers->resized();
}

void ArrangementView::updateScrollBars()
{
    const double visibleBeats = lanes->getWidth() / pixelsPerBeat;
    const double total = std::max(session.project.songEndBeats() + 32.0, scrollBeats + visibleBeats);
    hbar.setRangeLimits(0.0, total, juce::dontSendNotification);
    hbar.setCurrentRange(scrollBeats, visibleBeats, juce::dontSendNotification);
    const int content = static_cast<int>(session.project.tracks.size()) * trackHeight + 140;
    vbar.setRangeLimits(0.0, std::max(content, lanes->getHeight()), juce::dontSendNotification);
    vbar.setCurrentRange(scrollY, lanes->getHeight(), juce::dontSendNotification);
}

void ArrangementView::scrollBarMoved(juce::ScrollBar* bar, double start)
{
    if (bar == &hbar)
        scrollBeats = std::max(0.0, start);
    else
        scrollY = std::max(0, juce::roundToInt(start));
    headers->resized();
    repaint();
}

void ArrangementView::scrollBy(double beats, int pixelsY)
{
    scrollBeats = std::max(0.0, scrollBeats + beats);
    const int content = static_cast<int>(session.project.tracks.size()) * trackHeight + 140;
    scrollY = juce::jlimit(0, std::max(0, content - lanes->getHeight()), scrollY + pixelsY);
    updateScrollBars();
    headers->resized();
    repaint();
}

void ArrangementView::zoom(double factor, int anchorX)
{
    const double anchorBeat = xToBeat(anchorX);
    pixelsPerBeat = juce::jlimit(1.5, 400.0, pixelsPerBeat * factor);
    scrollBeats = std::max(0.0, anchorBeat - anchorX / pixelsPerBeat);
    updateScrollBars();
    repaint();
}

void ArrangementView::timerTick(bool followPlayhead)
{
    headers->refreshMeters();
    const double beat = session.engine.getPlayhead();
    if (followPlayhead && session.engine.isPlaying())
    {
        const double visible = lanes->getWidth() / pixelsPerBeat;
        if (beat < scrollBeats || beat > scrollBeats + visible * 0.92)
        {
            scrollBeats = std::max(0.0, beat - visible * 0.08);
            updateScrollBars();
            repaint();
            lastPlayheadX = beatToX(beat);
            return;
        }
    }
    const double x = beatToX(beat);
    if (std::abs(x - lastPlayheadX) >= 0.5)
    {
        for (double px : { lastPlayheadX, x })
        {
            lanes->repaint(juce::roundToInt(px) - 2, 0, 5, lanes->getHeight());
            ruler->repaint(juce::roundToInt(px) - 8, 0, 17, ruler->getHeight());
        }
        lastPlayheadX = x;
    }
}

void ArrangementView::paint(juce::Graphics& g)
{
    g.fillAll(theme::bg1);
    auto corner = getLocalBounds().removeFromTop(rulerHeight).removeFromLeft(headerWidth);
    g.setColour(theme::textDim);
    g.setFont(theme::font(11.0f, true));
    g.drawText("ARRANGEMENT", corner.reduced(12, 0), juce::Justification::centredLeft);
    g.setColour(theme::line);
    g.drawHorizontalLine(rulerHeight - 1, 0.0f, static_cast<float>(headerWidth));
}

void ArrangementView::resized()
{
    auto area = getLocalBounds();
    const int bar = 10;
    hbar.setBounds(area.removeFromBottom(bar).withTrimmedLeft(headerWidth).withTrimmedRight(bar));
    vbar.setBounds(area.removeFromRight(bar).withTrimmedTop(rulerHeight));
    auto top = area.removeFromTop(rulerHeight);
    top.removeFromLeft(headerWidth);
    ruler->setBounds(top);
    headers->setBounds(area.removeFromLeft(headerWidth));
    lanes->setBounds(area);
    if (needsFit)
        zoomToFit();
    updateScrollBars();
}

} // namespace az

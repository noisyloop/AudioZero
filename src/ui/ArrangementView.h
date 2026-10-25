#pragma once
#include "Session.h"
#include "Theme.h"

namespace az
{

// Ableton/Bitwig-style linear arrangement: track headers on the left, a bar ruler with loop
// brace on top, and clip lanes that accept drag & drop from the browser and the file system.
class ArrangementView final : public juce::Component, private juce::ChangeListener, private juce::ScrollBar::Listener
{
public:
    explicit ArrangementView(Session&);
    ~ArrangementView() override;

    void paint(juce::Graphics&) override;
    void resized() override;
    void timerTick(bool followPlayhead); // called by the main timer
    void zoom(double factor, int anchorX);
    void scrollBy(double beats, int pixelsY);
    void zoomToFit();

    static constexpr int headerWidth = 220;
    static constexpr int rulerHeight = 30;
    static constexpr int trackHeight = 68;

    double beatToX(double beat) const { return (beat - scrollBeats) * pixelsPerBeat; }
    double xToBeat(double x) const { return x / pixelsPerBeat + scrollBeats; }
    int laneTop(int index) const { return index * trackHeight - scrollY; }
    int trackIndexAt(int y) const { return (y + scrollY) / trackHeight; }
    double gridStep() const;

    Session& session;
    double pixelsPerBeat = 22.0;
    double scrollBeats = 0.0;
    int scrollY = 0;

private:
    class Ruler;
    class Lanes;
    class TrackHeader;
    class HeaderList;

    void changeListenerCallback(juce::ChangeBroadcaster*) override;
    void scrollBarMoved(juce::ScrollBar*, double newRangeStart) override;
    void refresh();
    void updateScrollBars();

    std::unique_ptr<Ruler> ruler;
    std::unique_ptr<Lanes> lanes;
    std::unique_ptr<HeaderList> headers;
    juce::ScrollBar hbar { false }, vbar { true };
    double lastPlayheadX = -1.0;
    std::vector<int> shownTrackIds;
    bool needsFit = true;
};

} // namespace az

#pragma once
#include "Session.h"
#include "Theme.h"
#include "TransportBar.h"

namespace az
{

// Note editor for the selected MIDI clip: keyboard, note grid and velocity lane.
class PianoRoll final : public juce::Component, private juce::ChangeListener, private juce::ScrollBar::Listener
{
public:
    explicit PianoRoll(Session&);
    ~PianoRoll() override;

    void paint(juce::Graphics&) override;
    void resized() override;
    void mouseDown(const juce::MouseEvent&) override;
    void mouseDrag(const juce::MouseEvent&) override;
    void mouseUp(const juce::MouseEvent&) override;
    void mouseMove(const juce::MouseEvent&) override;
    void mouseDoubleClick(const juce::MouseEvent&) override;
    void mouseWheelMove(const juce::MouseEvent&, const juce::MouseWheelDetails&) override;

    bool hasSelection() const { return ! selection.empty(); }
    void deleteSelected();
    void selectAll();
    void transposeSelected(int semitones);
    void quantize();
    void timerTick();

private:
    void changeListenerCallback(juce::ChangeBroadcaster*) override;
    void scrollBarMoved(juce::ScrollBar*, double) override;
    Clip* clip() const;
    void fitToClip();
    void updateScrollBars();
    double grid() const;
    double xToBeat(float x) const { return (x - gridArea.getX()) / pixelsPerBeat + scrollBeats; }
    float beatToX(double beat) const { return static_cast<float>(gridArea.getX() + (beat - scrollBeats) * pixelsPerBeat); }
    int yToPitch(float y) const { return 127 - static_cast<int>((y - gridArea.getY() + scrollY) / keyHeight); }
    float pitchToY(int pitch) const { return static_cast<float>(gridArea.getY() + (127 - pitch) * keyHeight - scrollY); }
    juce::Rectangle<float> noteBounds(const Note& n) const;
    int noteAt(juce::Point<float> p) const;
    void audition(int pitch);
    void stopAudition();

    Session& session;
    int clipId = 0;
    static constexpr int keyHeight = 13;
    static constexpr int keyboardWidth = 56;
    static constexpr int velocityHeight = 56;
    double pixelsPerBeat = 60.0, scrollBeats = 0.0;
    int scrollY = 0;
    juce::Rectangle<int> toolbar, keyboardArea, gridArea, velocityArea;

    juce::ComboBox gridBox;
    juce::TextButton quantizeButton { "Quantize" };
    DragNumber lengthBars { 0.25, 256.0, 2, " bars" };
    juce::ScrollBar vbar { true }, hbar { false };

    enum class Mode { none, move, resize, select, velocity, keyboard };
    Mode mode = Mode::none;
    std::set<size_t> selection;
    std::vector<Note> originals;
    std::vector<size_t> dragIndices;
    double downBeat = 0;
    int downPitch = 0;
    bool started = false;
    double lastLength = 0.25;
    int auditioning = -1;
    juce::Rectangle<float> rubberBand;
    float lastPlayheadX = -1;
    bool needsFit = false;
};

} // namespace az

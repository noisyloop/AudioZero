#pragma once
#include "Session.h"
#include "Theme.h"

namespace az
{

// Channel strips for every track plus the master bus.
class MixerView final : public juce::Component, private juce::ChangeListener
{
public:
    explicit MixerView(Session&);
    ~MixerView() override;
    void paint(juce::Graphics&) override;
    void resized() override;
    void timerTick();

private:
    class Strip;
    void changeListenerCallback(juce::ChangeBroadcaster*) override;
    void rebuild();

    Session& session;
    juce::Viewport viewport;
    juce::Component content;
    std::vector<std::unique_ptr<Strip>> strips;
    std::vector<int> shownIds;
};

} // namespace az

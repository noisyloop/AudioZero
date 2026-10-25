#pragma once
#include "Session.h"
#include "Theme.h"

namespace az
{

// Bottom "device view": the selected track's instrument and effect chain, left to right.
// Built-in devices show their editor inline; VST3 plugins open in their own window.
class DevicePanel final : public juce::Component,
                          public juce::DragAndDropTarget,
                          public juce::FileDragAndDropTarget,
                          private Session::Listener,
                          private juce::ChangeListener
{
public:
    explicit DevicePanel(Session&);
    ~DevicePanel() override;
    void paint(juce::Graphics&) override;
    void resized() override;

    bool isInterestedInDragSource(const SourceDetails&) override;
    void itemDragEnter(const SourceDetails&) override { dropHover = true; repaint(); }
    void itemDragExit(const SourceDetails&) override { dropHover = false; repaint(); }
    void itemDropped(const SourceDetails&) override;
    bool isInterestedInFileDrag(const juce::StringArray&) override;
    void fileDragEnter(const juce::StringArray&, int, int) override { dropHover = true; repaint(); }
    void fileDragExit(const juce::StringArray&) override { dropHover = false; repaint(); }
    void filesDropped(const juce::StringArray&, int, int) override;

private:
    class Card;
    void devicesAboutToChange() override { clear(); }
    void devicesChanged() override { rebuild(); }
    void changeListenerCallback(juce::ChangeBroadcaster*) override;
    void clear();
    void rebuild();
    void showAddMenu(juce::Component* target, bool instrument);

    Session& session;
    juce::Viewport viewport;
    juce::Component chain;
    std::vector<std::unique_ptr<juce::Component>> cards;
    int shownTrackId = -1;
    bool dropHover = false;
    std::unique_ptr<juce::FileChooser> chooser;
};

} // namespace az

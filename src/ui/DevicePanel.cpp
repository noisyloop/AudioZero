#include "DevicePanel.h"
#include "../dsp/BuiltinEffects.h"

namespace az
{

static constexpr int titleHeight = 24;

class DevicePanel::Card final : public juce::Component
{
public:
    // device == nullptr makes a placeholder ("drop an instrument" / "add effect").
    Card(DevicePanel& p, Device* d, bool instrumentSlot) : panel(p), device(d), instrument(instrumentSlot)
    {
        if (device == nullptr)
        {
            action.setButtonText(instrument ? "Choose Instrument..." : "+ Add Effect");
            action.onClick = [this] { panel.showAddMenu(&action, instrument); };
            action.setWantsKeyboardFocus(false);
            addAndMakeVisible(action);
            setSize(instrument ? 220 : 170, 100);
            return;
        }

        power.setClickingTogglesState(true);
        power.setToggleState(! device->bypassed, juce::dontSendNotification);
        power.setButtonText("ON");
        power.setTooltip("Enable / bypass");
        power.onClick = [this] {
            const bool bypass = ! power.getToggleState();
            panel.session.edit([this, bypass] { device->bypassed = bypass; }, false);
        };
        remove.setButtonText("X");
        remove.setTooltip("Remove device");
        remove.onClick = [this] {
            auto* d = device;
            auto& s = panel.session;
            const int trackId = panel.shownTrackId;
            juce::MessageManager::callAsync([&s, d, trackId] { s.removeDevice(trackId, d); });
        };
        window.setButtonText(device->isBuiltin() ? "Pop out" : "Open Editor");
        window.setTooltip("Open the device editor in its own window");
        window.onClick = [this] {
            auto& s = panel.session;
            auto* d = device;
            juce::MessageManager::callAsync([&s, d] { s.showEditorWindow(*d); });
        };
        for (auto* b : { &power, &remove, &window })
        {
            b->setWantsKeyboardFocus(false);
            addAndMakeVisible(b);
        }

        int width = 300;
        auto* processor = device->processor.get();
        if (device->isBuiltin() && processor->getActiveEditor() == nullptr)
        {
            if (auto* editor = processor->createEditorIfNeeded())
            {
                embedded.reset(editor);
                addAndMakeVisible(editor);
                width = editor->getWidth() + 12;
            }
        }
        else if (device->isBuiltin())
            info = "Editor is open in its own window";
        else
        {
            const auto& d = device->description;
            info = (d.manufacturerName.isNotEmpty() ? d.manufacturerName + "\n" : juce::String())
                 + d.pluginFormatName + (d.isInstrument ? " instrument" : " effect") + "\n"
                 + juce::String(processor->getParameters().size()) + " parameters";
            width = 240;
        }
        setSize(width, 100);
    }

    ~Card() override { embedded.reset(); }

    void paint(juce::Graphics& g) override
    {
        auto bounds = getLocalBounds().toFloat().reduced(2.0f);
        g.setColour(theme::bg1);
        g.fillRoundedRectangle(bounds, 6.0f);
        g.setColour(device == nullptr ? theme::line : (instrument ? theme::accent.withAlpha(0.6f) : theme::accent2.withAlpha(0.5f)));
        if (device == nullptr)
        {
            const float dashes[] = { 5.0f, 4.0f };
            juce::Path outline;
            outline.addRoundedRectangle(bounds, 6.0f);
            juce::Path dashed;
            juce::PathStrokeType(1.0f).createDashedStroke(dashed, outline, dashes, 2);
            g.fillPath(dashed);
            g.setColour(theme::textDim);
            g.setFont(theme::font(12.0f));
            g.drawFittedText(instrument ? "No instrument\nDrop one here from the browser\nor drag a .vst3 from your desktop"
                                        : "Drop audio effects\nor VST3 plugins here",
                             getLocalBounds().reduced(10).withTrimmedBottom(40), juce::Justification::centred, 4);
            return;
        }
        g.drawRoundedRectangle(bounds, 6.0f, 1.0f);
        auto title = getLocalBounds().removeFromTop(titleHeight).reduced(4, 2);
        g.setColour(device->bypassed ? theme::textDim : theme::text);
        g.setFont(theme::font(12.5f, true));
        g.drawText(device->displayName(), title.withTrimmedLeft(40).withTrimmedRight(window.getWidth() + 34),
                   juce::Justification::centredLeft, true);
        if (info.isNotEmpty())
        {
            g.setColour(theme::textDim);
            g.setFont(theme::font(12.0f));
            g.drawFittedText(info, getLocalBounds().withTrimmedTop(titleHeight + 10).reduced(14, 8).withTrimmedBottom(40),
                             juce::Justification::topLeft, 5);
        }
        if (device->bypassed && embedded != nullptr)
        {
            g.setColour(juce::Colours::black.withAlpha(0.35f));
            g.fillRect(embedded->getBounds());
        }
    }

    void resized() override
    {
        if (device == nullptr)
        {
            action.setBounds(getLocalBounds().reduced(16).removeFromBottom(28));
            return;
        }
        auto title = getLocalBounds().removeFromTop(titleHeight).reduced(5, 3);
        power.setBounds(title.removeFromLeft(34));
        remove.setBounds(title.removeFromRight(22));
        title.removeFromRight(4);
        window.setBounds(title.removeFromRight(device->isBuiltin() ? 62 : 90));
        if (embedded != nullptr)
            embedded->setBounds(getLocalBounds().withTrimmedTop(titleHeight).reduced(6, 3).withHeight(embedded->getHeight()));
    }

private:
    DevicePanel& panel;
    Device* device;
    bool instrument;
    juce::TextButton power, remove, window, action;
    std::unique_ptr<juce::Component> embedded;
    juce::String info;
};

//==============================================================================
DevicePanel::DevicePanel(Session& s) : session(s)
{
    viewport.setViewedComponent(&chain, false);
    viewport.setScrollBarsShown(false, true);
    viewport.setScrollBarThickness(10);
    addAndMakeVisible(viewport);
    session.addListener(this);
    session.addChangeListener(this);
    session.project.addChangeListener(this);
}

DevicePanel::~DevicePanel()
{
    clear();
    session.removeListener(this);
    session.removeChangeListener(this);
    session.project.removeChangeListener(this);
}

void DevicePanel::changeListenerCallback(juce::ChangeBroadcaster*)
{
    if (shownTrackId != session.selectedTrackId)
        rebuild();
    repaint();
}

void DevicePanel::clear()
{
    cards.clear();
    shownTrackId = -1;
}

void DevicePanel::rebuild()
{
    clear();
    auto track = session.selectedTrack();
    shownTrackId = session.selectedTrackId;
    if (track != nullptr)
    {
        cards.push_back(std::make_unique<Card>(*this, track->instrument.get(), true));
        for (auto& fx : track->effects)
            cards.push_back(std::make_unique<Card>(*this, fx.get(), false));
        cards.push_back(std::make_unique<Card>(*this, nullptr, false));
    }
    for (auto& c : cards)
        chain.addAndMakeVisible(*c);
    resized();
    repaint();
}

void DevicePanel::paint(juce::Graphics& g)
{
    g.fillAll(theme::bg0);
    if (session.selectedTrack() == nullptr)
    {
        g.setColour(theme::textDim);
        g.setFont(theme::font(14.0f));
        g.drawText("Select a track to see its instrument and effects", getLocalBounds(), juce::Justification::centred);
    }
}

void DevicePanel::resized()
{
    viewport.setBounds(getLocalBounds());
    const int height = getHeight() - 12;
    int x = 6;
    for (auto& c : cards)
    {
        c->setBounds(x, 4, c->getWidth(), height);
        x += c->getWidth() + 6;
    }
    chain.setSize(std::max(x, getWidth()), height + 6);
}

void DevicePanel::showAddMenu(juce::Component* target, bool instrument)
{
    juce::PopupMenu menu;
    std::vector<juce::String> items;
    auto add = [&](const juce::String& label, const juce::String& item) {
        items.push_back(item);
        menu.addItem(static_cast<int>(items.size()), label);
    };
    for (auto& info : Builtins::all())
        if (info.isInstrument == instrument)
            add(info.name + " (built-in)", "builtin:" + info.id);
    menu.addSeparator();
    for (auto& d : session.plugins.knownPlugins.getTypes())
        if (d.isInstrument == instrument)
            add(d.name + "  -  " + d.manufacturerName, "plugin:" + d.createIdentifierString());
    menu.addSeparator();
    menu.addItem(10000, "Browse for a .vst3 file...");
    menu.showMenuAsync(juce::PopupMenu::Options().withTargetComponent(target), [this, items](int result) {
        const int trackId = session.selectedTrackId;
        if (result == 10000)
        {
            chooser = std::make_unique<juce::FileChooser>("Choose a VST3 plugin", juce::File(), "*.vst3");
            chooser->launchAsync(juce::FileBrowserComponent::openMode | juce::FileBrowserComponent::canSelectFiles
                                     | juce::FileBrowserComponent::canSelectDirectories,
                                 [this, trackId](const juce::FileChooser& fc) {
                                     if (fc.getResult().exists())
                                         session.importFiles({ fc.getResult().getFullPathName() }, trackId, 0.0);
                                 });
            return;
        }
        if (result > 0 && result <= static_cast<int>(items.size()))
            session.applyItem(items[static_cast<size_t>(result - 1)], trackId, 0.0);
    });
}

bool DevicePanel::isInterestedInDragSource(const SourceDetails& d)
{
    const auto s = d.description.toString();
    return s.startsWith("builtin:") || s.startsWith("preset:") || s.startsWith("plugin:") || s.startsWith("file:");
}

void DevicePanel::itemDropped(const SourceDetails& d)
{
    dropHover = false;
    session.applyItem(d.description.toString(), session.selectedTrackId, session.engine.getPlayhead());
}

bool DevicePanel::isInterestedInFileDrag(const juce::StringArray& files)
{
    for (auto& f : files)
        if (PluginManager::isPluginFile(juce::File(f)))
            return true;
    return false;
}

void DevicePanel::filesDropped(const juce::StringArray& files, int, int)
{
    dropHover = false;
    session.importFiles(files, session.selectedTrackId, 0.0);
}

} // namespace az

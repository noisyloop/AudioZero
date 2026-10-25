#include "Browser.h"
#include "../dsp/OperatorSynth.h"
#include "../dsp/BuiltinEffects.h"

namespace az
{

static juce::File foldersFile() { return PluginManager::settingsFolder().getChildFile("sample-folders.txt"); }

Browser::Browser(Session& s) : session(s)
{
    search.setTextToShowWhenEmpty("Search instruments, effects, samples...", theme::textDim);
    search.setFont(theme::font(13.0f));
    search.onTextChange = [this] { rebuild(); };
    search.onEscapeKey = [this] { search.clear(); rebuild(); };
    addAndMakeVisible(search);

    list.setRowHeight(24);
    list.setColour(juce::ListBox::backgroundColourId, theme::bg1);
    list.setMultipleSelectionEnabled(false);
    list.setWantsKeyboardFocus(false);
    addAndMakeVisible(list);

    scanButton.setTooltip("Scan the standard VST3 folders (and any you added) for plugins");
    scanButton.onClick = [this] { session.plugins.startScan(false); };
    addButton.setTooltip("Add a folder of your own samples or VST3 plugins");
    addButton.onClick = [this] { showAddMenu(); };
    for (auto* b : { &scanButton, &addButton })
    {
        b->setWantsKeyboardFocus(false);
        addAndMakeVisible(b);
    }
    statusLabel.setFont(theme::font(11.0f));
    statusLabel.setColour(juce::Label::textColourId, theme::textDim);
    addAndMakeVisible(statusLabel);

    folders.addLines(foldersFile().loadFileAsString());
    folders.removeEmptyStrings();
    if (folders.isEmpty())
        for (auto location : { juce::File::userMusicDirectory, juce::File::userDesktopDirectory })
        {
            auto dir = juce::File::getSpecialLocation(location);
            if (dir.isDirectory())
            {
                folders.add(dir.getFullPathName());
                break;
            }
        }
    for (auto& f : folders)
        addSampleFolder(juce::File(f));

    session.plugins.addChangeListener(this);
    rebuild();
}

Browser::~Browser()
{
    session.plugins.removeChangeListener(this);
}

void Browser::changeListenerCallback(juce::ChangeBroadcaster*)
{
    statusLabel.setText(session.plugins.scanStatus(), juce::dontSendNotification);
    if (! session.plugins.isScanning())
        rebuild();
}

void Browser::addSampleFolder(const juce::File& folder)
{
    if (! folder.isDirectory())
        return;
    folders.addIfNotAlreadyThere(folder.getFullPathName());
    juce::Array<juce::File> files;
    const auto started = juce::Time::getMillisecondCounter();
    for (const auto& entry : juce::RangedDirectoryIterator(folder, true, "*.wav;*.aif;*.aiff;*.flac;*.mp3;*.ogg;*.mid;*.midi",
                                                           juce::File::findFiles | juce::File::ignoreHiddenFiles))
    {
        files.add(entry.getFile());
        if (files.size() >= 400 || juce::Time::getMillisecondCounter() - started > 400)
            break; // keep start-up fast on huge folders
    }
    files.sort();
    folderFiles[folder.getFullPathName()] = files;
}

void Browser::saveFolders()
{
    foldersFile().replaceWithText(folders.joinIntoString("\n"));
}

void Browser::showAddMenu()
{
    juce::PopupMenu menu;
    menu.addItem(1, "Add sample folder...");
    menu.addItem(2, "Add VST3 plugin folder...");
    menu.addItem(3, "Rescan all plugins");
    menu.showMenuAsync(juce::PopupMenu::Options().withTargetComponent(&addButton), [this](int result) {
        if (result == 3)
        {
            session.plugins.startScan(true);
            return;
        }
        if (result != 1 && result != 2)
            return;
        chooser = std::make_unique<juce::FileChooser>(result == 1 ? "Choose a sample folder" : "Choose a VST3 folder");
        chooser->launchAsync(juce::FileBrowserComponent::openMode | juce::FileBrowserComponent::canSelectDirectories,
                             [this, result](const juce::FileChooser& fc) {
                                 const auto dir = fc.getResult();
                                 if (! dir.isDirectory())
                                     return;
                                 if (result == 1)
                                 {
                                     addSampleFolder(dir);
                                     saveFolders();
                                     rebuild();
                                 }
                                 else
                                 {
                                     session.plugins.extraFolders.addIfNotAlreadyThere(dir.getFullPathName());
                                     session.plugins.startScan(false);
                                 }
                             });
    });
}

void Browser::rebuild()
{
    const auto query = search.getText().trim().toLowerCase();
    rows.clear();
    auto matches = [&](const juce::String& text) { return query.isEmpty() || text.toLowerCase().contains(query); };
    auto category = [&](const juce::String& name, std::vector<Row> items) {
        std::vector<Row> filtered;
        for (auto& r : items)
            if (matches(r.label) || matches(r.detail))
                filtered.push_back(r);
        if (filtered.empty() && query.isNotEmpty())
            return;
        rows.push_back({ name, juce::String(static_cast<int>(filtered.size())), {}, name, theme::textDim });
        if (collapsed.count(name) != 0 && query.isEmpty())
            return;
        for (auto& r : filtered)
        {
            r.category = name;
            rows.push_back(r);
        }
    };

    std::vector<Row> instruments, effects, presets, pluginInstruments, pluginEffects;
    for (auto& info : Builtins::all())
        (info.isInstrument ? instruments : effects).push_back({ info.name, info.description, "builtin:" + info.id, {}, info.isInstrument ? theme::accent : theme::accent2 });
    for (auto& preset : OperatorSynth::presetNames())
        if (preset != "Init")
            presets.push_back({ preset, "Operator preset", "preset:" + preset, {}, theme::accent.withAlpha(0.7f) });
    for (auto& d : session.plugins.knownPlugins.getTypes())
        (d.isInstrument ? pluginInstruments : pluginEffects)
            .push_back({ d.name, d.manufacturerName + " - " + d.pluginFormatName, "plugin:" + d.createIdentifierString(), {}, juce::Colour(0xff8c7bff) });

    category("INSTRUMENTS", instruments);
    category("OPERATOR PRESETS", presets);
    category("AUDIO EFFECTS", effects);
    category("PLUGIN INSTRUMENTS", pluginInstruments);
    category("PLUGIN EFFECTS", pluginEffects);
    for (auto& folder : folders)
    {
        std::vector<Row> files;
        for (auto& f : folderFiles[folder])
            files.push_back({ f.getFileName(), f.getParentDirectory().getFileName(), "file:" + f.getFullPathName(), {},
                              f.hasFileExtension("mid;midi") ? theme::accent : theme::accent2 });
        category("SAMPLES: " + juce::File(folder).getFileName().toUpperCase(), files);
    }

    if (session.plugins.knownPlugins.getNumTypes() == 0 && ! session.plugins.isScanning())
        statusLabel.setText("No plugins yet - click Scan Plugins, or drag a .vst3 onto a track", juce::dontSendNotification);
    list.updateContent();
    list.repaint();
}

void Browser::paint(juce::Graphics& g)
{
    g.fillAll(theme::bg1);
    g.setColour(theme::line);
    g.drawVerticalLine(getWidth() - 1, 0.0f, static_cast<float>(getHeight()));
    g.setColour(theme::textDim);
    g.setFont(theme::font(11.0f, true));
    g.drawText("BROWSER", getLocalBounds().removeFromTop(26).reduced(10, 0), juce::Justification::centredLeft);
}

void Browser::resized()
{
    auto area = getLocalBounds().withTrimmedRight(1);
    area.removeFromTop(26);
    search.setBounds(area.removeFromTop(28).reduced(8, 2));
    area.removeFromTop(4);
    auto bottom = area.removeFromBottom(58).reduced(8, 4);
    statusLabel.setBounds(bottom.removeFromBottom(18));
    bottom.removeFromBottom(4);
    scanButton.setBounds(bottom.removeFromLeft(bottom.getWidth() / 2 - 2));
    bottom.removeFromLeft(4);
    addButton.setBounds(bottom);
    list.setBounds(area);
}

void Browser::paintListBoxItem(int row, juce::Graphics& g, int width, int height, bool selected)
{
    if (row < 0 || row >= static_cast<int>(rows.size()))
        return;
    const auto& r = rows[static_cast<size_t>(row)];
    if (r.item.isEmpty())
    {
        g.setColour(theme::bg0.withAlpha(0.6f));
        g.fillRect(0, 0, width, height);
        juce::Path arrow;
        const bool isCollapsed = collapsed.count(r.label) != 0 && search.isEmpty();
        if (isCollapsed)
            arrow.addTriangle(10, 8, 10, 16, 15, 12);
        else
            arrow.addTriangle(8, 10, 16, 10, 12, 15);
        g.setColour(theme::textDim);
        g.fillPath(arrow);
        g.setFont(theme::font(11.0f, true));
        g.drawText(r.label, 22, 0, width - 60, height, juce::Justification::centredLeft);
        g.drawText(r.detail, width - 44, 0, 36, height, juce::Justification::centredRight);
        return;
    }
    if (selected)
    {
        g.setColour(theme::accent.withAlpha(0.18f));
        g.fillRect(0, 0, width, height);
    }
    g.setColour(r.colour);
    g.fillRoundedRectangle(14.0f, height * 0.5f - 4.0f, 8.0f, 8.0f, 2.0f);
    g.setColour(theme::text);
    g.setFont(theme::font(13.0f));
    g.drawText(r.label, 30, 0, width - 34, height, juce::Justification::centredLeft, true);
}

void Browser::listBoxItemClicked(int row, const juce::MouseEvent&)
{
    if (row < 0 || row >= static_cast<int>(rows.size()))
        return;
    const auto& r = rows[static_cast<size_t>(row)];
    if (r.item.isEmpty())
    {
        if (collapsed.count(r.label) != 0)
            collapsed.erase(r.label);
        else
            collapsed.insert(r.label);
        rebuild();
        return;
    }
    if (r.item.startsWith("file:"))
    {
        const juce::File file(r.item.fromFirstOccurrenceOf(":", false, false));
        if (AudioSample::isAudioFile(file))
        {
            juce::String error;
            if (auto sample = AudioSample::load(file, error))
                session.engine.previewSample(sample);
        }
    }
}

void Browser::listBoxItemDoubleClicked(int row, const juce::MouseEvent&)
{
    if (row < 0 || row >= static_cast<int>(rows.size()) || rows[static_cast<size_t>(row)].item.isEmpty())
        return;
    session.applyItem(rows[static_cast<size_t>(row)].item, session.selectedTrackId, session.engine.getPlayhead());
}

juce::var Browser::getDragSourceDescription(const juce::SparseSet<int>& selected)
{
    if (selected.isEmpty())
        return {};
    const int row = selected[0];
    if (row < 0 || row >= static_cast<int>(rows.size()))
        return {};
    return rows[static_cast<size_t>(row)].item.isEmpty() ? juce::var() : juce::var(rows[static_cast<size_t>(row)].item);
}

juce::String Browser::getTooltipForRow(int row)
{
    if (row < 0 || row >= static_cast<int>(rows.size()))
        return {};
    const auto& r = rows[static_cast<size_t>(row)];
    return r.item.isEmpty() ? "Click to expand/collapse" : r.detail + " - drag onto a track, or double-click to add to the selected track";
}

} // namespace az

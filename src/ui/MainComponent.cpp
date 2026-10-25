#include "MainComponent.h"

namespace az
{

static const char* noteKeys = "awsedftgyhujkolp;'";

class MainComponent::Divider final : public juce::Component
{
public:
    Divider(bool isVertical, std::function<void(int)> onDrag) : vertical(isVertical), drag(std::move(onDrag))
    {
        setMouseCursor(vertical ? juce::MouseCursor::LeftRightResizeCursor : juce::MouseCursor::UpDownResizeCursor);
    }
    void paint(juce::Graphics& g) override
    {
        g.fillAll(theme::bg0);
        g.setColour(isMouseOverOrDragging() ? theme::accent : theme::line);
        if (vertical)
            g.drawVerticalLine(getWidth() / 2, 0.0f, static_cast<float>(getHeight()));
        else
            g.drawHorizontalLine(getHeight() / 2, 0.0f, static_cast<float>(getWidth()));
    }
    void mouseDown(const juce::MouseEvent&) override { last = 0; }
    void mouseDrag(const juce::MouseEvent& e) override
    {
        const int d = vertical ? e.getDistanceFromDragStartX() : e.getDistanceFromDragStartY();
        drag(d - last);
        last = d;
    }
    void mouseEnter(const juce::MouseEvent&) override { repaint(); }
    void mouseExit(const juce::MouseEvent&) override { repaint(); }

private:
    bool vertical;
    std::function<void(int)> drag;
    int last = 0;
};

juce::PropertiesFile& MainComponent::settings()
{
    static juce::PropertiesFile file([] {
        juce::PropertiesFile::Options o;
        o.applicationName = "AudioZero";
        o.filenameSuffix = ".settings";
        o.folderName = "NOISYLOOP/AudioZero";
        o.osxLibrarySubFolder = "Application Support";
        return o;
    }());
    return file;
}

//==============================================================================
MainComponent::MainComponent(const juce::File& startupFile, bool forceDemo)
{
    session.onStatus = [this](const juce::String& text) { setStatus(text); };
    session.onEditClip = [this] { showTab(Tab::pianoRoll); };

    addAndMakeVisible(transport);
    addAndMakeVisible(browser);
    addAndMakeVisible(arrangement);
    addChildComponent(devicePanel);
    addChildComponent(pianoRoll);
    addChildComponent(mixer);

    browserDivider = std::make_unique<Divider>(true, [this](int d) {
        browserWidth = juce::jlimit(170, 420, browserWidth + d);
        resized();
    });
    bottomDivider = std::make_unique<Divider>(false, [this](int d) {
        bottomHeight = juce::jlimit(140, std::max(160, getHeight() - 260), bottomHeight - d);
        resized();
    });
    addAndMakeVisible(*browserDivider);
    addAndMakeVisible(*bottomDivider);

    int index = 0;
    for (auto* b : { &devicesTab, &pianoRollTab, &mixerTab })
    {
        b->setClickingTogglesState(false);
        b->setWantsKeyboardFocus(false);
        b->setConnectedEdges(juce::Button::ConnectedOnLeft | juce::Button::ConnectedOnRight);
        const auto t = static_cast<Tab>(index++);
        b->onClick = [this, t] { showTab(t); };
        addAndMakeVisible(b);
    }
    devicesTab.setTooltip("Instrument and effects of the selected track (D)");
    pianoRollTab.setTooltip("Notes of the selected MIDI clip (P)");
    mixerTab.setTooltip("Volume, pan, mute and solo for every track (M)");

    if (session.audioError.isNotEmpty())
        setStatus("Audio device problem: " + session.audioError + "  (Options > Audio Settings)");
    else
        setStatus("Welcome to AudioZero. Drag instruments, samples, MIDI files and VST3 plugins onto tracks. Space = play.");

    auto last = juce::File(settings().getValue("lastProject"));
    if (startupFile.existsAsFile())
        loadProject(startupFile);
    else if (! forceDemo && last.existsAsFile())
        loadProject(last);
    else
        session.install(io::createDemo(session.project, session.plugins, session.engine), {});

    showTab(Tab::devices);
    setWantsKeyboardFocus(true);
    juce::Desktop::getInstance().addFocusChangeListener(this);
    setSize(1600, 980);
    startTimerHz(30);
}

MainComponent::~MainComponent()
{
    stopTimer();
    juce::Desktop::getInstance().removeFocusChangeListener(this);
#if JUCE_MAC
    juce::MenuBarModel::setMacMainMenu(nullptr);
#endif
    session.closeEditorWindows();
}

void MainComponent::globalFocusChanged(juce::Component* focused)
{
    // Keep keyboard focus here (for shortcuts and the computer keyboard) unless typing text.
    if (focused == nullptr || focused == this || ! isParentOf(focused))
        return;
    if (dynamic_cast<juce::TextEditor*>(focused) != nullptr)
        return;
    juce::Component::SafePointer<MainComponent> safe(this);
    juce::MessageManager::callAsync([safe] {
        if (safe != nullptr && safe->isShowing() && dynamic_cast<juce::TextEditor*>(juce::Component::getCurrentlyFocusedComponent()) == nullptr)
            safe->grabKeyboardFocus();
    });
}

void MainComponent::setStatus(const juce::String& text)
{
    statusText = text;
    repaint(statusArea);
}

juce::String MainComponent::windowTitle() const
{
    const auto name = session.project.file == juce::File() ? juce::String("Untitled") : session.project.file.getFileNameWithoutExtension();
    return "AudioZero - " + name + (session.project.isDirty() ? " *" : "");
}

void MainComponent::showTab(Tab t)
{
    tab = t;
    devicePanel.setVisible(t == Tab::devices);
    pianoRoll.setVisible(t == Tab::pianoRoll);
    mixer.setVisible(t == Tab::mixer);
    devicesTab.setToggleState(t == Tab::devices, juce::dontSendNotification);
    pianoRollTab.setToggleState(t == Tab::pianoRoll, juce::dontSendNotification);
    mixerTab.setToggleState(t == Tab::mixer, juce::dontSendNotification);
    resized();
}

void MainComponent::timerCallback()
{
    session.updateRecording();
    transport.refresh();
    arrangement.timerTick(transport.follow());
    if (pianoRoll.isVisible())
        pianoRoll.timerTick();
    mixer.timerTick();
    const bool dirty = session.project.isDirty();
    if (dirty != lastDirty || session.project.version() != lastVersion)
    {
        lastDirty = dirty;
        lastVersion = session.project.version();
        if (onTitleChanged)
            onTitleChanged();
    }
}

//==============================================================================
void MainComponent::paint(juce::Graphics& g)
{
    g.fillAll(theme::bg0);
    g.setColour(theme::bg1);
    g.fillRect(bottomHeader);
    if (auto track = session.selectedTrack())
    {
        auto r = bottomHeader.withTrimmedLeft(340).reduced(8, 0);
        g.setColour(track->data.colour);
        g.fillRoundedRectangle(r.removeFromLeft(10).withSizeKeepingCentre(10, 10).toFloat(), 2.0f);
        g.setColour(theme::text);
        g.setFont(theme::font(12.5f, true));
        g.drawText(track->data.name, r.withTrimmedLeft(8), juce::Justification::centredLeft, true);
    }

    g.setColour(theme::bg1);
    g.fillRect(statusArea);
    g.setColour(theme::line);
    g.drawHorizontalLine(statusArea.getY(), 0.0f, static_cast<float>(getWidth()));
    g.setColour(theme::textDim);
    g.setFont(theme::font(11.5f));
    g.drawText(statusText, statusArea.reduced(10, 0), juce::Justification::centredLeft, true);
    g.drawText("Space play  |  Dbl-click lane: new clip  |  Cmd/Ctrl+Z undo  |  A-K play notes",
               statusArea.reduced(10, 0), juce::Justification::centredRight, true);
}

void MainComponent::resized()
{
    auto area = getLocalBounds();
    transport.setBounds(area.removeFromTop(50));
    statusArea = area.removeFromBottom(22);
    browser.setBounds(area.removeFromLeft(browserWidth));
    browserDivider->setBounds(area.removeFromLeft(5));

    auto bottom = area.removeFromBottom(juce::jlimit(140, std::max(140, getHeight() - 260), bottomHeight));
    bottomDivider->setBounds(area.removeFromBottom(5));
    arrangement.setBounds(area);

    bottomHeader = bottom.removeFromTop(30);
    auto tabs = bottomHeader.reduced(6, 4);
    for (auto* b : { &devicesTab, &pianoRollTab, &mixerTab })
        b->setBounds(tabs.removeFromLeft(106));
    for (juce::Component* c : { static_cast<juce::Component*>(&devicePanel), static_cast<juce::Component*>(&pianoRoll), static_cast<juce::Component*>(&mixer) })
        c->setBounds(bottom);
}

//==============================================================================
bool MainComponent::keyPressed(const juce::KeyPress& key)
{
    const auto mods = key.getModifiers();
    const int code = key.getKeyCode();
    const bool cmd = mods.isCommandDown();
    const bool pianoFocus = tab == Tab::pianoRoll && pianoRoll.isVisible();

    if (cmd)
    {
        const auto lower = juce::CharacterFunctions::toLowerCase(static_cast<juce::juce_wchar>(code));
        switch (lower)
        {
            case 'z': perform(mods.isShiftDown() ? cmdRedo : cmdUndo); return true;
            case 'y': perform(cmdRedo); return true;
            case 's': perform(mods.isShiftDown() ? cmdSaveAs : cmdSave); return true;
            case 'o': perform(cmdOpen); return true;
            case 'n': perform(cmdNew); return true;
            case 'e': perform(cmdSplit); return true;
            case 'd': perform(cmdDuplicate); return true;
            case 'c': perform(cmdCopy); return true;
            case 'v': perform(cmdPaste); return true;
            case 'a': perform(cmdSelectAll); return true;
            case 'l': perform(cmdLoop); return true;
            case 't': perform(mods.isShiftDown() ? cmdAddAudioTrack : cmdAddInstrumentTrack); return true;
            case 'r': perform(cmdExport); return true;
            case 'm': perform(cmdAddMidiClip); return true;
            case 'i': perform(cmdImport); return true;
            default: break;
        }
        return false;
    }

    if (code == juce::KeyPress::spaceKey)
    {
        session.engine.togglePlay();
        return true;
    }
    if (code == juce::KeyPress::deleteKey || code == juce::KeyPress::backspaceKey)
    {
        perform(cmdDelete);
        return true;
    }
    if (code == juce::KeyPress::homeKey)
    {
        session.engine.setPlayhead(0.0);
        return true;
    }
    if (pianoFocus && (code == juce::KeyPress::upKey || code == juce::KeyPress::downKey))
    {
        pianoRoll.transposeSelected((code == juce::KeyPress::upKey ? 1 : -1) * (mods.isShiftDown() ? 12 : 1));
        return true;
    }
    if (code == juce::KeyPress::tabKey)
    {
        showTab(tab == Tab::devices ? Tab::pianoRoll : tab == Tab::pianoRoll ? Tab::mixer : Tab::devices);
        return true;
    }

    const auto c = juce::CharacterFunctions::toLowerCase(static_cast<juce::juce_wchar>(code));
    if (transport.keyboardEnabled())
    {
        if (juce::String(noteKeys).containsChar(c))
            return true; // handled in keyStateChanged
        if (c == 'z' || c == 'x')
        {
            transport.octave = juce::jlimit(-1, 8, transport.octave + (c == 'z' ? -1 : 1));
            setStatus("Computer keyboard octave: C" + juce::String(transport.octave));
            return true;
        }
        if (c == 'c' || c == 'v')
        {
            transport.keyboardVelocity = juce::jlimit(0.1f, 1.0f, transport.keyboardVelocity + (c == 'c' ? -0.1f : 0.1f));
            setStatus("Computer keyboard velocity: " + juce::String(juce::roundToInt(transport.keyboardVelocity * 127)));
            return true;
        }
    }
    if (c == 'r')
    {
        session.toggleRecord();
        return true;
    }
    if (c == 'q' && pianoFocus)
    {
        pianoRoll.quantize();
        return true;
    }
    return false;
}

bool MainComponent::keyStateChanged(bool)
{
    if (! transport.keyboardEnabled() || dynamic_cast<juce::TextEditor*>(juce::Component::getCurrentlyFocusedComponent()) != nullptr)
        return false;
    if (juce::ModifierKeys::getCurrentModifiers().isCommandDown())
        return false;
    bool used = false;
    const int base = (transport.octave + 1) * 12;
    for (int i = 0; noteKeys[i] != 0; ++i)
    {
        const bool down = juce::KeyPress::isKeyCurrentlyDown(noteKeys[i]);
        const int note = juce::jlimit(0, 127, base + i);
        if (down && heldKeys.count(i) == 0)
        {
            heldKeys.insert(i);
            session.noteOn(note, transport.keyboardVelocity);
            used = true;
        }
        else if (! down && heldKeys.count(i) != 0)
        {
            heldKeys.erase(i);
            session.noteOff(note);
            used = true;
        }
    }
    return used;
}

//==============================================================================
juce::StringArray MainComponent::getMenuBarNames()
{
    return { "File", "Edit", "Create", "Options", "Help" };
}

juce::PopupMenu MainComponent::getMenuForIndex(int index, const juce::String&)
{
    juce::PopupMenu m;
    const auto cmdKey = [](const char* k) {
#if JUCE_MAC
        return juce::String("Cmd+") + k;
#else
        return juce::String("Ctrl+") + k;
#endif
    };
    auto item = [&m](int id, const juce::String& text, const juce::String& shortcut = {}, bool enabled = true, bool ticked = false) {
        juce::PopupMenu::Item i(text);
        i.itemID = id;
        i.isEnabled = enabled;
        i.isTicked = ticked;
        i.shortcutKeyDescription = shortcut;
        m.addItem(i);
    };
    switch (index)
    {
        case 0:
            item(cmdNew, "New Project", cmdKey("N"));
            item(cmdOpen, "Open Project...", cmdKey("O"));
            item(cmdOpenDemo, "Open Demo Song");
            m.addSeparator();
            item(cmdSave, "Save", cmdKey("S"));
            item(cmdSaveAs, "Save As...", cmdKey("Shift+S"));
            m.addSeparator();
            item(cmdImport, "Import Audio / MIDI / VST3...", cmdKey("I"));
            item(cmdExport, "Export to WAV...", cmdKey("R"));
#if ! JUCE_MAC
            m.addSeparator();
            item(cmdQuit, "Quit");
#endif
            break;
        case 1:
            item(cmdUndo, "Undo", cmdKey("Z"), session.project.canUndo());
            item(cmdRedo, "Redo", cmdKey("Shift+Z"), session.project.canRedo());
            m.addSeparator();
            item(cmdCopy, "Copy Clip", cmdKey("C"), session.selectedClipId != 0);
            item(cmdPaste, "Paste Clip", cmdKey("V"), session.hasClipboard());
            item(cmdDuplicate, "Duplicate Clip", cmdKey("D"), session.selectedClipId != 0);
            item(cmdSplit, "Split Clip at Playhead", cmdKey("E"), session.selectedClipId != 0);
            item(cmdDelete, "Delete", "Del");
            item(cmdSelectAll, "Select All Notes", cmdKey("A"));
            break;
        case 2:
            item(cmdAddInstrumentTrack, "Instrument Track (Operator)", cmdKey("T"));
            item(cmdAddAudioTrack, "Audio Track", cmdKey("Shift+T"));
            item(cmdAddMidiClip, "MIDI Clip at Playhead", cmdKey("M"), session.selectedTrack() != nullptr);
            break;
        case 3:
            item(cmdAudioSettings, "Audio / MIDI Settings...");
            item(cmdScanPlugins, "Scan for VST3 Plugins");
            m.addSeparator();
            item(cmdKeyboard, "Computer MIDI Keyboard", {}, true, transport.keyboardEnabled());
            item(cmdMetronome, "Metronome", {}, true, session.engine.isMetronomeOn());
            item(cmdLoop, "Loop", cmdKey("L"), true, session.project.loopEnabled);
            break;
        case 4:
            item(cmdShortcuts, "Keyboard Shortcuts");
            item(cmdAbout, "About AudioZero");
            break;
        default: break;
    }
    return m;
}

void MainComponent::menuItemSelected(int itemId, int)
{
    perform(itemId);
}

void MainComponent::perform(int command)
{
    auto& s = session;
    const bool pianoFocus = tab == Tab::pianoRoll && pianoRoll.isVisible();
    switch (command)
    {
        case cmdNew:
            confirmDiscard([this] {
                session.install(io::createEmpty(session.project, session.plugins, session.engine), {});
                setStatus("New project");
            });
            break;
        case cmdOpen:
            confirmDiscard([this] {
                chooser = std::make_unique<juce::FileChooser>("Open AudioZero project", juce::File(), "*.azproj");
                chooser->launchAsync(juce::FileBrowserComponent::openMode | juce::FileBrowserComponent::canSelectFiles,
                                     [this](const juce::FileChooser& fc) {
                                         if (fc.getResult().existsAsFile())
                                             loadProject(fc.getResult());
                                     });
            });
            break;
        case cmdOpenDemo:
            confirmDiscard([this] {
                session.install(io::createDemo(session.project, session.plugins, session.engine), {});
                setStatus("Demo song loaded - press Space to play");
            });
            break;
        case cmdSave: saveProject(false); break;
        case cmdSaveAs: saveProject(true); break;
        case cmdExport: exportAudio(); break;
        case cmdQuit: juce::JUCEApplication::getInstance()->systemRequestedQuit(); break;
        case cmdImport:
            chooser = std::make_unique<juce::FileChooser>("Import audio, MIDI or VST3", juce::File(), "*.wav;*.aif;*.aiff;*.flac;*.mp3;*.ogg;*.mid;*.midi;*.vst3");
            chooser->launchAsync(juce::FileBrowserComponent::openMode | juce::FileBrowserComponent::canSelectFiles
                                     | juce::FileBrowserComponent::canSelectMultipleItems,
                                 [this](const juce::FileChooser& fc) {
                                     juce::StringArray paths;
                                     for (auto& f : fc.getResults())
                                         paths.add(f.getFullPathName());
                                     session.importFiles(paths, session.selectedTrackId, session.engine.getPlayhead());
                                 });
            break;
        case cmdUndo:
            if (s.project.undo())
                setStatus("Undo");
            break;
        case cmdRedo:
            if (s.project.redo())
                setStatus("Redo");
            break;
        case cmdDelete:
            if (pianoFocus && pianoRoll.hasSelection())
                pianoRoll.deleteSelected();
            else if (s.selectedClipId != 0)
                s.deleteClip(s.selectedClipId);
            break;
        case cmdDuplicate:
            if (s.selectedClipId != 0)
                s.duplicateClip(s.selectedClipId);
            break;
        case cmdCopy:
            if (s.selectedClipId != 0)
            {
                s.copyClip(s.selectedClipId);
                setStatus("Clip copied - Cmd/Ctrl+V pastes at the playhead");
            }
            break;
        case cmdPaste: s.pasteClip(s.selectedTrackId, s.engine.getPlayhead()); break;
        case cmdSplit:
            if (s.selectedClipId != 0)
                s.splitClip(s.selectedClipId, s.engine.getPlayhead());
            break;
        case cmdSelectAll:
            showTab(Tab::pianoRoll);
            pianoRoll.selectAll();
            break;
        case cmdAddInstrumentTrack: s.addTrack(true); break;
        case cmdAddAudioTrack: s.addTrack(false); break;
        case cmdAddMidiClip:
            if (auto track = s.selectedTrack())
            {
                const double bar = s.project.beatsPerBar;
                s.editClip(s.createMidiClip(track->id, std::floor(s.engine.getPlayhead() / bar) * bar, bar));
            }
            break;
        case cmdAudioSettings:
        {
            auto* selector = new juce::AudioDeviceSelectorComponent(s.deviceManager, 0, 2, 0, 2, true, false, true, false);
            selector->setSize(520, 420);
            juce::DialogWindow::LaunchOptions o;
            o.content.setOwned(selector);
            o.dialogTitle = "Audio / MIDI Settings";
            o.dialogBackgroundColour = theme::bg1;
            o.useNativeTitleBar = true;
            o.resizable = true;
            o.launchAsync();
            break;
        }
        case cmdScanPlugins: s.plugins.startScan(false); break;
        case cmdKeyboard: transport.repaint(); break;
        case cmdMetronome: s.engine.setMetronome(! s.engine.isMetronomeOn()); break;
        case cmdLoop: s.edit([&s] { s.project.loopEnabled = ! s.project.loopEnabled; }); break;
        case cmdShortcuts:
            juce::AlertWindow::showMessageBoxAsync(juce::MessageBoxIconType::NoIcon, "Keyboard Shortcuts",
                "Space  Play / pause\n"
                "Home  Go to start\n"
                "R  Record MIDI\n"
                "Cmd/Ctrl+L  Loop on/off\n"
                "Cmd/Ctrl+Z / Shift+Z  Undo / redo\n"
                "Cmd/Ctrl+T / Shift+T  New instrument / audio track\n"
                "Cmd/Ctrl+M  New MIDI clip at playhead\n"
                "Cmd/Ctrl+D  Duplicate clip\n"
                "Cmd/Ctrl+E  Split clip at playhead\n"
                "Cmd/Ctrl+C / V  Copy / paste clip\n"
                "Delete  Delete clip or selected notes\n"
                "Tab  Switch Devices / Piano Roll / Mixer\n"
                "A W S E D F T G Y H U J K  Play notes\n"
                "Z / X  Octave down / up,  C / V  velocity\n"
                "Piano roll: Up/Down transpose, Q quantize, Alt-drag copy\n"
                "Arrangement: Alt-drag copies a clip, Ctrl/Cmd+wheel zooms");
            break;
        case cmdAbout:
            juce::AlertWindow::showMessageBoxAsync(juce::MessageBoxIconType::NoIcon, "AudioZero",
                "AudioZero " + juce::JUCEApplication::getInstance()->getApplicationVersion()
                    + "\nNOISYLOOP\n\nArrangement, built-in Operator FM synth and VST3 hosting.\nBuilt with JUCE.");
            break;
        default: break;
    }
    menuItemsChanged();
}

//==============================================================================
void MainComponent::rememberProject(const juce::File& file)
{
    settings().setValue("lastProject", file.getFullPathName());
    settings().saveIfNeeded();
}

void MainComponent::loadProject(const juce::File& file)
{
    io::Loaded loaded;
    juce::String error;
    juce::StringArray warnings;
    if (! io::load(file, session.project, session.plugins, session.engine, loaded, error, warnings))
    {
        juce::AlertWindow::showMessageBoxAsync(juce::MessageBoxIconType::WarningIcon, "Could not open project", error);
        return;
    }
    session.install(std::move(loaded), file);
    rememberProject(file);
    setStatus(warnings.isEmpty() ? "Opened " + file.getFileName() : warnings.joinIntoString("; "));
}

void MainComponent::saveProject(bool saveAs, std::function<void()> then)
{
    auto doSave = [this, then](const juce::File& file) {
        juce::String error;
        if (! io::save(session.project, file, error))
        {
            juce::AlertWindow::showMessageBoxAsync(juce::MessageBoxIconType::WarningIcon, "Save failed", error);
            return;
        }
        session.project.file = file;
        session.project.savedVersion = session.project.version();
        rememberProject(file);
        setStatus("Saved " + file.getFullPathName());
        if (onTitleChanged)
            onTitleChanged();
        if (then)
            then();
    };
    if (! saveAs && session.project.file != juce::File())
    {
        doSave(session.project.file);
        return;
    }
    auto start = juce::File::getSpecialLocation(juce::File::userMusicDirectory).getChildFile("AudioZero");
    start.createDirectory();
    chooser = std::make_unique<juce::FileChooser>("Save project", start.getChildFile("My Song.azproj"), "*.azproj");
    chooser->launchAsync(juce::FileBrowserComponent::saveMode | juce::FileBrowserComponent::warnAboutOverwriting,
                         [doSave](const juce::FileChooser& fc) {
                             auto file = fc.getResult();
                             if (file != juce::File())
                                 doSave(file.withFileExtension("azproj"));
                         });
}

void MainComponent::exportAudio()
{
    const auto name = session.project.file == juce::File() ? juce::String("AudioZero Export") : session.project.file.getFileNameWithoutExtension();
    chooser = std::make_unique<juce::FileChooser>("Export WAV", juce::File::getSpecialLocation(juce::File::userMusicDirectory).getChildFile(name + ".wav"), "*.wav");
    chooser->launchAsync(juce::FileBrowserComponent::saveMode | juce::FileBrowserComponent::warnAboutOverwriting,
                         [this](const juce::FileChooser& fc) {
                             auto file = fc.getResult();
                             if (file == juce::File())
                                 return;
                             file = file.withFileExtension("wav");
                             const double end = std::max(4.0, session.project.songEndBeats());
                             session.deviceManager.removeAudioCallback(&session.engine);
                             juce::String error;
                             const bool ok = session.engine.exportWav(file, end, error);
                             session.deviceManager.addAudioCallback(&session.engine);
                             setStatus(ok ? "Exported " + file.getFullPathName() : "Export failed: " + error);
                         });
}

void MainComponent::confirmDiscard(std::function<void()> then)
{
    if (! session.project.isDirty())
    {
        then();
        return;
    }
    auto options = juce::MessageBoxOptions::makeOptionsYesNoCancel(juce::MessageBoxIconType::QuestionIcon, "Save changes?",
                                                                   "Do you want to save the changes to this project?",
                                                                   "Save", "Don't Save", "Cancel", this);
    juce::AlertWindow::showAsync(options, [this, then](int result) {
        if (result == 1)
            saveProject(false, then);
        else if (result == 2)
            then();
    });
}

void MainComponent::requestQuit(std::function<void()> quit)
{
    confirmDiscard(std::move(quit));
}

} // namespace az

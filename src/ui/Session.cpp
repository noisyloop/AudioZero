#include "Session.h"
#include "../dsp/OperatorSynth.h"
#include "../dsp/BuiltinEffects.h"

namespace az
{

class Session::EditorWindow final : public juce::DocumentWindow
{
public:
    EditorWindow(Device& d, Session& s)
        : DocumentWindow(d.displayName(), juce::Colour(0xff1a1d22), DocumentWindow::closeButton | DocumentWindow::minimiseButton),
          device(&d), session(s)
    {
        setUsingNativeTitleBar(true);
        juce::AudioProcessorEditor* editor = d.processor->hasEditor() ? d.processor->createEditorIfNeeded() : nullptr;
        if (editor == nullptr)
            editor = new juce::GenericAudioProcessorEditor(*d.processor);
        setContentOwned(editor, true);
        setResizable(editor->isResizable(), false);
        setAlwaysOnTop(true);
        centreWithSize(getWidth(), getHeight());
        setVisible(true);
    }
    void closeButtonPressed() override
    {
        juce::MessageManager::callAsync([s = &session, d = device] { s->closeEditorWindows(d); });
    }
    Device* device;

private:
    Session& session;
};

Session::Session()
{
    juce::RuntimePermissions::request(juce::RuntimePermissions::recordAudio, [](bool) {});
    audioError = deviceManager.initialiseWithDefaultDevices(0, 2);
    deviceManager.addAudioCallback(&engine);
    // Enable every MIDI input so controllers work without setup.
    for (auto& input : juce::MidiInput::getAvailableDevices())
        deviceManager.setMidiInputDeviceEnabled(input.identifier, true);
    deviceManager.addMidiInputDeviceCallback({}, &engine);
}

Session::~Session()
{
    closeEditorWindows();
    deviceManager.removeMidiInputDeviceCallback({}, &engine);
    deviceManager.removeAudioCallback(&engine);
    deviceManager.closeAudioDevice();
}

//==============================================================================
void Session::selectTrack(int trackId)
{
    if (selectedTrackId == trackId)
        return;
    selectedTrackId = trackId;
    engine.setLiveTrack(trackId);
    sendChangeMessage();
}

void Session::selectClip(int clipId)
{
    selectedClipId = clipId;
    if (auto track = project.trackOwningClip(clipId))
    {
        selectTrack(track->id);
        if (auto* clip = track->findClip(clipId); clip != nullptr && clip->kind == ClipKind::midi)
            editClipId = clipId;
    }
    sendChangeMessage();
}

void Session::editClip(int clipId)
{
    selectClip(clipId);
    if (auto* clip = findClip(clipId); clip != nullptr && clip->kind == ClipKind::midi && onEditClip)
        onEditClip();
}

Clip* Session::findClip(int clipId) const
{
    if (auto track = project.trackOwningClip(clipId))
        return track->findClip(clipId);
    return nullptr;
}

void Session::install(io::Loaded&& loaded, const juce::File& file)
{
    closeEditorWindows();
    listeners.call([](Listener& l) { l.devicesAboutToChange(); });
    engine.stop();
    engine.stop(); // second stop rewinds to the start
    recordings.clear();
    io::install(project, std::move(loaded), file);
    selectedClipId = editClipId = 0;
    selectedTrackId = -1;
    selectTrack(project.tracks.empty() ? 0 : project.tracks.front()->id);
    listeners.call([](Listener& l) { l.devicesChanged(); });
    sendChangeMessage();
}

//==============================================================================
std::shared_ptr<Track> Session::addTrack(bool withOperator, int insertIndex, const juce::String& name)
{
    auto track = io::makeTrack(project, name.isNotEmpty() ? name : (withOperator ? juce::String("Operator") : juce::String("Audio")),
                               project.nextTrackColour());
    if (withOperator)
        track->instrument = plugins.createBuiltin("operator");
    engine.prepareTrack(*track);
    edit([&] {
        if (insertIndex < 0 || insertIndex > static_cast<int>(project.tracks.size()))
            project.tracks.push_back(track);
        else
            project.tracks.insert(project.tracks.begin() + insertIndex, track);
    });
    selectTrack(track->id);
    return track;
}

void Session::deleteTrack(int trackId)
{
    auto track = project.findTrack(trackId);
    if (track == nullptr)
        return;
    listeners.call([](Listener& l) { l.devicesAboutToChange(); });
    if (track->instrument != nullptr)
        closeEditorWindows(track->instrument.get());
    for (auto& fx : track->effects)
        closeEditorWindows(fx.get());
    const int index = project.indexOfTrack(trackId);
    edit([&] { project.tracks.erase(project.tracks.begin() + index); });
    if (! project.tracks.empty())
        selectTrack(project.tracks[static_cast<size_t>(std::min(index, static_cast<int>(project.tracks.size()) - 1))]->id);
    else
        selectTrack(0);
    listeners.call([](Listener& l) { l.devicesChanged(); });
}

void Session::duplicateTrack(int trackId)
{
    auto source = project.findTrack(trackId);
    if (source == nullptr)
        return;
    auto copy = io::makeTrack(project, source->data.name + " copy", source->data.colour);
    copy->data = source->data;
    copy->data.name = source->data.name + " copy";
    for (auto& clip : copy->data.clips)
        clip.id = project.newId();
    auto cloneDevice = [this](const Device& d) -> std::unique_ptr<Device> {
        juce::String error;
        auto out = d.isBuiltin() ? plugins.createBuiltin(d.builtinId) : plugins.createPlugin(d.description, engine.getSampleRate(), engine.getBlockSize(), error);
        if (out != nullptr)
        {
            juce::MemoryBlock state;
            d.processor->getStateInformation(state);
            out->processor->setStateInformation(state.getData(), static_cast<int>(state.getSize()));
            out->bypassed = d.bypassed;
        }
        return out;
    };
    if (source->instrument != nullptr)
        copy->instrument = cloneDevice(*source->instrument);
    for (auto& fx : source->effects)
        if (auto c = cloneDevice(*fx))
            copy->effects.push_back(std::move(c));
    engine.prepareTrack(*copy);
    const int index = project.indexOfTrack(trackId);
    edit([&] { project.tracks.insert(project.tracks.begin() + index + 1, copy); });
    selectTrack(copy->id);
}

void Session::moveTrack(int trackId, int delta)
{
    const int index = project.indexOfTrack(trackId);
    const int target = index + delta;
    if (index < 0 || target < 0 || target >= static_cast<int>(project.tracks.size()))
        return;
    edit([&] { std::swap(project.tracks[static_cast<size_t>(index)], project.tracks[static_cast<size_t>(target)]); });
}

//==============================================================================
int Session::createMidiClip(int trackId, double beat, double length)
{
    auto track = project.findTrack(trackId);
    if (track == nullptr)
        return 0;
    auto clip = io::makeMidiClip(project, std::max(0.0, beat), length, track->data.colour);
    edit([&] { track->data.clips.push_back(clip); });
    return clip.id;
}

void Session::deleteClip(int clipId)
{
    auto track = project.trackOwningClip(clipId);
    if (track == nullptr)
        return;
    edit([&] {
        auto& clips = track->data.clips;
        clips.erase(std::remove_if(clips.begin(), clips.end(), [clipId](const Clip& c) { return c.id == clipId; }), clips.end());
    });
    if (selectedClipId == clipId)
        selectedClipId = 0;
    if (editClipId == clipId)
        editClipId = 0;
    sendChangeMessage();
}

int Session::duplicateClip(int clipId)
{
    auto track = project.trackOwningClip(clipId);
    if (track == nullptr)
        return 0;
    Clip copy = *track->findClip(clipId);
    copy.id = project.newId();
    copy.start = copy.end();
    edit([&] { track->data.clips.push_back(copy); });
    selectClip(copy.id);
    return copy.id;
}

void Session::splitClip(int clipId, double beat)
{
    auto track = project.trackOwningClip(clipId);
    auto* clip = track != nullptr ? track->findClip(clipId) : nullptr;
    if (clip == nullptr || beat <= clip->start + 1e-6 || beat >= clip->end() - 1e-6)
        return;
    Clip right = *clip;
    right.id = project.newId();
    const double cut = beat - clip->start;
    right.start = beat;
    right.length = clip->end() - beat;
    edit([&] {
        auto* left = track->findClip(clipId);
        left->length = cut;
        if (left->kind == ClipKind::midi)
        {
            std::vector<Note> keep, moved;
            for (auto n : left->notes)
            {
                if (n.start >= cut)
                {
                    n.start -= cut;
                    moved.push_back(n);
                }
                else
                {
                    n.length = std::min(n.length, cut - n.start);
                    keep.push_back(n);
                }
            }
            left->notes = keep;
            right.notes = moved;
        }
        else
            right.sampleOffset += cut * 60.0 / project.bpm;
        track->data.clips.push_back(right);
    });
}

void Session::copyClip(int clipId)
{
    if (auto* clip = findClip(clipId))
        clipboard = *clip;
}

void Session::pasteClip(int trackId, double beat)
{
    auto track = project.findTrack(trackId);
    if (track == nullptr || ! clipboard)
        return;
    Clip copy = *clipboard;
    copy.id = project.newId();
    copy.start = std::max(0.0, beat);
    edit([&] { track->data.clips.push_back(copy); });
    selectClip(copy.id);
}

//==============================================================================
bool Session::isInstrumentDevice(const Device& device)
{
    if (device.isBuiltin())
        if (auto* info = Builtins::find(device.builtinId))
            return info->isInstrument;
    return device.description.isInstrument || (device.processor != nullptr && device.processor->acceptsMidi()
                                                   && device.processor->getTotalNumInputChannels() == 0);
}

bool Session::addDevice(int trackId, std::unique_ptr<Device> device, bool asInstrument)
{
    auto track = project.findTrack(trackId);
    if (track == nullptr || device == nullptr)
        return false;
    engine.prepareDevice(*device, asInstrument);
    listeners.call([](Listener& l) { l.devicesAboutToChange(); });
    std::unique_ptr<Device> replaced;
    {
        const juce::ScopedLock sl(project.lock);
        if (asInstrument)
        {
            replaced = std::move(track->instrument);
            track->instrument = std::move(device);
        }
        else
            track->effects.push_back(std::move(device));
    }
    if (replaced != nullptr)
        closeEditorWindows(replaced.get());
    replaced.reset();
    project.changed();
    listeners.call([](Listener& l) { l.devicesChanged(); });
    return true;
}

void Session::removeDevice(int trackId, Device* device)
{
    auto track = project.findTrack(trackId);
    if (track == nullptr || device == nullptr)
        return;
    closeEditorWindows(device);
    listeners.call([](Listener& l) { l.devicesAboutToChange(); });
    std::unique_ptr<Device> removed;
    {
        const juce::ScopedLock sl(project.lock);
        if (track->instrument.get() == device)
            removed = std::move(track->instrument);
        for (auto it = track->effects.begin(); it != track->effects.end(); ++it)
            if (it->get() == device)
            {
                removed = std::move(*it);
                track->effects.erase(it);
                break;
            }
    }
    removed.reset();
    project.changed();
    listeners.call([](Listener& l) { l.devicesChanged(); });
}

void Session::showEditorWindow(Device& device)
{
    for (auto& w : editorWindows)
        if (w->device == &device)
        {
            w->toFront(true);
            return;
        }
    listeners.call([](Listener& l) { l.devicesAboutToChange(); }); // an embedded editor may own the active editor
    editorWindows.push_back(std::make_unique<EditorWindow>(device, *this));
    listeners.call([](Listener& l) { l.devicesChanged(); });
}

void Session::closeEditorWindows(Device* device)
{
    const bool any = std::any_of(editorWindows.begin(), editorWindows.end(), [device](auto& w) { return device == nullptr || w->device == device; });
    editorWindows.erase(std::remove_if(editorWindows.begin(), editorWindows.end(),
                                       [device](auto& w) { return device == nullptr || w->device == device; }),
                        editorWindows.end());
    if (any && device != nullptr)
        listeners.call([](Listener& l) { l.devicesChanged(); });
}

std::unique_ptr<Device> Session::createDeviceFromItem(const juce::String& item, juce::String& error)
{
    const auto value = item.fromFirstOccurrenceOf(":", false, false);
    if (item.startsWith("builtin:"))
        return plugins.createBuiltin(value);
    if (item.startsWith("preset:"))
    {
        auto device = plugins.createBuiltin("operator");
        if (auto* op = dynamic_cast<OperatorSynth*>(device->processor.get()))
            op->applyPreset(value);
        return device;
    }
    if (item.startsWith("plugin:"))
    {
        if (auto* description = plugins.findByIdentifier(value))
            return plugins.createPlugin(*description, engine.getSampleRate(), engine.getBlockSize(), error);
        error = "Plugin not found";
        return nullptr;
    }
    if (item.startsWith("file:") && PluginManager::isPluginFile(juce::File(value)))
        return plugins.createFromFile(juce::File(value), engine.getSampleRate(), engine.getBlockSize(), error);
    return nullptr;
}

bool Session::isInterestingFile(const juce::String& path)
{
    const juce::File f(path);
    return AudioSample::isAudioFile(f) || f.hasFileExtension("mid;midi") || PluginManager::isPluginFile(f);
}

bool Session::applyItem(const juce::String& item, int trackId, double beat)
{
    if (item.startsWith("file:"))
        return importFiles({ item.fromFirstOccurrenceOf(":", false, false) }, trackId, beat);

    // Operator preset onto an existing Operator: just load the preset.
    if (item.startsWith("preset:"))
        if (auto track = project.findTrack(trackId); track != nullptr && track->instrument != nullptr)
            if (auto* op = dynamic_cast<OperatorSynth*>(track->instrument->processor.get()))
            {
                op->applyPreset(item.fromFirstOccurrenceOf(":", false, false));
                status("Loaded preset " + op->currentPreset + " on " + track->data.name);
                return true;
            }

    juce::String error;
    auto device = createDeviceFromItem(item, error);
    if (device == nullptr)
    {
        status("Could not load: " + (error.isNotEmpty() ? error : item));
        return false;
    }
    const bool instrument = isInstrumentDevice(*device);
    auto track = project.findTrack(trackId);
    if (track == nullptr)
    {
        auto name = device->displayName();
        if (item.startsWith("preset:"))
            name = item.fromFirstOccurrenceOf(":", false, false);
        track = addTrack(false, -1, name);
    }
    const auto name = device->displayName();
    addDevice(track->id, std::move(device), instrument);
    selectTrack(track->id);
    status((instrument ? "Instrument " : "Effect ") + name + " added to " + track->data.name);
    return true;
}

bool Session::importFiles(const juce::StringArray& files, int trackId, double beat)
{
    bool any = false;
    double position = std::max(0.0, beat);
    for (auto& path : files)
    {
        const juce::File file(path);
        if (PluginManager::isPluginFile(file))
        {
            juce::String error;
            auto device = plugins.createFromFile(file, engine.getSampleRate(), engine.getBlockSize(), error);
            if (device == nullptr)
            {
                status(error);
                continue;
            }
            const bool instrument = isInstrumentDevice(*device);
            auto track = project.findTrack(trackId);
            if (track == nullptr)
                track = addTrack(false, -1, device->displayName());
            const auto name = device->displayName();
            addDevice(track->id, std::move(device), instrument);
            status("Loaded VST3 " + name + " on " + track->data.name);
            any = true;
            continue;
        }

        auto track = project.findTrack(trackId);
        if (AudioSample::isAudioFile(file))
        {
            juce::String error;
            auto sample = AudioSample::load(file, error);
            if (sample == nullptr)
            {
                status(error);
                continue;
            }
            if (track == nullptr)
                track = addTrack(false, -1, file.getFileNameWithoutExtension());
            auto clip = io::makeAudioClip(project, sample, position, project.bpm, track->data.colour);
            edit([&] { track->data.clips.push_back(clip); });
            position = clip.end();
            selectClip(clip.id);
            status("Imported " + file.getFileName());
            any = true;
        }
        else if (file.hasFileExtension("mid;midi"))
        {
            Clip clip;
            juce::String error;
            if (! io::importMidiFile(file, clip, error))
            {
                status(error);
                continue;
            }
            if (track == nullptr)
                track = addTrack(true, -1, file.getFileNameWithoutExtension());
            clip.id = project.newId();
            clip.kind = ClipKind::midi;
            clip.colour = track->data.colour;
            clip.start = position;
            edit([&] { track->data.clips.push_back(clip); });
            position = clip.end();
            selectClip(clip.id);
            status("Imported " + file.getFileName());
            any = true;
        }
    }
    return any;
}

//==============================================================================
void Session::toggleRecord()
{
    const bool next = ! engine.isRecording();
    engine.setRecording(next);
    if (next && ! engine.isPlaying())
        engine.play();
    sendChangeMessage();
}

void Session::updateRecording()
{
    recordedEvents.clear();
    engine.popRecordedEvents(recordedEvents);
    const bool isRecording = engine.isRecording() && engine.isPlaying();
    const double now = engine.getPlayhead();

    for (auto& e : recordedEvents)
    {
        auto track = project.findTrack(e.trackId);
        if (track == nullptr)
            continue;
        auto& rec = recordings[e.trackId];
        if (rec.clipId == 0 || track->findClip(rec.clipId) == nullptr)
        {
            const double start = std::floor(e.beat / project.beatsPerBar) * project.beatsPerBar;
            auto clip = io::makeMidiClip(project, start, project.beatsPerBar, track->data.colour, "Recording");
            rec.clipId = clip.id;
            edit([&] { track->data.clips.push_back(clip); });
        }
        if (e.noteOn)
            rec.open[e.pitch] = { e.beat, e.velocity };
        else if (auto it = rec.open.find(e.pitch); it != rec.open.end())
        {
            auto* clip = track->findClip(rec.clipId);
            const Note note { e.pitch, it->second.start - clip->start, std::max(1.0 / 32.0, e.beat - it->second.start), it->second.velocity };
            rec.open.erase(it);
            if (note.start >= 0)
                edit([&] { clip->notes.push_back(note); }, false);
        }
    }

    // Grow clips while recording; close hanging notes when recording stops.
    for (auto it = recordings.begin(); it != recordings.end();)
    {
        auto track = project.findTrack(it->first);
        auto* clip = track != nullptr ? track->findClip(it->second.clipId) : nullptr;
        if (clip == nullptr)
        {
            it = recordings.erase(it);
            continue;
        }
        const double bar = project.beatsPerBar;
        const double wanted = std::max(clip->length, std::ceil((now - clip->start) / bar) * bar);
        if (isRecording && now > clip->start && wanted != clip->length)
            edit([&] { clip->length = wanted; }, false);
        if (! isRecording)
        {
            for (auto& [pitch, open] : it->second.open)
                if (open.start - clip->start >= 0)
                    clip->notes.push_back({ pitch, open.start - clip->start, std::max(0.125, now - open.start), open.velocity });
            project.changed();
            it = recordings.erase(it);
            continue;
        }
        ++it;
    }
    if (wasRecording != isRecording)
        sendChangeMessage();
    wasRecording = isRecording;
}

} // namespace az

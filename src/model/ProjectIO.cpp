#include "ProjectIO.h"
#include "../engine/AudioEngine.h"
#include "../plugins/PluginManager.h"
#include "../dsp/OperatorSynth.h"

namespace az::io
{

static constexpr int formatVersion = 1;

std::shared_ptr<Track> makeTrack(Project& idSource, const juce::String& name, juce::Colour colour)
{
    auto track = std::make_shared<Track>();
    track->id = idSource.newId();
    track->data.name = name;
    track->data.colour = colour;
    return track;
}

Clip makeMidiClip(Project& idSource, double start, double length, juce::Colour colour, const juce::String& name)
{
    Clip clip;
    clip.id = idSource.newId();
    clip.kind = ClipKind::midi;
    clip.name = name;
    clip.colour = colour;
    clip.start = start;
    clip.length = length;
    return clip;
}

Clip makeAudioClip(Project& idSource, std::shared_ptr<const AudioSample> sample, double start, double bpm, juce::Colour colour)
{
    Clip clip;
    clip.id = idSource.newId();
    clip.kind = ClipKind::audio;
    clip.name = sample->name;
    clip.colour = colour;
    clip.start = start;
    clip.length = std::max(0.25, sample->lengthSeconds() * bpm / 60.0);
    clip.sample = std::move(sample);
    return clip;
}

bool importMidiFile(const juce::File& file, Clip& clip, juce::String& error)
{
    juce::FileInputStream in(file);
    juce::MidiFile midi;
    if (! in.openedOk() || ! midi.readFrom(in))
    {
        error = "Could not read MIDI file " + file.getFileName();
        return false;
    }
    const int tpq = midi.getTimeFormat();
    if (tpq <= 0)
    {
        error = "SMPTE-timed MIDI files are not supported yet";
        return false;
    }
    clip.notes.clear();
    double end = 0.0;
    for (int t = 0; t < midi.getNumTracks(); ++t)
    {
        juce::MidiMessageSequence seq(*midi.getTrack(t));
        seq.updateMatchedPairs();
        for (auto* event : seq)
        {
            if (! event->message.isNoteOn())
                continue;
            Note note;
            note.pitch = event->message.getNoteNumber();
            note.velocity = event->message.getFloatVelocity();
            note.start = event->message.getTimeStamp() / tpq;
            const double off = event->noteOffObject != nullptr ? event->noteOffObject->message.getTimeStamp() / tpq : note.start + 0.25;
            note.length = std::max(1.0 / 64.0, off - note.start);
            end = std::max(end, note.start + note.length);
            clip.notes.push_back(note);
        }
    }
    if (clip.notes.empty())
    {
        error = "MIDI file has no notes: " + file.getFileName();
        return false;
    }
    clip.name = file.getFileNameWithoutExtension();
    clip.length = std::max(4.0, std::ceil(end / 4.0) * 4.0);
    return true;
}

//==============================================================================
static juce::XmlElement* writeDevice(juce::XmlElement& parent, const char* tag, Device& device)
{
    auto* e = parent.createNewChildElement(tag);
    e->setAttribute("bypass", device.bypassed);
    if (device.isBuiltin())
        e->setAttribute("builtin", device.builtinId);
    else
        e->addChildElement(device.description.createXml().release());
    if (device.processor != nullptr)
    {
        juce::MemoryBlock state;
        device.processor->getStateInformation(state);
        e->setAttribute("state", state.toBase64Encoding());
    }
    return e;
}

bool save(Project& project, const juce::File& file, juce::String& error)
{
    juce::XmlElement root("AudioZeroProject");
    root.setAttribute("version", formatVersion);
    root.setAttribute("bpm", project.bpm);
    root.setAttribute("beatsPerBar", project.beatsPerBar);
    root.setAttribute("loopEnabled", project.loopEnabled);
    root.setAttribute("loopStart", project.loopStart);
    root.setAttribute("loopEnd", project.loopEnd);
    root.setAttribute("masterVolume", project.masterVolume);

    const auto samplesFolder = file.getParentDirectory().getChildFile(file.getFileNameWithoutExtension() + " Samples");
    std::map<const AudioSample*, juce::File> written;

    for (auto& track : project.tracks)
    {
        auto* t = root.createNewChildElement("Track");
        const auto& d = track->data;
        t->setAttribute("name", d.name);
        t->setAttribute("colour", d.colour.toString());
        t->setAttribute("volume", d.volume);
        t->setAttribute("pan", d.pan);
        t->setAttribute("mute", d.mute);
        t->setAttribute("solo", d.solo);
        if (track->instrument != nullptr)
            writeDevice(*t, "Instrument", *track->instrument);
        for (auto& fx : track->effects)
            writeDevice(*t, "Effect", *fx);

        for (auto& clip : d.clips)
        {
            auto* c = t->createNewChildElement("Clip");
            c->setAttribute("kind", clip.kind == ClipKind::midi ? "midi" : "audio");
            c->setAttribute("name", clip.name);
            c->setAttribute("colour", clip.colour.toString());
            c->setAttribute("start", clip.start);
            c->setAttribute("length", clip.length);
            c->setAttribute("muted", clip.muted);
            if (clip.kind == ClipKind::midi)
            {
                for (auto& n : clip.notes)
                {
                    auto* ne = c->createNewChildElement("Note");
                    ne->setAttribute("p", n.pitch);
                    ne->setAttribute("s", n.start);
                    ne->setAttribute("l", n.length);
                    ne->setAttribute("v", n.velocity);
                }
            }
            else if (clip.sample != nullptr)
            {
                auto sampleFile = clip.sample->file;
                if (sampleFile == juce::File())
                {
                    auto& target = written[clip.sample.get()];
                    if (target == juce::File())
                    {
                        samplesFolder.createDirectory();
                        target = samplesFolder.getNonexistentChildFile(juce::File::createLegalFileName(clip.sample->name), ".wav");
                        juce::WavAudioFormat wav;
                        std::unique_ptr<juce::OutputStream> out = target.createOutputStream();
                        std::unique_ptr<juce::AudioFormatWriter> writer;
                        if (out != nullptr)
                            writer = wav.createWriterFor(out, juce::AudioFormatWriterOptions {}
                                                                  .withSampleRate(clip.sample->sampleRate)
                                                                  .withNumChannels(clip.sample->data.getNumChannels())
                                                                  .withBitsPerSample(24));
                        if (writer == nullptr)
                        {
                            error = "Could not write sample " + target.getFullPathName();
                            return false;
                        }
                        writer->writeFromAudioSampleBuffer(clip.sample->data, 0, clip.sample->data.getNumSamples());
                    }
                    sampleFile = target;
                }
                c->setAttribute("file", sampleFile.getFullPathName());
                c->setAttribute("relativeFile", sampleFile.getRelativePathFrom(file.getParentDirectory()));
                c->setAttribute("offset", clip.sampleOffset);
                c->setAttribute("gain", clip.gain);
            }
        }
    }

    if (! root.writeTo(file))
    {
        error = "Could not write " + file.getFullPathName();
        return false;
    }
    return true;
}

//==============================================================================
static std::unique_ptr<Device> readDevice(const juce::XmlElement& e, PluginManager& plugins, AudioEngine& engine, juce::StringArray& warnings)
{
    std::unique_ptr<Device> device;
    if (e.hasAttribute("builtin"))
        device = plugins.createBuiltin(e.getStringAttribute("builtin"));
    else if (auto* pe = e.getChildByName("PLUGIN"))
    {
        juce::PluginDescription description;
        description.loadFromXml(*pe);
        juce::String error;
        device = plugins.createPlugin(description, engine.getSampleRate(), engine.getBlockSize(), error);
        if (device == nullptr)
            warnings.add("Missing plugin \"" + description.name + "\": " + error);
    }
    if (device == nullptr)
        return nullptr;
    device->bypassed = e.getBoolAttribute("bypass");
    juce::MemoryBlock state;
    if (state.fromBase64Encoding(e.getStringAttribute("state")) && state.getSize() > 0)
        device->processor->setStateInformation(state.getData(), static_cast<int>(state.getSize()));
    return device;
}

bool load(const juce::File& file, Project& idSource, PluginManager& plugins, AudioEngine& engine,
          Loaded& result, juce::String& error, juce::StringArray& warnings)
{
    auto root = juce::XmlDocument::parse(file);
    if (root == nullptr || ! root->hasTagName("AudioZeroProject"))
    {
        error = file.getFileName() + " is not an AudioZero project";
        return false;
    }
    result.bpm = juce::jlimit(20.0, 999.0, root->getDoubleAttribute("bpm", 120.0));
    result.beatsPerBar = juce::jlimit(1, 16, root->getIntAttribute("beatsPerBar", 4));
    result.loopEnabled = root->getBoolAttribute("loopEnabled", true);
    result.loopStart = root->getDoubleAttribute("loopStart", 0.0);
    result.loopEnd = root->getDoubleAttribute("loopEnd", 16.0);
    result.masterVolume = static_cast<float>(root->getDoubleAttribute("masterVolume", 0.85));

    for (auto* t : root->getChildWithTagNameIterator("Track"))
    {
        auto track = makeTrack(idSource, t->getStringAttribute("name", "Track"),
                               juce::Colour::fromString(t->getStringAttribute("colour", "ffff8a3d")));
        auto& d = track->data;
        d.volume = static_cast<float>(t->getDoubleAttribute("volume", 0.8));
        d.pan = static_cast<float>(t->getDoubleAttribute("pan", 0.0));
        d.mute = t->getBoolAttribute("mute");
        d.solo = t->getBoolAttribute("solo");
        if (auto* ie = t->getChildByName("Instrument"))
            track->instrument = readDevice(*ie, plugins, engine, warnings);
        for (auto* fe : t->getChildWithTagNameIterator("Effect"))
            if (auto fx = readDevice(*fe, plugins, engine, warnings))
                track->effects.push_back(std::move(fx));

        for (auto* c : t->getChildWithTagNameIterator("Clip"))
        {
            Clip clip;
            clip.id = idSource.newId();
            clip.kind = c->getStringAttribute("kind") == "audio" ? ClipKind::audio : ClipKind::midi;
            clip.name = c->getStringAttribute("name");
            clip.colour = juce::Colour::fromString(c->getStringAttribute("colour", d.colour.toString()));
            clip.start = std::max(0.0, c->getDoubleAttribute("start"));
            clip.length = std::max(0.0625, c->getDoubleAttribute("length", 4.0));
            clip.muted = c->getBoolAttribute("muted");
            if (clip.kind == ClipKind::midi)
            {
                for (auto* n : c->getChildWithTagNameIterator("Note"))
                    clip.notes.push_back({ juce::jlimit(0, 127, n->getIntAttribute("p", 60)), n->getDoubleAttribute("s"),
                                           std::max(1.0 / 128.0, n->getDoubleAttribute("l", 1.0)), static_cast<float>(n->getDoubleAttribute("v", 0.8)) });
            }
            else
            {
                auto sampleFile = juce::File(c->getStringAttribute("file"));
                if (! sampleFile.existsAsFile())
                    sampleFile = file.getParentDirectory().getChildFile(c->getStringAttribute("relativeFile"));
                juce::String sampleError;
                clip.sample = AudioSample::load(sampleFile, sampleError);
                if (clip.sample == nullptr)
                {
                    warnings.add("Missing sample " + c->getStringAttribute("file"));
                    continue;
                }
                clip.sampleOffset = c->getDoubleAttribute("offset");
                clip.gain = static_cast<float>(c->getDoubleAttribute("gain", 1.0));
            }
            d.clips.push_back(std::move(clip));
        }
        engine.prepareTrack(*track);
        result.tracks.push_back(std::move(track));
    }
    return true;
}

void install(Project& project, Loaded&& loaded, const juce::File& file)
{
    std::vector<std::shared_ptr<Track>> old;
    {
        const juce::ScopedLock sl(project.lock);
        old.swap(project.tracks);
        project.tracks = std::move(loaded.tracks);
        project.bpm = loaded.bpm;
        project.beatsPerBar = loaded.beatsPerBar;
        project.loopEnabled = loaded.loopEnabled;
        project.loopStart = loaded.loopStart;
        project.loopEnd = loaded.loopEnd;
        project.masterVolume = loaded.masterVolume;
    }
    project.clearHistory();
    project.file = file;
    project.changed();
    project.savedVersion = project.version();
    old.clear(); // destroy previous devices outside the lock
}

//==============================================================================
Loaded createEmpty(Project& idSource, PluginManager& plugins, AudioEngine& engine)
{
    Loaded result;
    auto synth = makeTrack(idSource, "Operator", palette::trackColour(0));
    synth->instrument = plugins.createBuiltin("operator");
    engine.prepareTrack(*synth);
    auto audio = makeTrack(idSource, "Audio", palette::trackColour(1));
    engine.prepareTrack(*audio);
    result.tracks = { synth, audio };
    return result;
}

static std::shared_ptr<const AudioSample> makePercLoop(double bpm, int bars)
{
    // A small synthesized hat/clap/shaker loop so the demo has an audio clip with a waveform.
    const double sr = 44100.0;
    const double beatSeconds = 60.0 / bpm;
    const int frames = static_cast<int>(bars * 4 * beatSeconds * sr);
    juce::AudioBuffer<float> buffer(2, frames);
    buffer.clear();
    juce::Random random(1234);
    auto hit = [&](double beat, double decay, float gain, float hp, float pan, bool clap) {
        const int start = static_cast<int>(beat * beatSeconds * sr);
        float lp = 0.0f;
        for (int i = 0; i < static_cast<int>(sr * decay * 5.0) && start + i < frames; ++i)
        {
            const double t = i / sr;
            float env = static_cast<float>(std::exp(-t / decay));
            if (clap)
                env *= (t < 0.03 ? 0.6f + 0.4f * static_cast<float>(std::sin(t * 2.0 * juce::MathConstants<double>::pi * 100.0)) : 1.0f);
            const float noise = random.nextFloat() * 2.0f - 1.0f;
            lp += hp * (noise - lp);
            const float v = (noise - lp) * env * gain;
            buffer.addSample(0, start + i, v * (1.0f - pan));
            buffer.addSample(1, start + i, v * (1.0f + pan));
        }
    };
    for (int bar = 0; bar < bars; ++bar)
    {
        const double b = bar * 4.0;
        for (int eighth = 0; eighth < 8; ++eighth)
            hit(b + eighth * 0.5 + 0.0, eighth % 2 == 1 ? 0.045 : 0.02, eighth % 2 == 1 ? 0.35f : 0.18f, 0.6f, 0.2f, false);
        for (int sixteenth = 0; sixteenth < 16; ++sixteenth)
            hit(b + sixteenth * 0.25 + 0.02, 0.012, sixteenth % 4 == 2 ? 0.16f : 0.08f, 0.8f, -0.4f, false);
        hit(b + 1.0, 0.09, 0.55f, 0.25f, 0.0f, true);
        hit(b + 3.0, 0.09, 0.55f, 0.25f, 0.0f, true);
    }
    return AudioSample::fromBuffer("Perc Loop", std::move(buffer), sr);
}

Loaded createDemo(Project& idSource, PluginManager& plugins, AudioEngine& engine)
{
    Loaded result;
    result.bpm = 124.0;
    result.loopStart = 32.0;
    result.loopEnd = 48.0;
    result.masterVolume = 0.75f;

    auto addTrack = [&](const juce::String& name, int colour, const juce::String& preset) {
        auto track = makeTrack(idSource, name, palette::trackColour(colour));
        if (preset.isNotEmpty())
        {
            track->instrument = plugins.createBuiltin("operator");
            if (auto* op = dynamic_cast<OperatorSynth*>(track->instrument->processor.get()))
                op->applyPreset(preset);
        }
        result.tracks.push_back(track);
        return track;
    };
    auto clip = [&](Track& track, double start, double length, const juce::String& name) -> Clip& {
        track.data.clips.push_back(makeMidiClip(idSource, start, length, track.data.colour, name));
        return track.data.clips.back();
    };

    // Kick: four on the floor.
    auto kick = addTrack("Kick", 0, "Kick");
    kick->data.volume = 0.7f;
    for (double start : { 0.0, 32.0 })
    {
        auto& c = clip(*kick, start, 32.0, "Kick");
        for (int beat = 0; beat < 32; ++beat)
            c.notes.push_back({ 36, static_cast<double>(beat), 0.5, 0.9f });
    }

    // Percussion: generated audio loop.
    auto perc = addTrack("Perc Loop", 1, {});
    auto loop = makePercLoop(result.bpm, 4);
    for (double start : { 16.0, 32.0, 48.0 })
        perc->data.clips.push_back(makeAudioClip(idSource, loop, start, result.bpm, perc->data.colour));
    perc->data.volume = 0.7f;

    // Chord progression Am - F - C - G, two bars each.
    const std::vector<std::vector<int>> chords = { { 57, 60, 64 }, { 53, 57, 60 }, { 55, 60, 64 }, { 55, 59, 62 } };
    const int roots[] = { 33, 29, 36, 31 };

    auto bass = addTrack("Offbeat Bass", 2, "FM Bass");
    bass->data.volume = 0.75f;
    for (double start : { 16.0, 32.0, 48.0 })
    {
        auto& c = clip(*bass, start, 16.0, "Offbeat Bass");
        for (int bar = 0; bar < 4; ++bar)
            for (int i = 0; i < 4; ++i)
            {
                const int songBar = static_cast<int>(start / 4.0) + bar;
                const int root = roots[(songBar / 2) % 4];
                c.notes.push_back({ i == 3 ? root + 12 : root, bar * 4.0 + i + 0.5, 0.4, i == 0 ? 0.95f : 0.8f });
            }
    }

    auto keys = addTrack("Chords", 3, "Electric Piano");
    keys->effects.push_back(plugins.createBuiltin("reverb"));
    for (double start : { 0.0, 32.0 })
    {
        auto& c = clip(*keys, start, 32.0, "Chords");
        for (int bar = 0; bar < 8; ++bar)
        {
            const auto& chord = chords[static_cast<size_t>((bar / 2) % 4)];
            for (auto [pos, len, vel] : { std::tuple { 0.0, 1.4, 0.85f }, std::tuple { 1.5, 0.45, 0.6f }, std::tuple { 2.5, 1.2, 0.75f } })
                for (int pitch : chord)
                    c.notes.push_back({ pitch, bar * 4.0 + pos, len, vel });
        }
    }

    auto pad = addTrack("Pad", 6, "Warm Pad");
    pad->data.volume = 0.6f;
    for (double start : { 0.0, 48.0 })
    {
        auto& c = clip(*pad, start, 16.0, "Pad");
        for (int k = 0; k < 2; ++k)
            for (int pitch : chords[static_cast<size_t>((static_cast<int>(start / 8.0) + k) % 4)])
                c.notes.push_back({ pitch - 12, k * 8.0, 7.9, 0.6f });
    }

    auto bell = addTrack("Bell Hook", 4, "Glass Bell");
    bell->effects.push_back(plugins.createBuiltin("delay"));
    {
        auto& c = clip(*bell, 32.0, 32.0, "Bell Hook");
        const std::vector<std::pair<double, int>> phrase = {
            { 0.0, 76 }, { 0.75, 79 }, { 1.5, 81 }, { 2.5, 79 }, { 3.0, 76 },
            { 4.5, 72 }, { 5.0, 74 }, { 5.75, 76 }, { 6.5, 74 },
        };
        for (int rep = 0; rep < 4; ++rep)
            for (auto [pos, pitch] : phrase)
                c.notes.push_back({ pitch + (rep == 3 && pos > 4.0 ? 5 : 0), rep * 8.0 + pos, 0.6, 0.8f });
    }

    for (auto& track : result.tracks)
        engine.prepareTrack(*track);
    return result;
}

} // namespace az::io

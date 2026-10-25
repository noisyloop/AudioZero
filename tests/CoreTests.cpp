// Headless tests for the AudioZero engine, Operator synth and project files.
#include "../src/engine/AudioEngine.h"
#include "../src/model/ProjectIO.h"
#include "../src/plugins/PluginManager.h"
#include "../src/dsp/OperatorSynth.h"
#include "../src/dsp/BuiltinEffects.h"
#include <iostream>

namespace
{
int failures = 0;

void check(bool condition, const char* what)
{
    std::cout << (condition ? "  ok   " : "  FAIL ") << what << "\n";
    if (! condition)
        ++failures;
}

float peak(const juce::AudioBuffer<float>& b)
{
    float p = 0.0f;
    for (int ch = 0; ch < b.getNumChannels(); ++ch)
        p = std::max(p, b.getMagnitude(ch, 0, b.getNumSamples()));
    return p;
}

bool finite(const juce::AudioBuffer<float>& b)
{
    for (int ch = 0; ch < b.getNumChannels(); ++ch)
        for (int i = 0; i < b.getNumSamples(); ++i)
            if (! std::isfinite(b.getSample(ch, i)))
                return false;
    return true;
}

float renderNote(az::OperatorSynth& synth, int note, int blocks, bool release)
{
    synth.prepareToPlay(48000.0, 512);
    juce::AudioBuffer<float> buffer(2, 512);
    float p = 0.0f;
    for (int i = 0; i < blocks; ++i)
    {
        juce::MidiBuffer midi;
        if (i == 0)
            midi.addEvent(juce::MidiMessage::noteOn(1, note, 0.9f), 0);
        if (release && i == blocks / 2)
            midi.addEvent(juce::MidiMessage::noteOff(1, note), 0);
        synth.processBlock(buffer, midi);
        if (! finite(buffer))
            return -1.0f;
        p = std::max(p, peak(buffer));
    }
    return p;
}

void testOperator()
{
    std::cout << "Operator synth\n";
    az::OperatorSynth synth;
    check(renderNote(synth, 60, 20, false) > 0.05f, "init patch produces audio");

    bool allAlgorithms = true;
    for (int a = 0; a < az::OperatorSynth::numAlgorithms; ++a)
    {
        synth.applyPreset("Init");
        auto* alg = synth.params.getParameter("algorithm");
        alg->setValueNotifyingHost(alg->convertTo0to1(static_cast<float>(a)));
        for (const char* op : { "opBLevel", "opCLevel", "opDLevel" })
            synth.params.getParameter(op)->setValueNotifyingHost(0.7f);
        const float p = renderNote(synth, 57, 10, false);
        allAlgorithms = allAlgorithms && p > 0.02f && p < 4.0f;
    }
    check(allAlgorithms, "all 8 algorithms produce bounded, finite audio");

    bool allPresets = true;
    for (auto& name : az::OperatorSynth::presetNames())
    {
        synth.applyPreset(name);
        const float p = renderNote(synth, name == "Kick" ? 36 : 60, 16, false);
        if (! (p > 0.01f && p < 4.0f))
        {
            std::cout << "       preset " << name << " peak " << p << "\n";
            allPresets = false;
        }
    }
    check(allPresets, "every preset produces audio");

    synth.applyPreset("Pluck");
    synth.prepareToPlay(48000.0, 512);
    juce::AudioBuffer<float> buffer(2, 512);
    juce::MidiBuffer on;
    on.addEvent(juce::MidiMessage::noteOn(1, 60, 0.9f), 0);
    synth.processBlock(buffer, on);
    juce::MidiBuffer off;
    off.addEvent(juce::MidiMessage::noteOff(1, 60), 0);
    synth.processBlock(buffer, off);
    for (int i = 0; i < 400; ++i)
    {
        juce::MidiBuffer none;
        synth.processBlock(buffer, none);
    }
    check(synth.activeVoiceCount() == 0 && peak(buffer) < 1.0e-4f, "voices finish after release");

    synth.applyPreset("Glass Bell");
    juce::MemoryBlock state;
    synth.getStateInformation(state);
    az::OperatorSynth other;
    other.setStateInformation(state.getData(), static_cast<int>(state.getSize()));
    check(other.currentPreset == "Glass Bell"
              && std::abs(other.params.getRawParameterValue("opBFine")->load() - 500.0f) < 0.5f,
          "state save/restore");
}

std::shared_ptr<az::Track> addOperatorTrack(az::Project& project, az::PluginManager& plugins, az::AudioEngine& engine)
{
    auto track = az::io::makeTrack(project, "Synth", az::palette::trackColour(0));
    track->instrument = plugins.createBuiltin("operator");
    auto clip = az::io::makeMidiClip(project, 0.0, 4.0, track->data.colour);
    clip.notes.push_back({ 60, 0.0, 2.0, 0.9f });
    track->data.clips.push_back(clip);
    engine.prepareTrack(*track);
    project.tracks.push_back(track);
    return track;
}

void testEngine()
{
    std::cout << "Audio engine\n";
    az::Project project;
    az::PluginManager plugins;
    az::AudioEngine engine(project);
    engine.prepare(48000.0, 256);
    project.loopEnabled = false;
    auto track = addOperatorTrack(project, plugins, engine);

    juce::AudioBuffer<float> out(2, 48000);
    engine.render(out, out.getNumSamples());
    check(peak(out) < 1.0e-6f, "silent while stopped");

    engine.play();
    engine.render(out, out.getNumSamples());
    check(peak(out) > 0.02f && finite(out), "MIDI clip plays through Operator");
    check(std::abs(engine.getPlayhead() - 2.0) < 0.01, "transport advanced one second (2 beats at 120 BPM)");

    track->data.mute = true;
    engine.setPlayhead(0.0);
    engine.render(out, out.getNumSamples());
    check(peak(out) < 1.0e-6f, "muted track is silent");
    track->data.mute = false;

    auto other = az::io::makeTrack(project, "Other", az::palette::trackColour(1));
    engine.prepareTrack(*other);
    other->data.solo = true;
    project.tracks.push_back(other);
    engine.setPlayhead(0.0);
    engine.render(out, out.getNumSamples());
    check(peak(out) < 1.0e-6f, "solo on another track silences this one");
    other->data.solo = false;

    // Audio clip
    juce::AudioBuffer<float> tone(1, 48000);
    for (int i = 0; i < tone.getNumSamples(); ++i)
        tone.setSample(0, i, 0.5f * std::sin(i * 0.05f));
    auto sample = az::AudioSample::fromBuffer("Tone", std::move(tone), 48000.0);
    other->data.clips.push_back(az::io::makeAudioClip(project, sample, 0.0, project.bpm, other->data.colour));
    track->data.mute = true;
    engine.setPlayhead(0.0);
    engine.render(out, out.getNumSamples());
    check(std::abs(peak(out) - 0.5f * project.masterVolume * other->data.volume) < 0.05f, "audio clip plays at unity gain");
    check(std::abs(other->data.clips.back().length - 2.0) < 1e-6, "audio clip length follows tempo");

    // Loop
    project.loopEnabled = true;
    project.loopStart = 0.0;
    project.loopEnd = 1.0;
    engine.setPlayhead(0.0);
    bool inRange = true;
    juce::AudioBuffer<float> small(2, 300);
    for (int i = 0; i < 400; ++i)
    {
        engine.render(small, small.getNumSamples());
        inRange = inRange && engine.getPlayhead() >= 0.0 && engine.getPlayhead() < 1.0;
    }
    check(inRange, "playhead stays inside the loop");
    engine.stop();
}

void testProjectFiles()
{
    std::cout << "Project files and undo\n";
    az::Project project;
    az::PluginManager plugins;
    az::AudioEngine engine(project);
    engine.prepare(44100.0, 512);
    az::io::install(project, az::io::createDemo(project, plugins, engine), {});
    check(project.tracks.size() >= 5, "demo project created");
    {
        // Render the busiest part of the demo and check the mix level.
        engine.setPlayhead(32.0);
        engine.play();
        juce::AudioBuffer<float> mix(2, 44100 * 8);
        engine.render(mix, mix.getNumSamples());
        engine.stop();
        std::cout << "       demo peak " << peak(mix) << "\n";
        check(finite(mix) && peak(mix) > 0.2f && peak(mix) < 0.95f, "demo song mix is loud but not clipping");
    }

    auto dir = juce::File::getSpecialLocation(juce::File::tempDirectory).getChildFile("AudioZeroTest");
    dir.deleteRecursively();
    dir.createDirectory();
    auto file = dir.getChildFile("Demo.azproj");
    juce::String error;
    check(az::io::save(project, file, error), "save project");
    check(dir.getChildFile("Demo Samples").getNumberOfChildFiles(juce::File::findFiles) == 1, "generated audio written next to project");

    az::Project loadedProject;
    az::io::Loaded loaded;
    juce::StringArray warnings;
    check(az::io::load(file, loadedProject, plugins, engine, loaded, error, warnings) && warnings.isEmpty(), "load project");
    check(loaded.tracks.size() == project.tracks.size() && std::abs(loaded.bpm - 124.0) < 1e-6, "tracks and tempo round trip");
    size_t notesBefore = 0, notesAfter = 0;
    for (auto& t : project.tracks)
        for (auto& c : t->data.clips)
            notesBefore += c.notes.size();
    for (auto& t : loaded.tracks)
        for (auto& c : t->data.clips)
            notesAfter += c.notes.size();
    check(notesBefore > 100 && notesBefore == notesAfter, "notes round trip");
    auto* op = dynamic_cast<az::OperatorSynth*>(loaded.tracks[0]->instrument->processor.get());
    check(op != nullptr && op->currentPreset == "Kick", "instrument state round trips");
    check(loaded.tracks[3]->effects.size() == 1 && loaded.tracks[3]->effects[0]->builtinId == "reverb", "effects round trip");

    const auto clipsBefore = project.tracks[0]->data.clips.size();
    project.checkpoint();
    project.tracks[0]->data.clips.push_back(az::io::makeMidiClip(project, 80.0, 4.0, juce::Colours::red));
    project.changed();
    project.undo();
    check(project.tracks[0]->data.clips.size() == clipsBefore, "undo removes the clip");
    project.redo();
    check(project.tracks[0]->data.clips.size() == clipsBefore + 1, "redo restores it");

    auto removed = project.tracks.back();
    project.checkpoint();
    project.tracks.pop_back();
    project.undo();
    check(project.tracks.back() == removed && removed->instrument != nullptr, "undo brings back a deleted track with its instrument");

    juce::MidiMessageSequence seq;
    seq.addEvent(juce::MidiMessage::noteOn(1, 64, 0.8f), 0);
    seq.addEvent(juce::MidiMessage::noteOff(1, 64), 480);
    seq.addEvent(juce::MidiMessage::noteOn(1, 67, 0.8f), 960);
    seq.addEvent(juce::MidiMessage::noteOff(1, 67), 1440);
    juce::MidiFile midi;
    midi.setTicksPerQuarterNote(480);
    midi.addTrack(seq);
    auto midiFile = dir.getChildFile("riff.mid");
    {
        juce::FileOutputStream out(midiFile);
        midi.writeTo(out);
    }
    az::Clip clip;
    check(az::io::importMidiFile(midiFile, clip, error) && clip.notes.size() == 2
              && std::abs(clip.notes[1].start - 2.0) < 1e-6 && std::abs(clip.notes[1].length - 1.0) < 1e-6,
          "MIDI file import");

    // Offline export
    project.loopEnabled = true;
    auto wav = dir.getChildFile("export.wav");
    check(engine.exportWav(wav, 8.0, error) && wav.getSize() > 100000, "WAV export");
    dir.deleteRecursively();
}

void testEffects()
{
    std::cout << "Built-in effects\n";
    for (auto id : { "reverb", "delay" })
    {
        auto fx = az::Builtins::create(id);
        fx->prepareToPlay(48000.0, 512);
        juce::AudioBuffer<float> buffer(2, 512);
        buffer.clear();
        buffer.setSample(0, 0, 1.0f);
        buffer.setSample(1, 0, 1.0f);
        juce::MidiBuffer midi;
        float tail = 0.0f;
        for (int i = 0; i < 200; ++i)
        {
            fx->processBlock(buffer, midi);
            if (i > 0)
                tail = std::max(tail, peak(buffer));
            buffer.clear();
        }
        check(tail > 1.0e-4f && finite(buffer), (juce::String(id) + " produces a tail").toRawUTF8());
    }
}
void testVst3Hosting(const juce::File& bundle)
{
    std::cout << "VST3 hosting (" << bundle.getFileName() << ")\n";
    az::Project project;
    az::PluginManager plugins;
    az::AudioEngine engine(project);
    engine.prepare(48000.0, 256);
    project.loopEnabled = false;

    juce::String error;
    auto device = plugins.createFromFile(bundle, 48000.0, 256, error);
    check(device != nullptr, ("load VST3 through the AudioZero host " + error).toRawUTF8());
    if (device == nullptr)
        return;
    check(device->description.isInstrument && device->processor->acceptsMidi(), "plugin reports itself as an instrument");

    auto track = az::io::makeTrack(project, "Plugin", az::palette::trackColour(0));
    track->instrument = std::move(device);
    auto clip = az::io::makeMidiClip(project, 0.0, 4.0, track->data.colour);
    clip.notes.push_back({ 60, 0.0, 2.0, 0.9f });
    track->data.clips.push_back(clip);
    engine.prepareTrack(*track);
    project.tracks.push_back(track);

    engine.play();
    juce::AudioBuffer<float> out(2, 48000);
    engine.render(out, out.getNumSamples());
    engine.stop();
    check(peak(out) > 0.02f && finite(out), "VST3 instrument plays the MIDI clip");

    auto dir = juce::File::getSpecialLocation(juce::File::tempDirectory).getChildFile("AudioZeroVst3Test");
    dir.createDirectory();
    auto file = dir.getChildFile("plugin.azproj");
    check(az::io::save(project, file, error), "save project with a VST3 instrument");
    az::Project other;
    az::io::Loaded loaded;
    juce::StringArray warnings;
    check(az::io::load(file, other, plugins, engine, loaded, error, warnings) && warnings.isEmpty()
              && loaded.tracks.size() == 1 && loaded.tracks[0]->instrument != nullptr
              && ! loaded.tracks[0]->instrument->isBuiltin(),
          "VST3 instrument reloads from the project file");
    dir.deleteRecursively();
}
} // namespace

int main(int argc, char** argv)
{
    juce::ScopedJuceInitialiser_GUI init;
    if (argc > 2 && juce::String(argv[1]) == "--vst3")
    {
        testVst3Hosting(juce::File(juce::String::fromUTF8(argv[2])));
        std::cout << (failures == 0 ? "ALL TESTS PASSED\n" : "TESTS FAILED\n");
        return failures == 0 ? 0 : 1;
    }
    testOperator();
    testEffects();
    testEngine();
    testProjectFiles();
    std::cout << (failures == 0 ? "ALL TESTS PASSED\n" : "TESTS FAILED\n");
    return failures == 0 ? 0 : 1;
}

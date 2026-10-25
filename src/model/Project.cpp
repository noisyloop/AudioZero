#include "Project.h"

namespace az
{

//==============================================================================
void AudioSample::computePeaks()
{
    const int frames = data.getNumSamples();
    peaks.assign(static_cast<size_t>((frames + peakBlock - 1) / peakBlock), 0.0f);
    for (int ch = 0; ch < data.getNumChannels(); ++ch)
    {
        const float* samples = data.getReadPointer(ch);
        for (int i = 0; i < frames; ++i)
        {
            auto& peak = peaks[static_cast<size_t>(i / peakBlock)];
            peak = std::max(peak, std::abs(samples[i]));
        }
    }
}

std::shared_ptr<AudioSample> AudioSample::fromBuffer(const juce::String& name, juce::AudioBuffer<float> buffer, double sampleRate)
{
    auto sample = std::make_shared<AudioSample>();
    sample->name = name;
    sample->data = std::move(buffer);
    sample->sampleRate = sampleRate;
    sample->computePeaks();
    return sample;
}

juce::AudioFormatManager& AudioSample::formats()
{
    static juce::AudioFormatManager manager;
    static const bool registered = (manager.registerBasicFormats(), true);
    juce::ignoreUnused(registered);
    return manager;
}

bool AudioSample::isAudioFile(const juce::File& file)
{
    return file.hasFileExtension("wav;aif;aiff;flac;ogg;mp3;m4a;caf");
}

std::shared_ptr<AudioSample> AudioSample::load(const juce::File& file, juce::String& error)
{
    // Share decoded data between clips that use the same file.
    static std::map<juce::String, std::weak_ptr<AudioSample>> cache;
    if (auto cached = cache[file.getFullPathName()].lock())
        return cached;

    std::unique_ptr<juce::AudioFormatReader> reader(formats().createReaderFor(file));
    if (reader == nullptr)
    {
        error = "Unsupported or unreadable audio file: " + file.getFileName();
        return nullptr;
    }
    const auto maxFrames = static_cast<juce::int64>(reader->sampleRate * 60.0 * 20.0); // 20 minutes
    if (reader->lengthInSamples <= 0 || reader->lengthInSamples > maxFrames)
    {
        error = "Audio file is empty or too long: " + file.getFileName();
        return nullptr;
    }
    const int channels = juce::jlimit(1, 2, static_cast<int>(reader->numChannels));
    juce::AudioBuffer<float> buffer(channels, static_cast<int>(reader->lengthInSamples));
    reader->read(&buffer, 0, buffer.getNumSamples(), 0, true, channels > 1);
    auto sample = fromBuffer(file.getFileNameWithoutExtension(), std::move(buffer), reader->sampleRate);
    sample->file = file;
    cache[file.getFullPathName()] = sample;
    return sample;
}

//==============================================================================
Clip* Track::findClip(int clipId)
{
    for (auto& clip : data.clips)
        if (clip.id == clipId)
            return &clip;
    return nullptr;
}

//==============================================================================
juce::Colour palette::trackColour(int index)
{
    static const juce::uint32 colours[] = {
        0xffff8a3d, 0xff38d9c6, 0xff8c7bff, 0xffffc94a, 0xffff5d8f,
        0xff5fb3ff, 0xff9be15d, 0xffe07bff, 0xff4ce0a0, 0xffff6b5b,
    };
    constexpr int count = static_cast<int>(std::size(colours));
    return juce::Colour(colours[((index % count) + count) % count]);
}

Project::Project() = default;

std::shared_ptr<Track> Project::findTrack(int trackId) const
{
    for (auto& track : tracks)
        if (track->id == trackId)
            return track;
    return nullptr;
}

std::shared_ptr<Track> Project::trackOwningClip(int clipId) const
{
    for (auto& track : tracks)
        if (track->findClip(clipId) != nullptr)
            return track;
    return nullptr;
}

int Project::indexOfTrack(int trackId) const
{
    for (size_t i = 0; i < tracks.size(); ++i)
        if (tracks[i]->id == trackId)
            return static_cast<int>(i);
    return -1;
}

double Project::songEndBeats() const
{
    double end = 0.0;
    for (auto& track : tracks)
        for (auto& clip : track->data.clips)
            end = std::max(end, clip.end());
    return end;
}

juce::Colour Project::nextTrackColour()
{
    return palette::trackColour(colourIndex++);
}

void Project::changed()
{
    ++editVersion;
    flushNotes = true;
    sendChangeMessage();
}

Project::Snapshot Project::capture() const
{
    Snapshot s;
    for (auto& track : tracks)
        s.tracks.emplace_back(track, track->data);
    s.bpm = bpm;
    s.loopStart = loopStart;
    s.loopEnd = loopEnd;
    s.beatsPerBar = beatsPerBar;
    s.loopEnabled = loopEnabled;
    return s;
}

void Project::restore(const Snapshot& s)
{
    for (auto& [track, data] : s.tracks)
        if (indexOfTrack(track->id) < 0 && prepareTrack)
            prepareTrack(*track);

    {
        const juce::ScopedLock sl(lock);
        tracks.clear();
        for (auto& [track, data] : s.tracks)
        {
            track->data = data;
            tracks.push_back(track);
        }
        bpm = s.bpm;
        loopStart = s.loopStart;
        loopEnd = s.loopEnd;
        beatsPerBar = s.beatsPerBar;
        loopEnabled = s.loopEnabled;
    }
    changed();
}

void Project::checkpoint()
{
    undoStack.push_back(capture());
    if (undoStack.size() > 200)
        undoStack.erase(undoStack.begin());
    redoStack.clear();
}

bool Project::undo()
{
    if (undoStack.empty())
        return false;
    redoStack.push_back(capture());
    auto snapshot = std::move(undoStack.back());
    undoStack.pop_back();
    restore(snapshot);
    return true;
}

bool Project::redo()
{
    if (redoStack.empty())
        return false;
    undoStack.push_back(capture());
    auto snapshot = std::move(redoStack.back());
    redoStack.pop_back();
    restore(snapshot);
    return true;
}

} // namespace az

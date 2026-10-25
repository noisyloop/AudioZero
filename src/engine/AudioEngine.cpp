#include "AudioEngine.h"

namespace az
{

AudioEngine::AudioEngine(Project& p) : project(p)
{
    liveCollector.reset(sampleRate);
    recordQueue.reserve(1024);
    project.prepareTrack = [this](Track& track) { prepareTrack(track); };
}

AudioEngine::~AudioEngine()
{
    project.prepareTrack = nullptr;
}

//==============================================================================
static void configureBuses(juce::AudioProcessor& processor, bool isInstrument)
{
    processor.disableNonMainBuses();
    auto layout = processor.getBusesLayout();
    if (! layout.outputBuses.isEmpty())
        layout.outputBuses.getReference(0) = juce::AudioChannelSet::stereo();
    if (! isInstrument && ! layout.inputBuses.isEmpty())
        layout.inputBuses.getReference(0) = juce::AudioChannelSet::stereo();
    if (processor.checkBusesLayoutSupported(layout))
        processor.setBusesLayout(layout);
}

void AudioEngine::prepareDevice(Device& device, bool isInstrument)
{
    if (device.processor == nullptr)
        return;
    auto& processor = *device.processor;
    configureBuses(processor, isInstrument);
    processor.setPlayHead(this);
    processor.setRateAndBufferSizeDetails(sampleRate, blockSize);
    processor.prepareToPlay(sampleRate, blockSize);
    const int channels = std::max({ 2, processor.getTotalNumInputChannels(), processor.getTotalNumOutputChannels() });
    device.scratch.setSize(channels, blockSize);
}

void AudioEngine::prepareTrack(Track& track)
{
    if (track.instrument != nullptr)
        prepareDevice(*track.instrument, true);
    for (auto& fx : track.effects)
        prepareDevice(*fx, false);
    track.buffer.setSize(2, blockSize);
    track.midi.ensureSize(2048);
    track.preparedRate = sampleRate;
    track.preparedBlock = blockSize;
}

void AudioEngine::prepare(double newRate, int maxBlockSize)
{
    const juce::ScopedLock sl(project.lock);
    sampleRate = newRate > 0 ? newRate : 44100.0;
    blockSize = std::max(32, maxBlockSize);
    master.setSize(2, blockSize);
    liveBlock.ensureSize(2048);
    liveCollector.reset(sampleRate);
    for (auto& track : project.tracks)
        prepareTrack(*track);
}

void AudioEngine::audioDeviceAboutToStart(juce::AudioIODevice* device)
{
    prepare(device->getCurrentSampleRate(), device->getCurrentBufferSizeSamples());
}

void AudioEngine::audioDeviceStopped() {}

//==============================================================================
void AudioEngine::play()
{
    playing = true;
}

void AudioEngine::stop()
{
    if (! playing.load())
    {
        setPlayhead(0.0);
        return;
    }
    stopRequested = true;
    playing = false;
    recording = false;
}

void AudioEngine::setPlayhead(double beats)
{
    beats = std::max(0.0, beats);
    requestedPosition = beats;
    position = beats;
}

void AudioEngine::addLiveMidi(const juce::MidiMessage& message)
{
    auto copy = message;
    copy.setTimeStamp(juce::Time::getMillisecondCounterHiRes() * 0.001);
    liveCollector.addMessageToQueue(copy);
}

int AudioEngine::popRecordedEvents(std::vector<RecordedEvent>& out)
{
    const juce::SpinLock::ScopedLockType sl(recordLock);
    out.insert(out.end(), recordQueue.begin(), recordQueue.end());
    const int count = static_cast<int>(recordQueue.size());
    recordQueue.clear();
    return count;
}

void AudioEngine::previewSample(std::shared_ptr<const AudioSample> sample)
{
    const juce::SpinLock::ScopedLockType sl(previewLock);
    preview = std::move(sample);
    previewPos = 0.0;
}

juce::Optional<juce::AudioPlayHead::PositionInfo> AudioEngine::getPosition() const
{
    PositionInfo info;
    info.setBpm(project.bpm);
    info.setTimeSignature(juce::AudioPlayHead::TimeSignature { project.beatsPerBar, 4 });
    info.setPpqPosition(blockStartBeat);
    info.setPpqPositionOfLastBarStart(std::floor(blockStartBeat / project.beatsPerBar) * project.beatsPerBar);
    info.setTimeInSeconds(blockStartBeat * 60.0 / project.bpm);
    info.setTimeInSamples(static_cast<juce::int64>(blockStartBeat * 60.0 / project.bpm * sampleRate));
    info.setIsPlaying(blockPlaying);
    info.setIsRecording(blockPlaying && recording.load());
    info.setIsLooping(project.loopEnabled);
    info.setLoopPoints(juce::AudioPlayHead::LoopPoints { project.loopStart, project.loopEnd });
    return info;
}

//==============================================================================
void AudioEngine::audioDeviceIOCallbackWithContext(const float* const*, int, float* const* outputs, int numOutputs,
                                                   int numSamples, const juce::AudioIODeviceCallbackContext&)
{
    int done = 0;
    while (done < numSamples)
    {
        const int chunk = std::min(blockSize, numSamples - done);
        renderBlock(master, chunk);
        for (int ch = 0; ch < numOutputs; ++ch)
            if (outputs[ch] != nullptr)
                juce::FloatVectorOperations::copy(outputs[ch] + done, master.getReadPointer(std::min(ch, 1)), chunk);
        done += chunk;
    }
}

void AudioEngine::render(juce::AudioBuffer<float>& stereoOut, int numSamples)
{
    int done = 0;
    while (done < numSamples)
    {
        const int chunk = std::min(blockSize, numSamples - done);
        renderBlock(master, chunk);
        for (int ch = 0; ch < std::min(2, stereoOut.getNumChannels()); ++ch)
            stereoOut.copyFrom(ch, done, master, ch, 0, chunk);
        done += chunk;
    }
}

bool AudioEngine::exportWav(const juce::File& file, double endBeat, juce::String& error)
{
    const bool wasPlaying = playing.exchange(false);
    const double oldPosition = position.load();
    stopRequested = true;
    juce::AudioBuffer<float> silence(2, blockSize);
    renderBlock(silence, blockSize); // flush notes

    file.deleteFile();
    std::unique_ptr<juce::OutputStream> stream = file.createOutputStream();
    if (stream == nullptr)
    {
        error = "Cannot write " + file.getFullPathName();
        return false;
    }
    juce::WavAudioFormat wav;
    auto writer = wav.createWriterFor(stream, juce::AudioFormatWriterOptions {}.withSampleRate(sampleRate).withNumChannels(2).withBitsPerSample(24));
    if (writer == nullptr)
    {
        error = "Cannot create WAV writer";
        return false;
    }

    const bool loopWas = project.loopEnabled;
    {
        const juce::ScopedLock sl(project.lock);
        project.loopEnabled = false;
    }
    setPlayhead(0.0);
    playing = true;
    const auto totalSamples = static_cast<juce::int64>((endBeat * 60.0 / project.bpm + 2.0) * sampleRate);
    juce::AudioBuffer<float> block(2, blockSize);
    for (juce::int64 rendered = 0; rendered < totalSamples; rendered += blockSize)
    {
        const int n = static_cast<int>(std::min<juce::int64>(blockSize, totalSamples - rendered));
        renderBlock(block, n);
        writer->writeFromAudioSampleBuffer(block, 0, n);
    }
    playing = false;
    stopRequested = true;
    renderBlock(silence, blockSize);
    {
        const juce::ScopedLock sl(project.lock);
        project.loopEnabled = loopWas;
    }
    setPlayhead(oldPosition);
    playing = wasPlaying;
    return true;
}

//==============================================================================
void AudioEngine::renderBlock(juce::AudioBuffer<float>& out, int numSamples)
{
    juce::ScopedNoDenormals noDenormals;
    out.clear(0, numSamples);
    liveBlock.clear();
    liveCollector.removeNextBlockOfMessages(liveBlock, numSamples);

    const juce::ScopedLock sl(project.lock);

    bool stopNotes = project.consumeNoteFlush() || stopRequested.exchange(false);
    const double seek = requestedPosition.exchange(-1.0);
    double beat = position.load();
    if (seek >= 0.0)
    {
        beat = seek;
        stopNotes = true;
    }

    const bool isPlaying = playing.load();
    const double samplesPerBeat = sampleRate * 60.0 / project.bpm;
    blockStartBeat = beat;
    blockPlaying = isPlaying;

    // Split the block where the loop wraps around.
    Segment segments[8];
    int numSegments = 0;
    if (isPlaying)
    {
        int offset = 0;
        const bool looping = project.loopEnabled && project.loopEnd > project.loopStart + 0.01;
        if (looping && beat >= project.loopEnd)
            beat = project.loopStart;
        while (offset < numSamples && numSegments < 8)
        {
            int length = numSamples - offset;
            if (looping)
            {
                const double samplesToLoopEnd = (project.loopEnd - beat) * samplesPerBeat;
                if (samplesToLoopEnd < length)
                    length = std::max(1, static_cast<int>(std::ceil(samplesToLoopEnd)));
            }
            segments[numSegments++] = { offset, length, beat };
            beat += length / samplesPerBeat;
            offset += length;
            if (looping && beat >= project.loopEnd - 1e-9)
                beat = project.loopStart;
        }
    }

    // Record incoming notes against the timeline.
    const int liveId = liveTrackId.load();
    bool anyArmed = false;
    for (auto& track : project.tracks)
        anyArmed = anyArmed || track->data.arm;

    if (isPlaying && recording.load() && ! liveBlock.isEmpty())
    {
        const juce::SpinLock::ScopedLockType rl(recordLock);
        for (const auto meta : liveBlock)
        {
            const auto msg = meta.getMessage();
            if (! msg.isNoteOnOrOff())
                continue;
            const double eventBeat = blockStartBeat + meta.samplePosition / samplesPerBeat;
            for (auto& track : project.tracks)
                if (anyArmed ? track->data.arm : track->id == liveId)
                    recordQueue.push_back({ track->id, eventBeat, msg.getNoteNumber(), msg.getFloatVelocity(), msg.isNoteOn() });
        }
    }

    bool anySolo = false;
    for (auto& track : project.tracks)
        anySolo = anySolo || track->data.solo;

    for (auto& trackPtr : project.tracks)
    {
        auto& track = *trackPtr;
        if (track.preparedBlock < numSamples)
            continue; // not prepared yet (being added on the message thread)
        const bool isLiveTarget = anyArmed ? track.data.arm : track.id == liveId;
        renderTrack(track, segments, numSegments, numSamples, stopNotes, liveBlock, isLiveTarget);

        const bool audible = ! track.data.mute && (! anySolo || track.data.solo);
        const float pan = track.data.pan;
        const float gainL = track.data.volume * std::min(1.0f, 1.0f - pan);
        const float gainR = track.data.volume * std::min(1.0f, 1.0f + pan);
        track.buffer.applyGain(0, 0, numSamples, gainL);
        track.buffer.applyGain(1, 0, numSamples, gainR);
        const float peakL = track.buffer.getMagnitude(0, 0, numSamples);
        const float peakR = track.buffer.getMagnitude(1, 0, numSamples);
        track.meterL = std::max(peakL, track.meterL.load() * 0.85f);
        track.meterR = std::max(peakR, track.meterR.load() * 0.85f);
        if (audible)
        {
            out.addFrom(0, 0, track.buffer, 0, 0, numSamples);
            out.addFrom(1, 0, track.buffer, 1, 0, numSamples);
        }
    }

    if (metronome.load())
        for (int i = 0; i < numSegments; ++i)
            renderMetronome(out, segments[i], samplesPerBeat);
    renderPreview(out, numSamples);

    out.applyGain(0, numSamples, project.masterVolume);
    // Safety soft clipper: transparent below -0.9 dBFS, never exceeds full scale.
    for (int ch = 0; ch < 2; ++ch)
    {
        float* d = out.getWritePointer(ch);
        for (int i = 0; i < numSamples; ++i)
        {
            const float a = std::abs(d[i]);
            if (a > 0.9f)
                d[i] = std::copysign(0.9f + 0.1f * std::tanh((a - 0.9f) * 10.0f), d[i]);
        }
    }
    project.masterMeterL = std::max(out.getMagnitude(0, 0, numSamples), project.masterMeterL.load() * 0.85f);
    project.masterMeterR = std::max(out.getMagnitude(1, 0, numSamples), project.masterMeterR.load() * 0.85f);

    if (isPlaying)
        position = beat;
    else if (seek >= 0.0)
        position = seek;
}

void AudioEngine::renderTrack(Track& track, const Segment* segments, int numSegments, int numSamples,
                              bool stopNotes, const juce::MidiBuffer& live, bool isLiveTarget)
{
    auto& midi = track.midi;
    midi.clear();
    track.buffer.clear(0, numSamples);

    if (stopNotes || numSegments == 0)
        releaseClipNotes(track, 0);

    const double samplesPerBeat = sampleRate * 60.0 / project.bpm;
    for (int i = 0; i < numSegments; ++i)
    {
        if (i > 0) // loop wrapped: end notes from the previous pass
            releaseClipNotes(track, segments[i].offset);
        addClipMidi(track, segments[i], samplesPerBeat);
    }

    if (isLiveTarget)
        midi.addEvents(live, 0, numSamples, 0);

    if (track.instrument != nullptr && ! track.instrument->bypassed)
        runDevice(*track.instrument, track.buffer, midi, numSamples);

    for (int i = 0; i < numSegments; ++i)
        renderAudioClips(track, segments[i], samplesPerBeat);

    juce::MidiBuffer noMidi;
    for (auto& fx : track.effects)
        if (! fx->bypassed)
            runDevice(*fx, track.buffer, noMidi, numSamples);
}

void AudioEngine::releaseClipNotes(Track& track, int offset)
{
    for (int pitch = 0; pitch < 128; ++pitch)
        if (track.clipNotesOn[static_cast<size_t>(pitch)])
        {
            track.midi.addEvent(juce::MidiMessage::noteOff(1, pitch), offset);
            track.clipNotesOn[static_cast<size_t>(pitch)] = false;
        }
}

void AudioEngine::addClipMidi(Track& track, const Segment& seg, double samplesPerBeat)
{
    const double b0 = seg.beat;
    const double b1 = seg.beat + seg.length / samplesPerBeat;
    auto toOffset = [&](double beat) {
        return juce::jlimit(seg.offset, seg.offset + seg.length - 1,
                            seg.offset + static_cast<int>((beat - b0) * samplesPerBeat));
    };

    // Note-offs first so a re-triggered pitch at the same sample is not cut.
    for (int pass = 0; pass < 2; ++pass)
    {
        for (auto& clip : track.data.clips)
        {
            if (clip.kind != ClipKind::midi || clip.muted || clip.end() <= b0 || clip.start >= b1 + 4.0)
                continue;
            for (auto& note : clip.notes)
            {
                const double on = clip.start + note.start;
                if (note.start < 0.0 || on >= clip.end())
                    continue;
                const double off = std::min(on + note.length, clip.end());
                const auto pitch = static_cast<size_t>(juce::jlimit(0, 127, note.pitch));
                if (pass == 0 && off >= b0 && off < b1 && track.clipNotesOn[pitch])
                {
                    track.midi.addEvent(juce::MidiMessage::noteOff(1, note.pitch), toOffset(off));
                    track.clipNotesOn[pitch] = false;
                }
                if (pass == 1 && on >= b0 && on < b1)
                {
                    track.midi.addEvent(juce::MidiMessage::noteOn(1, note.pitch, juce::jlimit(0.01f, 1.0f, note.velocity)), toOffset(on));
                    track.clipNotesOn[pitch] = true;
                }
            }
        }
    }
}

void AudioEngine::renderAudioClips(Track& track, const Segment& seg, double samplesPerBeat)
{
    const double b0 = seg.beat;
    const double b1 = seg.beat + seg.length / samplesPerBeat;
    const double secondsPerBeat = 60.0 / project.bpm;
    const double fadeSeconds = 0.003;

    for (auto& clip : track.data.clips)
    {
        if (clip.kind != ClipKind::audio || clip.muted || clip.sample == nullptr || clip.end() <= b0 || clip.start >= b1)
            continue;
        const auto& sample = *clip.sample;
        const int srcFrames = sample.data.getNumSamples();
        const int srcChannels = sample.data.getNumChannels();
        const double clipSeconds = clip.length * secondsPerBeat;

        const int first = std::max(0, static_cast<int>(std::ceil((clip.start - b0) * samplesPerBeat)));
        const int last = std::min(seg.length, static_cast<int>(std::ceil((clip.end() - b0) * samplesPerBeat)));
        for (int i = first; i < last; ++i)
        {
            const double tInClip = ((b0 - clip.start) * samplesPerBeat + i) / sampleRate;
            const double srcPos = (tInClip + clip.sampleOffset) * sample.sampleRate;
            const int idx = static_cast<int>(srcPos);
            if (idx < 0 || idx + 1 >= srcFrames)
                continue;
            const float frac = static_cast<float>(srcPos - idx);
            float gain = clip.gain;
            if (tInClip < fadeSeconds)
                gain *= static_cast<float>(tInClip / fadeSeconds);
            else if (clipSeconds - tInClip < fadeSeconds)
                gain *= static_cast<float>(std::max(0.0, clipSeconds - tInClip) / fadeSeconds);
            for (int ch = 0; ch < 2; ++ch)
            {
                const float* src = sample.data.getReadPointer(std::min(ch, srcChannels - 1));
                const float value = src[idx] + frac * (src[idx + 1] - src[idx]);
                track.buffer.addSample(ch, seg.offset + i, value * gain);
            }
        }
    }
}

void AudioEngine::runDevice(Device& device, juce::AudioBuffer<float>& buffer, juce::MidiBuffer& midi, int numSamples)
{
    auto& processor = *device.processor;
    const int ins = processor.getTotalNumInputChannels();
    const int outs = processor.getTotalNumOutputChannels();
    auto& scratch = device.scratch;
    if (scratch.getNumSamples() < numSamples || outs == 0)
        return;

    juce::AudioBuffer<float> view(scratch.getArrayOfWritePointers(), scratch.getNumChannels(), numSamples);
    view.clear();
    if (ins == 1)
    {
        view.addFrom(0, 0, buffer, 0, 0, numSamples, 0.5f);
        view.addFrom(0, 0, buffer, 1, 0, numSamples, 0.5f);
    }
    else
    {
        for (int ch = 0; ch < std::min(2, ins); ++ch)
            view.copyFrom(ch, 0, buffer, ch, 0, numSamples);
    }

    {
        const juce::ScopedLock pl(processor.getCallbackLock());
        if (processor.isSuspended())
            return;
        processor.processBlock(view, midi);
    }

    buffer.copyFrom(0, 0, view, 0, 0, numSamples);
    buffer.copyFrom(1, 0, view, outs > 1 ? 1 : 0, 0, numSamples);
}

void AudioEngine::renderMetronome(juce::AudioBuffer<float>& out, const Segment& seg, double samplesPerBeat)
{
    for (int i = 0; i < seg.length; ++i)
    {
        const double b = seg.beat + i / samplesPerBeat;
        const double prev = seg.beat + (i - 1) / samplesPerBeat;
        if (i == 0 ? std::abs(b - std::round(b)) < 0.5 / samplesPerBeat : std::floor(b) != std::floor(prev))
        {
            const bool downbeat = static_cast<int>(std::round(b)) % project.beatsPerBar == 0;
            clickFreq = downbeat ? 1760.0 : 1100.0;
            clickRemaining = static_cast<int>(sampleRate * 0.03);
            clickPhase = 0.0;
        }
        if (clickRemaining > 0)
        {
            const float env = static_cast<float>(clickRemaining) / static_cast<float>(sampleRate * 0.03);
            const float v = 0.35f * env * env * static_cast<float>(std::sin(clickPhase));
            clickPhase += juce::MathConstants<double>::twoPi * clickFreq / sampleRate;
            out.addSample(0, seg.offset + i, v);
            out.addSample(1, seg.offset + i, v);
            --clickRemaining;
        }
    }
}

void AudioEngine::renderPreview(juce::AudioBuffer<float>& out, int numSamples)
{
    const juce::SpinLock::ScopedTryLockType sl(previewLock);
    if (! sl.isLocked() || preview == nullptr)
        return;
    const auto& data = preview->data;
    const double step = preview->sampleRate / sampleRate;
    for (int i = 0; i < numSamples; ++i)
    {
        const int idx = static_cast<int>(previewPos);
        if (idx + 1 >= data.getNumSamples())
        {
            preview.reset();
            return;
        }
        const float frac = static_cast<float>(previewPos - idx);
        for (int ch = 0; ch < 2; ++ch)
        {
            const float* src = data.getReadPointer(std::min(ch, data.getNumChannels() - 1));
            out.addSample(ch, i, 0.8f * (src[idx] + frac * (src[idx + 1] - src[idx])));
        }
        previewPos += step;
    }
}

} // namespace az

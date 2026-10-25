#include <juce_audio_processors/juce_audio_processors.h>

class SmokeSynth final : public juce::AudioProcessor
{
public:
    SmokeSynth() : juce::AudioProcessor(BusesProperties().withOutput("Output", juce::AudioChannelSet::stereo(), true)) {}
    const juce::String getName() const override { return "AudioZero SmokeSynth"; }
    void prepareToPlay(double sr, int) override { sampleRate = sr; }
    void releaseResources() override {}
    bool isBusesLayoutSupported(const BusesLayout& l) const override
    { return l.getMainOutputChannelSet() == juce::AudioChannelSet::stereo(); }
    void processBlock(juce::AudioBuffer<float>& audio, juce::MidiBuffer& midi) override
    {
        for (const auto metadata : midi)
        {
            const auto message = metadata.getMessage();
            if (message.isNoteOn()) active = true;
            if (message.isAllNotesOff() || message.isNoteOff()) active = false;
        }
        for (int ch = 0; ch < audio.getNumChannels(); ++ch)
            for (int i = 0; i < audio.getNumSamples(); ++i)
                audio.setSample(ch, i, active ? 0.125f : 0.0f);
    }
    juce::AudioProcessorEditor* createEditor() override { return new juce::GenericAudioProcessorEditor(*this); }
    bool hasEditor() const override { return true; }
    double getTailLengthSeconds() const override { return 0; }
    bool acceptsMidi() const override { return true; }
    bool producesMidi() const override { return false; }
    bool isMidiEffect() const override { return false; }
    int getNumPrograms() override { return 1; }
    int getCurrentProgram() override { return 0; }
    void setCurrentProgram(int) override {}
    const juce::String getProgramName(int) override { return {}; }
    void changeProgramName(int, const juce::String&) override {}
    void getStateInformation(juce::MemoryBlock& state) override
    {
        state.setSize(1);
        static_cast<unsigned char*>(state.getData())[0] = active ? 1 : 0;
    }
    void setStateInformation(const void* data, int size) override
    { if (size > 0) active = static_cast<const unsigned char*>(data)[0] != 0; }
private:
    double sampleRate = 44100.0;
    bool active = false;
};
juce::AudioProcessor* JUCE_CALLTYPE createPluginFilter() { return new SmokeSynth(); }

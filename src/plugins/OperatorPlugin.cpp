// Entry point for the standalone "AudioZero Operator" VST3 build.
#include "../dsp/OperatorSynth.h"

juce::AudioProcessor* JUCE_CALLTYPE createPluginFilter()
{
    return new az::OperatorSynth();
}

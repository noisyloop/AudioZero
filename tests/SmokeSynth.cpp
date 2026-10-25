#include "../src/SmokeSynth.h"
juce::AudioProcessor* JUCE_CALLTYPE createPluginFilter() { return new SmokeSynth(); }

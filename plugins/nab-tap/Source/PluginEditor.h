#pragma once

#include "PluginProcessor.h"

class NabTapAudioProcessorEditor final : public juce::AudioProcessorEditor,
                                         private juce::Timer
{
public:
    explicit NabTapAudioProcessorEditor(NabTapAudioProcessor&);
    ~NabTapAudioProcessorEditor() override;

    void paint(juce::Graphics&) override;
    void resized() override;

private:
    void timerCallback() override;

    NabTapAudioProcessor& processor;
    uint64_t lastPackets = 0;
    uint64_t lastCallbacks = 0;
    uint64_t lastFrames = 0;
    bool packetsMoving = false;
    bool audioMoving = false;

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR(NabTapAudioProcessorEditor)
};

#include "PluginEditor.h"

NabTapAudioProcessorEditor::NabTapAudioProcessorEditor(NabTapAudioProcessor& p)
    : AudioProcessorEditor(&p), processor(p)
{
    setSize(360, 170);
    startTimerHz(2);
}

NabTapAudioProcessorEditor::~NabTapAudioProcessorEditor() = default;

void NabTapAudioProcessorEditor::paint(juce::Graphics& g)
{
    g.fillAll(juce::Colour(0xff07121f));

    auto area = getLocalBounds().reduced(16);
    g.setColour(juce::Colour(0xff49d3ff));
    g.setFont(juce::Font(juce::FontOptions(22.0f, juce::Font::bold)));
    g.drawFittedText("NAB Tap", area.removeFromTop(34), juce::Justification::left, 1);

    g.setFont(juce::Font(juce::FontOptions(14.0f)));
    g.setColour(juce::Colour(0xffc7f3ff));
    g.drawFittedText("Pass-through master tap for nab-live", area.removeFromTop(24), juce::Justification::left, 1);

    area.removeFromTop(10);
    const auto packets = processor.getPacketsSent();
    const auto drops = processor.getDroppedFrames();
    const auto errors = processor.getSocketErrors();
    const auto sampleRate = processor.getSampleRateHz();

    g.setColour(juce::Colour(0xff91ffb8));
    g.drawFittedText("Sample rate: " + juce::String(sampleRate) + " Hz", area.removeFromTop(22), juce::Justification::left, 1);
    g.drawFittedText("Packets sent: " + juce::String(packets), area.removeFromTop(22), juce::Justification::left, 1);

    g.setColour((drops > 0 || errors > 0) ? juce::Colour(0xffff6b6b) : juce::Colour(0xff8a99a8));
    g.drawFittedText("Dropped frames: " + juce::String(drops) + "   socket errors: " + juce::String(errors),
                     area.removeFromTop(22),
                     juce::Justification::left,
                     1);
}

void NabTapAudioProcessorEditor::resized()
{
}

void NabTapAudioProcessorEditor::timerCallback()
{
    repaint();
}

#include "PluginEditor.h"

NabTapAudioProcessorEditor::NabTapAudioProcessorEditor(NabTapAudioProcessor& p)
    : AudioProcessorEditor(&p), processor(p)
{
    setSize(520, 260);
    startTimerHz(2);
}

NabTapAudioProcessorEditor::~NabTapAudioProcessorEditor() = default;

void NabTapAudioProcessorEditor::paint(juce::Graphics& g)
{
    g.fillAll(juce::Colour(0xff07121f));

    auto area = getLocalBounds().reduced(16);
    g.setColour(juce::Colour(0xff49d3ff));
    g.setFont(juce::Font(juce::FontOptions(26.0f, juce::Font::bold)));
    g.drawFittedText("NAB Tap", area.removeFromTop(36), juce::Justification::left, 1);

    g.setFont(juce::Font(juce::FontOptions(14.0f)));
    g.setColour(juce::Colour(0xffc7f3ff));
    g.drawFittedText("Pass-through master tap for nab-live", area.removeFromTop(24), juce::Justification::left, 1);

    area.removeFromTop(10);
    const auto packets = processor.getPacketsSent();
    const auto drops = processor.getDroppedFrames();
    const auto errors = processor.getSocketErrors();
    const auto sampleRate = processor.getSampleRateHz();
    const auto callbacks = processor.getAudioCallbacks();
    const auto frames = processor.getFramesSeen();
    const auto socketReady = processor.getSocketReady();

    const auto statusColour = packetsMoving ? juce::Colour(0xff91ffb8)
                                            : (audioMoving ? juce::Colour(0xffffd166)
                                                           : juce::Colour(0xffff6b6b));
    const auto statusText = packetsMoving ? "CONNECTED - audio is reaching nab-live"
                                          : (audioMoving ? "WAITING - start nab-live sender"
                                                         : "NO AUDIO CALLBACKS YET");

    auto statusArea = area.removeFromTop(46);
    g.setColour(statusColour.withAlpha(0.18f));
    g.fillRoundedRectangle(statusArea.toFloat(), 8.0f);
    g.setColour(statusColour);
    g.setFont(juce::Font(juce::FontOptions(17.0f, juce::Font::bold)));
    g.drawFittedText(statusText, statusArea.reduced(12, 8), juce::Justification::centredLeft, 1);

    area.removeFromTop(10);

    g.setColour(juce::Colour(0xff91ffb8));
    g.setFont(juce::Font(juce::FontOptions(14.0f)));
    g.drawFittedText("Sample rate: " + juce::String(sampleRate) + " Hz", area.removeFromTop(22), juce::Justification::left, 1);
    g.drawFittedText("Audio callbacks: " + juce::String(callbacks) + "   frames seen: " + juce::String(frames),
                     area.removeFromTop(22),
                     juce::Justification::left,
                     1);
    g.drawFittedText("Packets sent to nab-live: " + juce::String(packets),
                     area.removeFromTop(22),
                     juce::Justification::left,
                     1);

    g.setColour((drops > 0 || errors > 0) ? juce::Colour(0xffff6b6b) : juce::Colour(0xff8a99a8));
    g.drawFittedText("Dropped frames: " + juce::String(drops) + "   socket errors: " + juce::String(errors),
                     area.removeFromTop(22),
                     juce::Justification::left,
                     1);
    g.drawFittedText("Socket: " + juce::String(socketReady ? "ready" : "not ready") +
                         "   Open NAB Live Sender.command to create the receiver.",
                     area.removeFromTop(22),
                     juce::Justification::left,
                     1);
}

void NabTapAudioProcessorEditor::resized()
{
}

void NabTapAudioProcessorEditor::timerCallback()
{
    const auto packets = processor.getPacketsSent();
    const auto callbacks = processor.getAudioCallbacks();
    const auto frames = processor.getFramesSeen();

    packetsMoving = packets > lastPackets;
    audioMoving = callbacks > lastCallbacks || frames > lastFrames;
    lastPackets = packets;
    lastCallbacks = callbacks;
    lastFrames = frames;

    repaint();
}

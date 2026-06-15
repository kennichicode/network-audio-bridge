#pragma once

#include <JuceHeader.h>
#include <atomic>
#include <cstdint>
#include <vector>

class NabTapBridge final : private juce::Thread
{
public:
    NabTapBridge();
    ~NabTapBridge() override;

    void prepare(double sampleRate, int maxBlockSize);
    void release();
    void pushAudio(const juce::AudioBuffer<float>& buffer) noexcept;

    uint64_t getDroppedFrames() const noexcept { return droppedFrames.load(std::memory_order_relaxed); }
    uint64_t getPacketsSent() const noexcept { return packetsSent.load(std::memory_order_relaxed); }
    uint64_t getSocketErrors() const noexcept { return socketErrors.load(std::memory_order_relaxed); }
    uint32_t getSampleRateHz() const noexcept { return sampleRateHz.load(std::memory_order_relaxed); }

private:
    void run() override;
    void resetSocket();
    bool ensureSocket();
    size_t popSamples(float* dest, size_t maxSamples) noexcept;

    std::vector<float> ring;
    size_t ringMask = 0;
    std::atomic<uint64_t> readIndex { 0 };
    std::atomic<uint64_t> writeIndex { 0 };
    std::atomic<uint64_t> droppedFrames { 0 };
    std::atomic<uint64_t> packetsSent { 0 };
    std::atomic<uint64_t> socketErrors { 0 };
    std::atomic<uint64_t> sequence { 0 };
    std::atomic<uint32_t> sampleRateHz { 48000 };

    int socketFd = -1;

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR(NabTapBridge)
};

class NabTapAudioProcessor final : public juce::AudioProcessor
{
public:
    NabTapAudioProcessor();
    ~NabTapAudioProcessor() override;

    void prepareToPlay(double sampleRate, int samplesPerBlock) override;
    void releaseResources() override;
    bool isBusesLayoutSupported(const BusesLayout& layouts) const override;
    void processBlock(juce::AudioBuffer<float>&, juce::MidiBuffer&) override;

    juce::AudioProcessorEditor* createEditor() override;
    bool hasEditor() const override { return true; }

    const juce::String getName() const override { return JucePlugin_Name; }
    bool acceptsMidi() const override { return false; }
    bool producesMidi() const override { return false; }
    bool isMidiEffect() const override { return false; }
    double getTailLengthSeconds() const override { return 0.0; }

    int getNumPrograms() override { return 1; }
    int getCurrentProgram() override { return 0; }
    void setCurrentProgram(int) override {}
    const juce::String getProgramName(int) override { return {}; }
    void changeProgramName(int, const juce::String&) override {}

    void getStateInformation(juce::MemoryBlock&) override {}
    void setStateInformation(const void*, int) override {}

    uint64_t getDroppedFrames() const noexcept { return bridge.getDroppedFrames(); }
    uint64_t getPacketsSent() const noexcept { return bridge.getPacketsSent(); }
    uint64_t getSocketErrors() const noexcept { return bridge.getSocketErrors(); }
    uint32_t getSampleRateHz() const noexcept { return bridge.getSampleRateHz(); }

private:
    NabTapBridge bridge;

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR(NabTapAudioProcessor)
};

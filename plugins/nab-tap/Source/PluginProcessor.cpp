#include "PluginProcessor.h"
#include "PluginEditor.h"

#include <algorithm>
#include <cerrno>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <fcntl.h>
#include <memory>
#include <string>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

namespace
{
constexpr const char* kSocketEnv = "NAB_TAP_SOCKET";
constexpr const char* kSocketRelativePath = "/Library/Caches/KenichiNAB/nab-tap.sock";
constexpr const char* kFallbackSocketPath = "/tmp/nab-tap.sock";
constexpr uint32_t kTapVersion = 1;
constexpr uint16_t kTapChannels = 2;
constexpr size_t kFramesPerPacket = 240;
constexpr size_t kMaxBlockSize = 16384;
constexpr size_t kMaxRingSeconds = 4;
constexpr int kSocketBufferBytes = 4 * 1024 * 1024;

#pragma pack(push, 1)
struct TapPacketHeader
{
    char magic[8];
    uint32_t version;
    uint32_t sampleRate;
    uint16_t channels;
    uint16_t frames;
    uint64_t sequence;
    uint64_t droppedFrames;
    uint32_t reserved;
};
#pragma pack(pop)

static_assert(sizeof(TapPacketHeader) == 40, "Unexpected NAB Tap header size");

size_t nextPowerOfTwo(size_t value)
{
    size_t result = 1;
    while (result < value)
        result <<= 1;
    return result;
}

std::string defaultSocketPath()
{
    if (const auto* overridePath = std::getenv(kSocketEnv))
    {
        if (overridePath[0] != '\0')
            return overridePath;
    }

    if (const auto* home = std::getenv("HOME"))
    {
        if (home[0] != '\0')
            return std::string(home) + kSocketRelativePath;
    }

    return kFallbackSocketPath;
}

sockaddr_un makeAddress()
{
    const auto path = defaultSocketPath();
    sockaddr_un address {};
    address.sun_family = AF_UNIX;
    std::strncpy(address.sun_path, path.c_str(), sizeof(address.sun_path) - 1);
    return address;
}

bool isSocketPath(const std::string& path) noexcept
{
    struct stat info {};
    return ::lstat(path.c_str(), &info) == 0 && S_ISSOCK(info.st_mode);
}

void configureSocketBuffer(int fd) noexcept
{
    if (fd < 0)
        return;

    int size = kSocketBufferBytes;
    ::setsockopt(fd, SOL_SOCKET, SO_SNDBUF, &size, sizeof(size));
}
}

NabTapBridge::NabTapBridge()
    : juce::Thread("NAB Tap IPC")
{
}

NabTapBridge::~NabTapBridge()
{
    release();
}

void NabTapBridge::prepare(double sampleRate, int maxBlockSize)
{
    release();

    const auto sr = static_cast<uint32_t>(std::max(8000.0, std::min(sampleRate, 384000.0)));
    sampleRateHz.store(sr, std::memory_order_relaxed);

    const auto blockSize = std::min(static_cast<size_t>(std::max(1, maxBlockSize)), kMaxBlockSize);
    const auto samplesForFourSeconds = static_cast<size_t>(sr) * kTapChannels * kMaxRingSeconds;
    const auto samplesForBlocks = blockSize * kTapChannels * 64;
    const auto capacity = nextPowerOfTwo(std::max(samplesForFourSeconds, samplesForBlocks));

    std::shared_ptr<NabTapRingState> nextState;
    try
    {
        nextState = std::make_shared<NabTapRingState>(capacity);
    }
    catch (...)
    {
        socketErrors.fetch_add(1, std::memory_order_relaxed);
        return;
    }

    readIndex.store(0, std::memory_order_relaxed);
    writeIndex.store(0, std::memory_order_relaxed);
    droppedFrames.store(0, std::memory_order_relaxed);
    audioCallbacks.store(0, std::memory_order_relaxed);
    framesSeen.store(0, std::memory_order_relaxed);
    sequence.store(0, std::memory_order_relaxed);
    socketReady.store(false, std::memory_order_relaxed);
    std::atomic_store_explicit(&ringState, nextState, std::memory_order_release);

    startThread(juce::Thread::Priority::background);
}

void NabTapBridge::release()
{
    std::shared_ptr<NabTapRingState> empty;
    std::atomic_store_explicit(&ringState, empty, std::memory_order_release);
    socketReady.store(false, std::memory_order_relaxed);
    signalThreadShouldExit();
    stopThread(1000);
    resetSocket();
}

void NabTapBridge::pushAudio(const juce::AudioBuffer<float>& buffer) noexcept
{
    const auto state = std::atomic_load_explicit(&ringState, std::memory_order_acquire);
    if (! state || state->ring.empty())
        return;

    const auto numFrames = buffer.getNumSamples();
    const auto numChannels = buffer.getNumChannels();
    audioCallbacks.fetch_add(1, std::memory_order_relaxed);
    framesSeen.fetch_add(static_cast<uint64_t>(std::max(0, numFrames)), std::memory_order_relaxed);
    const float* left = numChannels > 0 ? buffer.getReadPointer(0) : nullptr;
    const float* right = numChannels > 1 ? buffer.getReadPointer(1) : left;

    auto read = readIndex.load(std::memory_order_acquire);
    auto write = writeIndex.load(std::memory_order_relaxed);
    const auto capacity = static_cast<uint64_t>(state->ring.size());
    uint64_t dropped = 0;

    for (int frame = 0; frame < numFrames; ++frame)
    {
        if ((write - read + kTapChannels) > capacity)
        {
            ++dropped;
            continue;
        }

        const auto l = left != nullptr ? left[frame] : 0.0f;
        const auto r = right != nullptr ? right[frame] : l;
        state->ring[static_cast<size_t>(write++) & state->mask] = l;
        state->ring[static_cast<size_t>(write++) & state->mask] = r;
    }

    if (dropped > 0)
        droppedFrames.fetch_add(dropped, std::memory_order_relaxed);

    writeIndex.store(write, std::memory_order_release);
}

void NabTapBridge::run()
{
    std::vector<float> audio(kFramesPerPacket * kTapChannels, 0.0f);
    std::vector<char> packet(sizeof(TapPacketHeader) + audio.size() * sizeof(float), 0);
    bool receiverWasMissing = true;
    double nextSilencePacketMs = 0.0;

    while (! threadShouldExit())
    {
        const auto ready = destinationReady();
        socketReady.store(ready, std::memory_order_relaxed);
        if (! ready || ! ensureSocket())
        {
            dropBufferedSamples();
            receiverWasMissing = true;
            wait(100);
            continue;
        }

        if (receiverWasMissing)
        {
            dropBufferedSamples();
            receiverWasMissing = false;
            wait(2);
            continue;
        }

        const auto sampleRate = sampleRateHz.load(std::memory_order_relaxed);
        const auto packetIntervalMs =
            (1000.0 * static_cast<double>(kFramesPerPacket))
            / static_cast<double>(std::max<uint32_t>(1, sampleRate));
        const auto nowMs = juce::Time::getMillisecondCounterHiRes();
        const auto haveSamples = availableSamples();

        size_t samples = 0;
        if (haveSamples >= audio.size())
        {
            samples = popSamples(audio.data(), audio.size());
            nextSilencePacketMs = nowMs + packetIntervalMs;
        }
        else
        {
            if (nowMs < nextSilencePacketMs)
            {
                const auto waitMs =
                    static_cast<int>(std::max(1.0, std::ceil(nextSilencePacketMs - nowMs)));
                wait(waitMs);
                continue;
            }

            std::fill(audio.begin(), audio.end(), 0.0f);
            samples = audio.size();
            nextSilencePacketMs = nowMs + packetIntervalMs;
        }

        const auto frames = static_cast<uint16_t>(samples / kTapChannels);
        auto* header = reinterpret_cast<TapPacketHeader*>(packet.data());
        std::memcpy(header->magic, "NABTAP1", 7);
        header->magic[7] = '\0';
        header->version = kTapVersion;
        header->sampleRate = sampleRateHz.load(std::memory_order_relaxed);
        header->channels = kTapChannels;
        header->frames = frames;
        header->sequence = sequence.fetch_add(1, std::memory_order_relaxed);
        header->droppedFrames = droppedFrames.load(std::memory_order_relaxed);
        header->reserved = 0;

        std::memcpy(packet.data() + sizeof(TapPacketHeader), audio.data(), samples * sizeof(float));

        auto address = makeAddress();
        const auto bytes = sizeof(TapPacketHeader) + samples * sizeof(float);
        const auto sent = ::sendto(
            socketFd,
            packet.data(),
            bytes,
            MSG_DONTWAIT,
            reinterpret_cast<sockaddr*>(&address),
            sizeof(address));

        if (sent == static_cast<ssize_t>(bytes))
        {
            packetsSent.fetch_add(1, std::memory_order_relaxed);
            socketReady.store(true, std::memory_order_relaxed);
        }
        else
        {
            const auto err = errno;
            lastSocketErrno.store(err, std::memory_order_relaxed);
            socketErrors.fetch_add(1, std::memory_order_relaxed);
            droppedFrames.fetch_add(frames, std::memory_order_relaxed);

            if (err == EAGAIN || err == EWOULDBLOCK || err == ENOBUFS)
            {
                socketReady.store(true, std::memory_order_relaxed);
                wait(2);
                continue;
            }

            if (err == ENOENT || err == ECONNREFUSED || err == EBADF)
                resetSocket();
            socketReady.store(false, std::memory_order_relaxed);
            receiverWasMissing = true;
            wait(5);
        }
    }
}

void NabTapBridge::resetSocket()
{
    if (socketFd >= 0)
    {
        ::close(socketFd);
        socketFd = -1;
    }
}

bool NabTapBridge::ensureSocket()
{
    if (socketFd >= 0)
        return true;

    socketFd = ::socket(AF_UNIX, SOCK_DGRAM, 0);
    if (socketFd < 0)
    {
        lastSocketErrno.store(errno, std::memory_order_relaxed);
        socketErrors.fetch_add(1, std::memory_order_relaxed);
        return false;
    }

    configureSocketBuffer(socketFd);

    const auto flags = ::fcntl(socketFd, F_GETFL, 0);
    if (flags >= 0)
        ::fcntl(socketFd, F_SETFL, flags | O_NONBLOCK);

    const auto fdFlags = ::fcntl(socketFd, F_GETFD, 0);
    if (fdFlags >= 0)
        ::fcntl(socketFd, F_SETFD, fdFlags | FD_CLOEXEC);

    return true;
}

bool NabTapBridge::destinationReady() const noexcept
{
    const auto path = defaultSocketPath();
    sockaddr_un address {};
    if (path.size() >= sizeof(address.sun_path))
        return false;

    return isSocketPath(path);
}

void NabTapBridge::dropBufferedSamples() noexcept
{
    readIndex.store(writeIndex.load(std::memory_order_acquire), std::memory_order_release);
}

size_t NabTapBridge::availableSamples() const noexcept
{
    const auto state = std::atomic_load_explicit(&ringState, std::memory_order_acquire);
    if (! state)
        return 0;

    const auto read = readIndex.load(std::memory_order_relaxed);
    const auto write = writeIndex.load(std::memory_order_acquire);
    return static_cast<size_t>(write - read);
}

size_t NabTapBridge::popSamples(float* dest, size_t maxSamples) noexcept
{
    const auto state = std::atomic_load_explicit(&ringState, std::memory_order_acquire);
    if (! state || state->ring.empty())
        return 0;

    auto read = readIndex.load(std::memory_order_relaxed);
    const auto write = writeIndex.load(std::memory_order_acquire);
    auto available = static_cast<size_t>(write - read);
    auto toRead = std::min(maxSamples, available);
    toRead -= toRead % kTapChannels;

    for (size_t i = 0; i < toRead; ++i)
        dest[i] = state->ring[static_cast<size_t>(read++) & state->mask];

    readIndex.store(read, std::memory_order_release);
    return toRead;
}

NabTapAudioProcessor::NabTapAudioProcessor()
    : AudioProcessor(
          BusesProperties()
              .withInput("Input", juce::AudioChannelSet::stereo(), true)
              .withOutput("Output", juce::AudioChannelSet::stereo(), true))
{
}

NabTapAudioProcessor::~NabTapAudioProcessor() = default;

void NabTapAudioProcessor::prepareToPlay(double sampleRate, int samplesPerBlock)
{
    bridge.prepare(sampleRate, samplesPerBlock);
}

void NabTapAudioProcessor::releaseResources()
{
    bridge.release();
}

bool NabTapAudioProcessor::isBusesLayoutSupported(const BusesLayout& layouts) const
{
    const auto input = layouts.getMainInputChannelSet();
    const auto output = layouts.getMainOutputChannelSet();

    if (input != output)
        return false;

    const auto channels = output.size();
    return channels == 1 || channels == 2;
}

void NabTapAudioProcessor::processBlock(juce::AudioBuffer<float>& buffer, juce::MidiBuffer&)
{
    juce::ScopedNoDenormals noDenormals;
    bridge.pushAudio(buffer);

    for (auto channel = getTotalNumInputChannels(); channel < getTotalNumOutputChannels(); ++channel)
        buffer.clear(channel, 0, buffer.getNumSamples());
}

juce::AudioProcessorEditor* NabTapAudioProcessor::createEditor()
{
    return new NabTapAudioProcessorEditor(*this);
}

juce::AudioProcessor* JUCE_CALLTYPE createPluginFilter()
{
    return new NabTapAudioProcessor();
}

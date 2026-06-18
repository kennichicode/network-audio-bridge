use byteorder::{ByteOrder, LittleEndian};
use clap::{Parser, ValueEnum};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use livekit::options::{AudioEncoding, TrackPublishOptions};
use livekit::prelude::*;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::prelude::{AudioFrame, AudioSourceOptions};
use livekit_api::access_token::{AccessToken, VideoGrants};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph},
    Terminal,
};
use ringbuf::HeapRb;
use rubato::{FftFixedInOut, Resampler};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
#[cfg(unix)]
use std::os::unix::net::UnixDatagram;
use tokio::sync::mpsc::UnboundedReceiver;

#[path = "../log.rs"]
mod log;

const OUT_RATE: u32 = 48_000;
const OUT_CHANNELS: usize = 2;
const OUT_FRAME_MS: u32 = 10;
const OUT_FRAMES_PER_PACKET: usize = (OUT_RATE as usize * OUT_FRAME_MS as usize) / 1000;

const TAP_MAGIC: &[u8; 8] = b"NABTAP1\0";
const TAP_VERSION: u32 = 1;
const TAP_HEADER_BYTES: usize = 40;
const TAP_MAX_PACKET_BYTES: usize = 64 * 1024;
const DEFAULT_TAP_SOCKET: &str = "~/Library/Caches/KenichiNAB/nab-tap.sock";
const MIN_BITRATE: u64 = 16_000;
const MAX_BITRATE: u64 = 510_000;
const MIN_LIVEKIT_BUFFER_MS: u32 = 20;
const MAX_LIVEKIT_BUFFER_MS: u32 = 5_000;
const MAX_RING_BUFFER_SECONDS: usize = 30;
const CONNECTION_CONNECTING: usize = 0;
const CONNECTION_CONNECTED: usize = 1;
const CONNECTION_RECONNECTING: usize = 2;
const CONNECTION_DISCONNECTED: usize = 3;
static TERMINATE_REQUESTED: AtomicBool = AtomicBool::new(false);

type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum SourceArg {
    Plugin,
    Coreaudio,
    TestTone,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum AudioProfileArg {
    /// Stable music monitoring default.
    #[value(
        name = "stable-music",
        alias = "default-stable-music",
        alias = "balanced"
    )]
    StableMusic,
    /// High-quality music monitoring when the network is stable.
    #[value(name = "hi-fi-music", alias = "concert")]
    HiFiMusic,
    /// Speech-oriented, lower bitrate, enables DTX unless overridden.
    Speech,
    /// Lowest bandwidth profile for difficult networks.
    #[value(name = "low-bandwidth", alias = "safe-low-bandwidth")]
    LowBandwidth,
    /// Lab-only maximum Opus bitrate. Do not use as the default.
    #[value(name = "max-quality-lab")]
    MaxQualityLab,
}

#[derive(Parser, Debug)]
#[command(name = "nab-live")]
#[command(about = "Publish REAPER/NAB audio to LiveKit/WebRTC with an optional terminal wizard.")]
struct Args {
    /// List CoreAudio input devices and exit.
    #[arg(long)]
    list_devices: bool,

    /// Audio source. If omitted with no --input, a wizard is shown.
    #[arg(long, value_enum)]
    source: Option<SourceArg>,

    /// Disable the status TUI and print plain log lines instead.
    #[arg(long)]
    no_tui: bool,

    /// Unix socket used by the NAB Tap plugin.
    #[arg(long, default_value = DEFAULT_TAP_SOCKET)]
    plugin_socket: String,

    /// CoreAudio input device name substring. Implies --source coreaudio.
    #[arg(long)]
    input: Option<String>,

    /// CoreAudio input sample rate. Use 96000 for a 96kHz REAPER session.
    #[arg(long, default_value_t = 96_000)]
    input_sample_rate: u32,

    /// Use an internal test tone instead of REAPER/NAB Tap. This bypasses VST3.
    #[arg(long)]
    test_tone: bool,

    /// Test tone duration in seconds. Omit for continuous tone.
    #[arg(long)]
    test_tone_duration: Option<f64>,

    /// Test tone frequency in Hz.
    #[arg(long, default_value_t = 1_000.0)]
    test_tone_hz: f32,

    /// Test tone level in dBFS.
    #[arg(long, default_value_t = -18.0)]
    test_tone_dbfs: f32,

    /// Number of channels to open on the CoreAudio input device.
    #[arg(long, default_value_t = 2)]
    input_channels: u16,

    /// 1-based source channel to use as left.
    #[arg(long, default_value_t = 1)]
    left_channel: usize,

    /// 1-based source channel to use as right.
    #[arg(long, default_value_t = 2)]
    right_channel: usize,

    /// LiveKit URL. Also read from LIVEKIT_URL in --env-file or the environment.
    #[arg(long)]
    url: Option<String>,

    /// LiveKit room name.
    #[arg(long, default_value = "reaper-master")]
    room: String,

    /// LiveKit participant identity.
    #[arg(long, default_value = "nab-live-mac-mini")]
    identity: String,

    /// LiveKit API key. Also read from LIVEKIT_KEY or LIVEKIT_API_KEY.
    #[arg(long)]
    key: Option<String>,

    /// LiveKit API secret. Also read from LIVEKIT_SECRET or LIVEKIT_API_SECRET.
    #[arg(long)]
    secret: Option<String>,

    /// Env file containing LIVEKIT_URL and credentials.
    #[arg(long, default_value = "~/.config/kenichi-vps/livekit.env")]
    env_file: String,

    /// Audio profile. stable-music is the default for music monitoring.
    #[arg(long, value_enum, default_value_t = AudioProfileArg::StableMusic)]
    profile: AudioProfileArg,

    /// Audio bitrate requested for the published track. Overrides --profile.
    #[arg(long)]
    bitrate: Option<u64>,

    /// Disable LiveKit/WebRTC RED redundant audio payloads.
    #[arg(long)]
    disable_red: bool,

    /// Enable DTX. Useful for speech; normally keep disabled for music.
    #[arg(long, conflicts_with = "disable_dtx")]
    enable_dtx: bool,

    /// Disable DTX, even when the selected profile would enable it.
    #[arg(long)]
    disable_dtx: bool,

    /// LiveKit native source queue in milliseconds.
    #[arg(long)]
    livekit_buffer_ms: Option<u32>,

    /// Disable automatic reconnect on LiveKit/session errors.
    #[arg(long)]
    no_reconnect: bool,

    /// Local capture ring buffer in seconds.
    #[arg(long, default_value_t = 4)]
    ring_buffer_seconds: usize,

    /// JSON status file for external health checks.
    #[arg(long, default_value = "~/.nab/status.json")]
    status_file: String,
}

#[derive(Clone)]
enum CaptureConfig {
    Plugin {
        socket_path: String,
    },
    TestTone {
        frequency_hz: f32,
        level_dbfs: f32,
        duration: Option<Duration>,
    },
    CoreAudio {
        input: Option<String>,
        input_sample_rate: u32,
        input_channels: u16,
        left_channel: usize,
        right_channel: usize,
    },
}

struct RuntimeInfo {
    source_label: String,
    source_kind: String,
    livekit_url: String,
    room: String,
    identity: String,
    status_file: PathBuf,
    input_rate: u32,
    audio_profile: AudioProfileArg,
    bitrate: u64,
    red: bool,
    dtx: bool,
    livekit_buffer_ms: u32,
}

#[derive(Clone, Debug)]
struct AudioSendConfig {
    profile: AudioProfileArg,
    bitrate: u64,
    red: bool,
    dtx: bool,
    livekit_buffer_ms: u32,
}

#[derive(Clone, Debug)]
struct CoreAudioCaptureOptions {
    input: Option<String>,
    input_sample_rate: u32,
    input_channels: u16,
    left_channel: usize,
    right_channel: usize,
}

#[derive(Clone, Debug)]
struct LiveKitConfig {
    url: String,
    api_key: String,
    api_secret: String,
    room: String,
    identity: String,
    audio: AudioSendConfig,
    reconnect: bool,
}

#[derive(Default)]
struct State {
    captured_frames: AtomicU64,
    sent_frames: AtomicU64,
    overflow_frames: AtomicU64,
    underruns: AtomicU64,
    capture_errors: AtomicU64,
    livekit_errors: AtomicU64,
    reconnects: AtomicU64,
    connection_state: AtomicUsize,
    ring_buffer_ms: AtomicUsize,
    peak_l_milli: AtomicUsize,
    peak_r_milli: AtomicUsize,
    rms_l_milli: AtomicUsize,
    rms_r_milli: AtomicUsize,
    last_audio_unix_ms: AtomicU64,
    tap_packets: AtomicU64,
    tap_seq_gaps: AtomicU64,
    tap_reported_drops: AtomicU64,
}

struct CaptureSetup {
    input_rate: u32,
    source_label: String,
    guard: CaptureGuard,
    cons: ringbuf::Consumer<f32, Arc<HeapRb<f32>>>,
}

enum CaptureGuard {
    CoreAudio(cpal::Stream),
    #[cfg(unix)]
    Plugin(PluginCaptureGuard),
    TestTone(TestToneGuard),
}

impl CaptureGuard {
    fn kind(&self) -> &'static str {
        match self {
            CaptureGuard::CoreAudio(stream) => {
                let _ = stream;
                "coreaudio"
            }
            #[cfg(unix)]
            CaptureGuard::Plugin(guard) => {
                let _ = guard.running.load(Ordering::Relaxed);
                "plugin"
            }
            CaptureGuard::TestTone(guard) => {
                let _ = guard.running.load(Ordering::Relaxed);
                "test-tone"
            }
        }
    }
}

#[cfg(unix)]
extern "C" fn handle_termination_signal(_: libc::c_int) {
    TERMINATE_REQUESTED.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
fn install_termination_handler() {
    unsafe {
        libc::signal(
            libc::SIGTERM,
            handle_termination_signal as *const () as libc::sighandler_t,
        );
    }
}

#[cfg(not(unix))]
fn install_termination_handler() {}

fn termination_requested() -> bool {
    TERMINATE_REQUESTED.load(Ordering::SeqCst)
}

struct TestToneGuard {
    running: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Drop for TestToneGuard {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct RuntimeInstanceLock {
    _file: File,
    path: PathBuf,
}

impl Drop for RuntimeInstanceLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
struct PluginCaptureGuard {
    running: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    socket_path: PathBuf,
    _lock: PluginInstanceLock,
}

#[cfg(unix)]
struct PluginInstanceLock {
    _file: File,
    path: PathBuf,
}

#[cfg(unix)]
impl Drop for PluginInstanceLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
impl Drop for PluginCaptureGuard {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let _ = fs::remove_file(&self.socket_path);
        let _ = fs::remove_file(&self._lock.path);
    }
}

#[derive(Clone)]
enum WizardStep {
    SelectSource { cursor: usize },
    SelectSampleRate { cursor: usize },
    SelectInput { devices: Vec<String>, cursor: usize },
    Confirm,
}

#[tokio::main]
async fn main() -> AppResult<()> {
    log::init("nab-live");
    install_termination_handler();

    let args = Args::parse();
    let host = cpal::default_host();

    if args.list_devices {
        list_input_devices(&host)?;
        return Ok(());
    }

    validate_buffer_seconds(args.ring_buffer_seconds)?;
    let audio_config = resolve_audio_send_config(&args)?;
    let capture_config = if should_show_wizard(&args) {
        match run_wizard(&host)? {
            Some(config) => config,
            None => return Ok(()),
        }
    } else {
        capture_config_from_args(&args)?
    };

    let env = load_env(&args.env_file);
    let livekit_url = normalize_livekit_url(
        args.url
            .clone()
            .or_else(|| read_key(&env, &["LIVEKIT_URL", "LK_URL"]))
            .ok_or_else(|| io_error("LIVEKIT_URL is missing"))?,
    );
    let api_key = args
        .key
        .clone()
        .or_else(|| read_key(&env, &["LIVEKIT_KEY", "LIVEKIT_API_KEY"]))
        .ok_or_else(|| io_error("LIVEKIT_KEY/LIVEKIT_API_KEY is missing"))?;
    let api_secret = args
        .secret
        .clone()
        .or_else(|| read_key(&env, &["LIVEKIT_SECRET", "LIVEKIT_API_SECRET"]))
        .ok_or_else(|| io_error("LIVEKIT_SECRET/LIVEKIT_API_SECRET is missing"))?;

    let livekit_config = LiveKitConfig {
        url: livekit_url.clone(),
        api_key,
        api_secret,
        room: args.room.clone(),
        identity: args.identity.clone(),
        audio: audio_config.clone(),
        reconnect: !args.no_reconnect,
    };

    let _runtime_lock = acquire_runtime_instance_lock(&args.room, &args.identity)?;
    let state = Arc::new(State::default());
    let mut capture = start_capture(
        &host,
        capture_config,
        Arc::clone(&state),
        args.ring_buffer_seconds,
    )?;
    let capture_guard_kind = capture.guard.kind().to_string();

    eprintln!("NAB Live source: {}", capture.source_label);
    eprintln!(
        "Publish: {} room={} identity={}",
        livekit_url, args.room, args.identity
    );
    let status_file = expand_tilde(&args.status_file);
    eprintln!("Status: {}", status_file.display());

    let mut pump = AudioPump::new(capture.input_rate)?;
    wait_for_initial_buffer(&mut capture.cons, capture.input_rate).await;

    let runtime = RuntimeInfo {
        source_label: capture.source_label.clone(),
        source_kind: capture_guard_kind,
        livekit_url: livekit_url.clone(),
        room: args.room.clone(),
        identity: args.identity.clone(),
        status_file,
        input_rate: capture.input_rate,
        audio_profile: audio_config.profile,
        bitrate: audio_config.bitrate,
        red: audio_config.red,
        dtx: audio_config.dtx,
        livekit_buffer_ms: audio_config.livekit_buffer_ms,
    };

    let use_tui = !args.no_tui && io::stdout().is_terminal();
    let loop_result = run_livekit_supervisor(
        &livekit_config,
        &runtime,
        &mut pump,
        &mut capture.cons,
        &state,
        use_tui,
    )
    .await;

    drop(capture);
    loop_result
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum LoopExit {
    UserQuit,
    Reconnect,
}

async fn run_livekit_supervisor(
    config: &LiveKitConfig,
    runtime: &RuntimeInfo,
    pump: &mut AudioPump,
    cons: &mut ringbuf::Consumer<f32, Arc<HeapRb<f32>>>,
    state: &Arc<State>,
    use_tui: bool,
) -> AppResult<()> {
    let mut attempt: u32 = 0;

    loop {
        state
            .connection_state
            .store(CONNECTION_CONNECTING, Ordering::Relaxed);
        match run_livekit_session(config, runtime, pump, cons, state, use_tui).await {
            Ok(LoopExit::UserQuit) => return Ok(()),
            Ok(LoopExit::Reconnect) if config.reconnect => {
                attempt = 0;
                state.reconnects.fetch_add(1, Ordering::Relaxed);
                attempt = attempt.saturating_add(1);
                let delay = reconnect_delay(attempt);
                eprintln!(
                    "LiveKit disconnected. Reconnecting in {}s...",
                    delay.as_secs()
                );
                if !wait_before_reconnect(delay).await {
                    return Ok(());
                }
            }
            Ok(LoopExit::Reconnect) => {
                return Err(io_error("LiveKit disconnected and --no-reconnect is set").into());
            }
            Err(err) if config.reconnect => {
                state.livekit_errors.fetch_add(1, Ordering::Relaxed);
                state
                    .connection_state
                    .store(CONNECTION_DISCONNECTED, Ordering::Relaxed);
                attempt = attempt.saturating_add(1);
                let delay = reconnect_delay(attempt);
                eprintln!(
                    "LiveKit error: {err}. Reconnecting in {}s...",
                    delay.as_secs()
                );
                if !wait_before_reconnect(delay).await {
                    return Ok(());
                }
            }
            Err(err) => return Err(err),
        }
    }
}

async fn run_livekit_session(
    config: &LiveKitConfig,
    runtime: &RuntimeInfo,
    pump: &mut AudioPump,
    cons: &mut ringbuf::Consumer<f32, Arc<HeapRb<f32>>>,
    state: &Arc<State>,
    use_tui: bool,
) -> AppResult<LoopExit> {
    let token = AccessToken::with_api_key(&config.api_key, &config.api_secret)
        .with_identity(&config.identity)
        .with_name(&config.identity)
        .with_grants(VideoGrants {
            room_join: true,
            room: config.room.clone(),
            can_publish: true,
            can_subscribe: false,
            ..Default::default()
        })
        .to_jwt()?;

    let mut room_options = RoomOptions::default();
    room_options.auto_subscribe = false;
    let (room, mut events) = Room::connect(&config.url, &token, room_options).await?;

    let native_source = NativeAudioSource::new(
        AudioSourceOptions::default(),
        OUT_RATE,
        OUT_CHANNELS as u32,
        config.audio.livekit_buffer_ms,
    );
    let track = LocalAudioTrack::create_audio_track(
        "reaper-master",
        RtcAudioSource::Native(native_source.clone()),
    );
    let publication = room
        .local_participant()
        .publish_track(
            LocalTrack::Audio(track),
            TrackPublishOptions {
                audio_encoding: Some(AudioEncoding {
                    max_bitrate: config.audio.bitrate,
                }),
                dtx: config.audio.dtx,
                red: config.audio.red,
                source: TrackSource::Microphone,
                ..Default::default()
            },
        )
        .await?;
    let publication_sid = publication.sid().clone();
    state
        .connection_state
        .store(CONNECTION_CONNECTED, Ordering::Relaxed);
    eprintln!("Published track SID: {}", publication.sid());
    let _ = write_status_file(state, runtime);

    let loop_result = if use_tui {
        run_status_tui(runtime, pump, cons, &native_source, state, &mut events).await
    } else {
        run_plain_loop(runtime, pump, cons, &native_source, state, &mut events).await
    };

    let _ = room
        .local_participant()
        .unpublish_track(&publication_sid)
        .await;
    let _ = room.close().await;
    loop_result
}

fn reconnect_delay(attempt: u32) -> Duration {
    Duration::from_secs(match attempt {
        0 | 1 => 1,
        2 => 2,
        3 => 4,
        4 => 8,
        _ => 15,
    })
}

async fn wait_before_reconnect(delay: Duration) -> bool {
    if termination_requested() {
        return false;
    }
    #[cfg(unix)]
    let mut sigterm =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();

    tokio::select! {
        _ = tokio::time::sleep(delay) => true,
        _ = tokio::signal::ctrl_c() => false,
        _ = async {
            #[cfg(unix)]
            {
                if let Some(signal) = sigterm.as_mut() {
                    let _ = signal.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            }
            #[cfg(not(unix))]
            {
                std::future::pending::<()>().await;
            }
        } => false,
    }
}

fn handle_room_event(event: RoomEvent, state: &State) -> bool {
    match event {
        RoomEvent::Disconnected { reason } => {
            state
                .connection_state
                .store(CONNECTION_DISCONNECTED, Ordering::Relaxed);
            log::log(&format!("LiveKit disconnected: {reason:?}"));
            true
        }
        RoomEvent::Reconnecting => {
            state
                .connection_state
                .store(CONNECTION_RECONNECTING, Ordering::Relaxed);
            log::log("LiveKit reconnecting");
            false
        }
        RoomEvent::Reconnected | RoomEvent::Connected { .. } => {
            state
                .connection_state
                .store(CONNECTION_CONNECTED, Ordering::Relaxed);
            log::log("LiveKit connected");
            false
        }
        RoomEvent::ConnectionStateChanged(connection_state) => {
            let value = match connection_state {
                ConnectionState::Disconnected => CONNECTION_DISCONNECTED,
                ConnectionState::Connected => CONNECTION_CONNECTED,
                ConnectionState::Reconnecting => CONNECTION_RECONNECTING,
            };
            state.connection_state.store(value, Ordering::Relaxed);
            log::log(&format!(
                "LiveKit connection state changed: {connection_state:?}"
            ));
            connection_state == ConnectionState::Disconnected
        }
        RoomEvent::TokenRefreshed { .. } => {
            log::log("LiveKit token refreshed");
            false
        }
        other => {
            log::log(&format!("LiveKit event: {other:?}"));
            false
        }
    }
}

fn should_show_wizard(args: &Args) -> bool {
    args.source.is_none() && args.input.is_none() && !args.test_tone
}

fn resolve_audio_send_config(args: &Args) -> AppResult<AudioSendConfig> {
    let (default_bitrate, default_red, default_dtx, default_buffer_ms) =
        profile_defaults(args.profile);
    let bitrate = args.bitrate.unwrap_or(default_bitrate);
    if !(MIN_BITRATE..=MAX_BITRATE).contains(&bitrate) {
        return Err(io_error(format!("bitrate must be {MIN_BITRATE}..={MAX_BITRATE} bps")).into());
    }

    let livekit_buffer_ms = args.livekit_buffer_ms.unwrap_or(default_buffer_ms);
    if !(MIN_LIVEKIT_BUFFER_MS..=MAX_LIVEKIT_BUFFER_MS).contains(&livekit_buffer_ms) {
        return Err(io_error(format!(
            "livekit-buffer-ms must be {MIN_LIVEKIT_BUFFER_MS}..={MAX_LIVEKIT_BUFFER_MS}"
        ))
        .into());
    }

    let dtx = if args.disable_dtx {
        false
    } else {
        default_dtx || args.enable_dtx
    };

    Ok(AudioSendConfig {
        profile: args.profile,
        bitrate,
        red: default_red && !args.disable_red,
        dtx,
        livekit_buffer_ms,
    })
}

fn profile_defaults(profile: AudioProfileArg) -> (u64, bool, bool, u32) {
    match profile {
        AudioProfileArg::StableMusic => (160_000, true, false, 1_200),
        AudioProfileArg::HiFiMusic => (256_000, true, false, 1_000),
        AudioProfileArg::Speech => (64_000, true, true, 800),
        AudioProfileArg::LowBandwidth => (96_000, true, false, 1_500),
        AudioProfileArg::MaxQualityLab => (510_000, false, false, 1_000),
    }
}

fn validate_buffer_seconds(seconds: usize) -> AppResult<()> {
    if seconds == 0 || seconds > MAX_RING_BUFFER_SECONDS {
        return Err(io_error(format!(
            "ring-buffer-seconds must be 1..={MAX_RING_BUFFER_SECONDS}"
        ))
        .into());
    }
    Ok(())
}

fn capture_config_from_args(args: &Args) -> AppResult<CaptureConfig> {
    if args.test_tone || args.source == Some(SourceArg::TestTone) {
        if !(20.0..=20_000.0).contains(&args.test_tone_hz) {
            return Err(io_error("test-tone-hz must be 20..=20000").into());
        }
        if !(-90.0..=0.0).contains(&args.test_tone_dbfs) {
            return Err(io_error("test-tone-dbfs must be -90..=0").into());
        }
        let duration = match args.test_tone_duration {
            Some(seconds) if seconds > 0.0 && seconds <= 24.0 * 60.0 * 60.0 => {
                Some(Duration::from_secs_f64(seconds))
            }
            Some(_) => {
                return Err(io_error("test-tone-duration must be >0 and <=86400 seconds").into());
            }
            None => None,
        };
        return Ok(CaptureConfig::TestTone {
            frequency_hz: args.test_tone_hz,
            level_dbfs: args.test_tone_dbfs,
            duration,
        });
    }

    match args.source.unwrap_or(SourceArg::Coreaudio) {
        SourceArg::Plugin => Ok(CaptureConfig::Plugin {
            socket_path: args.plugin_socket.clone(),
        }),
        SourceArg::TestTone => unreachable!("test tone handled above"),
        SourceArg::Coreaudio => {
            validate_coreaudio_selection(
                args.input_channels,
                args.left_channel,
                args.right_channel,
            )?;
            Ok(CaptureConfig::CoreAudio {
                input: args.input.clone(),
                input_sample_rate: args.input_sample_rate,
                input_channels: args.input_channels,
                left_channel: args.left_channel,
                right_channel: args.right_channel,
            })
        }
    }
}

fn run_wizard(host: &cpal::Host) -> AppResult<Option<CaptureConfig>> {
    let input_devices: Vec<String> = host
        .input_devices()
        .map(|d| d.filter_map(|x| x.name().ok()).collect())
        .unwrap_or_default();

    enable_raw_mode()?;
    if let Err(err) = execute!(io::stdout(), EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(err.into());
    }

    let mut terminal = match Terminal::new(CrosstermBackend::new(io::stdout())) {
        Ok(terminal) => terminal,
        Err(err) => {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            return Err(err.into());
        }
    };

    let result = run_wizard_inner(&mut terminal, &input_devices);
    let cleanup_result = cleanup_terminal(&mut terminal);
    match (result, cleanup_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(err), _) => Err(err),
        (_, Err(err)) => Err(err),
    }
}

fn run_wizard_inner(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    input_devices: &[String],
) -> AppResult<Option<CaptureConfig>> {
    let mut step = WizardStep::SelectSource { cursor: 0 };
    let mut source = SourceArg::Plugin;
    let mut sample_rate = 96_000;
    let mut input: Option<String> = None;

    loop {
        terminal.draw(|f| draw_wizard(f, &step))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        match event::read()? {
            Event::Resize(_, _) => {
                terminal.autoresize()?;
            }
            Event::Key(key) if key.kind == crossterm::event::KeyEventKind::Press => match &mut step
            {
                WizardStep::SelectSource { cursor } => match key.code {
                    KeyCode::Up => *cursor = cursor.saturating_sub(1),
                    KeyCode::Down => *cursor = (*cursor + 1).min(1),
                    KeyCode::Enter => {
                        source = if *cursor == 0 {
                            SourceArg::Plugin
                        } else {
                            SourceArg::Coreaudio
                        };
                        step = if source == SourceArg::Plugin {
                            WizardStep::Confirm
                        } else {
                            WizardStep::SelectSampleRate { cursor: 1 }
                        };
                    }
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
                    _ => {}
                },
                WizardStep::SelectSampleRate { cursor } => match key.code {
                    KeyCode::Up => *cursor = cursor.saturating_sub(1),
                    KeyCode::Down => *cursor = (*cursor + 1).min(SAMPLE_RATES.len() - 1),
                    KeyCode::Enter => {
                        sample_rate = SAMPLE_RATES[*cursor].0;
                        step = WizardStep::SelectInput {
                            devices: input_devices.to_vec(),
                            cursor: 0,
                        };
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },
                WizardStep::SelectInput { devices, cursor } => match key.code {
                    KeyCode::Up => *cursor = cursor.saturating_sub(1),
                    KeyCode::Down => {
                        *cursor = (*cursor + 1).min(devices.len().saturating_sub(1));
                    }
                    KeyCode::Enter => {
                        input = devices.get(*cursor).cloned();
                        step = WizardStep::Confirm;
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },
                WizardStep::Confirm => match key.code {
                    KeyCode::Enter => {
                        return Ok(Some(match source {
                            SourceArg::Plugin => CaptureConfig::Plugin {
                                socket_path: DEFAULT_TAP_SOCKET.to_string(),
                            },
                            SourceArg::Coreaudio => CaptureConfig::CoreAudio {
                                input: input.clone(),
                                input_sample_rate: sample_rate,
                                input_channels: OUT_CHANNELS as u16,
                                left_channel: 1,
                                right_channel: 2,
                            },
                            SourceArg::TestTone => CaptureConfig::TestTone {
                                frequency_hz: 1_000.0,
                                level_dbfs: -18.0,
                                duration: None,
                            },
                        }));
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },
            },
            _ => {}
        }
    }
}

const SAMPLE_RATES: &[(u32, &str)] = &[
    (48_000, "48 kHz"),
    (96_000, "96 kHz"),
    (44_100, "44.1 kHz"),
    (88_200, "88.2 kHz"),
    (176_400, "176.4 kHz"),
    (192_000, "192 kHz"),
];

fn draw_wizard(f: &mut ratatui::Frame, step: &WizardStep) {
    let outer = Block::default()
        .title(" NAB Live Setup ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    f.render_widget(outer, f.area());

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(f.area());

    match step {
        WizardStep::SelectSource { cursor } => {
            f.render_widget(
                Paragraph::new("Select audio source").style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            let options = [
                "REAPER Master Plugin - NAB Tap (recommended)",
                "CoreAudio input - device/channel capture",
            ];
            draw_select_list(f, layout[1], &options, *cursor);
            draw_wizard_help(f, layout[2]);
        }
        WizardStep::SelectSampleRate { cursor } => {
            f.render_widget(
                Paragraph::new("Select CoreAudio input sample rate")
                    .style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            let options: Vec<&str> = SAMPLE_RATES.iter().map(|(_, label)| *label).collect();
            draw_select_list(f, layout[1], &options, *cursor);
            draw_wizard_help(f, layout[2]);
        }
        WizardStep::SelectInput { devices, cursor } => {
            f.render_widget(
                Paragraph::new("Select CoreAudio input device")
                    .style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            let options: Vec<&str> = devices.iter().map(String::as_str).collect();
            draw_select_list(f, layout[1], &options, *cursor);
            draw_wizard_help(f, layout[2]);
        }
        WizardStep::Confirm => {
            f.render_widget(
                Paragraph::new("Ready").style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            f.render_widget(
                Paragraph::new("Press Enter to start. Press Esc to quit.")
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Color::Cyan)),
                    )
                    .style(Style::default().fg(Color::Cyan)),
                layout[1],
            );
            f.render_widget(
                Paragraph::new("Enter Start   Esc Quit")
                    .style(Style::default().fg(Color::DarkGray)),
                layout[2],
            );
        }
    }
}

fn draw_select_list(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    options: &[&str],
    cursor: usize,
) {
    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(i, &label)| {
            let item = ListItem::new(format!("  {label}"));
            if i == cursor {
                item.style(Style::default().fg(Color::Black).bg(Color::Cyan))
            } else {
                item
            }
        })
        .collect();
    let mut state = ListState::default();
    if !options.is_empty() {
        state.select(Some(cursor.min(options.len() - 1)));
    }
    f.render_stateful_widget(List::new(items), area, &mut state);
}

fn draw_wizard_help(f: &mut ratatui::Frame, area: ratatui::layout::Rect) {
    f.render_widget(
        Paragraph::new("Up/Down Select   Enter Confirm   Esc Quit")
            .style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

fn cleanup_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> AppResult<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

fn start_capture(
    host: &cpal::Host,
    config: CaptureConfig,
    state: Arc<State>,
    ring_buffer_seconds: usize,
) -> AppResult<CaptureSetup> {
    match config {
        CaptureConfig::Plugin { socket_path } => {
            start_plugin_capture(&socket_path, state, ring_buffer_seconds)
        }
        CaptureConfig::TestTone {
            frequency_hz,
            level_dbfs,
            duration,
        } => start_test_tone_capture(
            frequency_hz,
            level_dbfs,
            duration,
            state,
            ring_buffer_seconds,
        ),
        CaptureConfig::CoreAudio {
            input,
            input_sample_rate,
            input_channels,
            left_channel,
            right_channel,
        } => start_coreaudio_capture(
            host,
            CoreAudioCaptureOptions {
                input,
                input_sample_rate,
                input_channels,
                left_channel,
                right_channel,
            },
            state,
            ring_buffer_seconds,
        ),
    }
}

#[cfg(unix)]
fn start_plugin_capture(
    socket_path: &str,
    state: Arc<State>,
    ring_buffer_seconds: usize,
) -> AppResult<CaptureSetup> {
    let socket_path = prepare_tap_socket_path(socket_path)?;
    let lock = acquire_plugin_instance_lock(&socket_path)?;
    remove_stale_tap_socket(&socket_path)?;
    let socket = UnixDatagram::bind(&socket_path)?;
    configure_tap_socket(socket.as_raw_fd());
    socket.set_read_timeout(Some(Duration::from_millis(100)))?;

    eprintln!(
        "Waiting for NAB Tap plugin at {} ...",
        socket_path.display()
    );
    let mut last_wait_notice = Instant::now();

    let mut buf = vec![0u8; TAP_MAX_PACKET_BYTES];
    let (first_packet_bytes, first_packet) = loop {
        if termination_requested() {
            let _ = fs::remove_file(&socket_path);
            return Err(io_error("shutdown requested before first NAB Tap packet").into());
        }
        match socket.recv(&mut buf) {
            Ok(amt) => match parse_tap_packet(&buf[..amt]) {
                Ok(packet) => {
                    let bytes = buf[..amt].to_vec();
                    break (bytes, packet.into_owned());
                }
                Err(err) => {
                    state.capture_errors.fetch_add(1, Ordering::Relaxed);
                    log::log(&format!("tap packet ignored: {err}"));
                }
            },
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::TimedOut =>
            {
                if last_wait_notice.elapsed() >= Duration::from_secs(5) {
                    eprintln!(
                        "Still waiting for NAB Tap audio. Start REAPER playback/monitoring and make sure NAB Tap is inserted on Master FX or Monitor FX."
                    );
                    last_wait_notice = Instant::now();
                }
                continue;
            }
            Err(err) => return Err(err.into()),
        }
    };

    let input_rate = first_packet.sample_rate;
    let ring_capacity =
        input_rate.max(OUT_RATE) as usize * OUT_CHANNELS * ring_buffer_seconds.max(1);
    let rb = HeapRb::<f32>::new(ring_capacity);
    let (mut prod, cons) = rb.split();
    let mut scratch = vec![0.0f32; TAP_MAX_PACKET_BYTES / std::mem::size_of::<f32>()];
    push_tap_packet(&first_packet, &mut prod, &state, &mut scratch);

    let running = Arc::new(AtomicBool::new(true));
    let running_thread = Arc::clone(&running);
    let state_thread = Arc::clone(&state);
    let handle = thread::spawn(move || {
        let mut expected_seq = first_packet.seq.wrapping_add(1);
        drop(first_packet_bytes);
        let mut recv_buf = vec![0u8; TAP_MAX_PACKET_BYTES];
        let mut scratch = scratch;
        while running_thread.load(Ordering::Relaxed) {
            if termination_requested() {
                break;
            }
            match socket.recv(&mut recv_buf) {
                Ok(amt) => match parse_tap_packet(&recv_buf[..amt]) {
                    Ok(packet) => {
                        if packet.sample_rate != input_rate {
                            state_thread.capture_errors.fetch_add(1, Ordering::Relaxed);
                            log::log(&format!(
                                "tap packet ignored: sample rate changed from {input_rate} to {}",
                                packet.sample_rate
                            ));
                            continue;
                        }
                        if packet.seq != expected_seq {
                            state_thread.tap_seq_gaps.fetch_add(1, Ordering::Relaxed);
                        }
                        expected_seq = packet.seq.wrapping_add(1);
                        push_tap_packet(&packet, &mut prod, &state_thread, &mut scratch);
                    }
                    Err(err) => {
                        state_thread.capture_errors.fetch_add(1, Ordering::Relaxed);
                        log::log(&format!("tap packet ignored: {err}"));
                    }
                },
                Err(err)
                    if err.kind() == io::ErrorKind::WouldBlock
                        || err.kind() == io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(err) => {
                    state_thread.capture_errors.fetch_add(1, Ordering::Relaxed);
                    log::log(&format!("tap socket error: {err}"));
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });

    Ok(CaptureSetup {
        input_rate,
        source_label: format!("NAB Tap plugin ({input_rate} Hz)"),
        guard: CaptureGuard::Plugin(PluginCaptureGuard {
            running,
            handle: Some(handle),
            socket_path,
            _lock: lock,
        }),
        cons,
    })
}

#[cfg(unix)]
fn configure_tap_socket(fd: std::os::unix::io::RawFd) {
    let size: libc::c_int = 4 * 1024 * 1024;
    unsafe {
        let _ = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            &size as *const _ as *const libc::c_void,
            std::mem::size_of_val(&size) as libc::socklen_t,
        );
    }
}

#[cfg(not(unix))]
fn start_plugin_capture(
    _socket_path: &str,
    _state: Arc<State>,
    _ring_buffer_seconds: usize,
) -> AppResult<CaptureSetup> {
    Err(io_error("NAB Tap plugin IPC is only supported on macOS/Linux").into())
}

#[cfg(unix)]
fn prepare_tap_socket_path(socket_path: &str) -> AppResult<PathBuf> {
    let socket_path = expand_tilde(socket_path);
    let parent = socket_path
        .parent()
        .ok_or_else(|| io_error("plugin socket path must include a parent directory"))?;
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;

    Ok(socket_path)
}

#[cfg(unix)]
fn acquire_plugin_instance_lock(socket_path: &PathBuf) -> AppResult<PluginInstanceLock> {
    let lock_path = PathBuf::from(format!("{}.lock", socket_path.display()));
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)?;

    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result != 0 {
        let err = io::Error::last_os_error();
        if matches!(
            err.raw_os_error(),
            Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN
        ) {
            return Err(io_error(format!(
                "NAB Live Sender is already running for {}. Open NAB Live Status.command instead of starting another sender.",
                socket_path.display()
            ))
            .into());
        }
        return Err(err.into());
    }

    file.set_len(0)?;
    writeln!(file, "pid={}", std::process::id())?;
    writeln!(file, "socket={}", socket_path.display())?;
    Ok(PluginInstanceLock {
        _file: file,
        path: lock_path,
    })
}

#[cfg(unix)]
fn remove_stale_tap_socket(socket_path: &PathBuf) -> AppResult<()> {
    match fs::symlink_metadata(socket_path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(socket_path)?;
        }
        Ok(_) => {
            return Err(io_error(format!(
                "refusing to replace non-socket file at {}",
                socket_path.display()
            ))
            .into());
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    Ok(())
}

fn start_coreaudio_capture(
    host: &cpal::Host,
    options: CoreAudioCaptureOptions,
    state: Arc<State>,
    ring_buffer_seconds: usize,
) -> AppResult<CaptureSetup> {
    let CoreAudioCaptureOptions {
        input,
        input_sample_rate,
        input_channels,
        left_channel,
        right_channel,
    } = options;
    validate_coreaudio_selection(input_channels, left_channel, right_channel)?;
    let device = select_input_device(host, input.as_deref())?;
    let device_name = device_name(&device);
    ensure_input_support(&device, input_sample_rate, input_channels)?;

    let ring_capacity =
        input_sample_rate.max(OUT_RATE) as usize * OUT_CHANNELS * ring_buffer_seconds.max(1);
    let rb = HeapRb::<f32>::new(ring_capacity);
    let (mut prod, cons) = rb.split();

    let left_index = left_channel - 1;
    let right_index = right_channel - 1;
    let input_channels_usize = input_channels as usize;
    let state_cb = Arc::clone(&state);
    let input_stream = device.build_input_stream(
        &cpal::StreamConfig {
            channels: input_channels,
            sample_rate: cpal::SampleRate(input_sample_rate),
            buffer_size: cpal::BufferSize::Default,
        },
        move |data: &[f32], _| {
            let mut peak_l = 0.0f32;
            let mut peak_r = 0.0f32;
            let mut sum_l = 0.0f64;
            let mut sum_r = 0.0f64;
            let mut frames_seen = 0usize;
            for frame in data.chunks(input_channels_usize) {
                if frame.len() <= left_index || frame.len() <= right_index {
                    state_cb.capture_errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let left = frame[left_index];
                let right = frame[right_index];
                peak_l = peak_l.max(left.abs());
                peak_r = peak_r.max(right.abs());
                sum_l += (left as f64) * (left as f64);
                sum_r += (right as f64) * (right as f64);
                frames_seen += 1;
                if prod.free_len() >= OUT_CHANNELS {
                    let pair = [left, right];
                    let pushed = prod.push_slice(&pair);
                    if pushed == OUT_CHANNELS {
                        state_cb.captured_frames.fetch_add(1, Ordering::Relaxed);
                    } else {
                        state_cb.overflow_frames.fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    state_cb.overflow_frames.fetch_add(1, Ordering::Relaxed);
                }
            }
            if frames_seen > 0 {
                store_levels(
                    &state_cb,
                    peak_l,
                    peak_r,
                    (sum_l / frames_seen as f64).sqrt() as f32,
                    (sum_r / frames_seen as f64).sqrt() as f32,
                );
            }
        },
        {
            let state_err = Arc::clone(&state);
            move |err| {
                state_err.capture_errors.fetch_add(1, Ordering::Relaxed);
                log::log(&format!("input stream error: {err}"));
            }
        },
        None,
    )?;
    input_stream.play()?;

    Ok(CaptureSetup {
        input_rate: input_sample_rate,
        source_label: format!(
            "{} / {} Hz / {}ch / L{} R{}",
            device_name, input_sample_rate, input_channels, left_channel, right_channel
        ),
        guard: CaptureGuard::CoreAudio(input_stream),
        cons,
    })
}

fn start_test_tone_capture(
    frequency_hz: f32,
    level_dbfs: f32,
    duration: Option<Duration>,
    state: Arc<State>,
    ring_buffer_seconds: usize,
) -> AppResult<CaptureSetup> {
    let input_rate = OUT_RATE;
    let ring_capacity = input_rate as usize * OUT_CHANNELS * ring_buffer_seconds.max(1);
    let rb = HeapRb::<f32>::new(ring_capacity);
    let (mut prod, cons) = rb.split();
    let running = Arc::new(AtomicBool::new(true));
    let running_thread = Arc::clone(&running);
    let state_thread = Arc::clone(&state);
    let amplitude = 10.0f32.powf(level_dbfs / 20.0);
    let frames_per_chunk = (input_rate as usize * OUT_FRAME_MS as usize) / 1000;
    let sleep_duration = Duration::from_millis(OUT_FRAME_MS as u64);

    let handle = thread::spawn(move || {
        let started = Instant::now();
        let mut frame_index: u64 = 0;
        let mut chunk = vec![0.0f32; frames_per_chunk * OUT_CHANNELS];
        while running_thread.load(Ordering::Relaxed) {
            if duration.is_some_and(|limit| started.elapsed() >= limit) {
                thread::sleep(Duration::from_millis(50));
                continue;
            }

            let mut peak_l = 0.0f32;
            let mut peak_r = 0.0f32;
            let mut sum_l = 0.0f64;
            let mut sum_r = 0.0f64;
            for frame in 0..frames_per_chunk {
                let t = frame_index as f32 / input_rate as f32;
                let sample = amplitude * (std::f32::consts::TAU * frequency_hz * t).sin();
                chunk[frame * OUT_CHANNELS] = sample;
                chunk[frame * OUT_CHANNELS + 1] = sample;
                peak_l = peak_l.max(sample.abs());
                peak_r = peak_r.max(sample.abs());
                sum_l += (sample as f64) * (sample as f64);
                sum_r += (sample as f64) * (sample as f64);
                frame_index = frame_index.wrapping_add(1);
            }

            if prod.free_len() >= chunk.len() {
                let pushed = prod.push_slice(&chunk);
                state_thread
                    .captured_frames
                    .fetch_add((pushed / OUT_CHANNELS) as u64, Ordering::Relaxed);
                if pushed != chunk.len() {
                    state_thread.overflow_frames.fetch_add(
                        ((chunk.len() - pushed) / OUT_CHANNELS) as u64,
                        Ordering::Relaxed,
                    );
                }
            } else {
                state_thread
                    .overflow_frames
                    .fetch_add(frames_per_chunk as u64, Ordering::Relaxed);
            }
            store_levels(
                &state_thread,
                peak_l,
                peak_r,
                (sum_l / frames_per_chunk as f64).sqrt() as f32,
                (sum_r / frames_per_chunk as f64).sqrt() as f32,
            );
            thread::sleep(sleep_duration);
        }
    });

    let duration_label = duration
        .map(|d| format!(", {:.1}s", d.as_secs_f64()))
        .unwrap_or_else(|| ", continuous".to_string());

    Ok(CaptureSetup {
        input_rate,
        source_label: format!(
            "Internal test tone ({:.0} Hz, {:.1} dBFS{duration_label})",
            frequency_hz, level_dbfs
        ),
        guard: CaptureGuard::TestTone(TestToneGuard {
            running,
            handle: Some(handle),
        }),
        cons,
    })
}

fn acquire_runtime_instance_lock(room: &str, identity: &str) -> AppResult<RuntimeInstanceLock> {
    let lock_dir = expand_tilde("~/.nab/locks");
    fs::create_dir_all(&lock_dir)?;
    fs::set_permissions(&lock_dir, fs::Permissions::from_mode(0o700))?;
    let lock_path = lock_dir.join(format!(
        "nab-live-{}-{}.lock",
        safe_lock_component(room),
        safe_lock_component(identity)
    ));
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)?;

    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result != 0 {
        let err = io::Error::last_os_error();
        if matches!(
            err.raw_os_error(),
            Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN
        ) {
            return Err(io_error(format!(
                "NAB Live is already running for room={room} identity={identity}. Refusing duplicate LiveKit identity."
            ))
            .into());
        }
        return Err(err.into());
    }

    file.set_len(0)?;
    writeln!(file, "pid={}", std::process::id())?;
    writeln!(file, "room={room}")?;
    writeln!(file, "identity={identity}")?;
    Ok(RuntimeInstanceLock {
        _file: file,
        path: lock_path,
    })
}

fn safe_lock_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len().max(1));
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "default".to_string()
    } else {
        out
    }
}

#[derive(Clone)]
struct TapPacket<'a> {
    sample_rate: u32,
    channels: usize,
    frames: usize,
    seq: u64,
    dropped_frames: u64,
    payload: std::borrow::Cow<'a, [u8]>,
}

impl<'a> TapPacket<'a> {
    fn into_owned(self) -> TapPacket<'static> {
        TapPacket {
            sample_rate: self.sample_rate,
            channels: self.channels,
            frames: self.frames,
            seq: self.seq,
            dropped_frames: self.dropped_frames,
            payload: std::borrow::Cow::Owned(self.payload.into_owned()),
        }
    }
}

fn parse_tap_packet(buf: &[u8]) -> Result<TapPacket<'_>, String> {
    if buf.len() < TAP_HEADER_BYTES {
        return Err("packet too short".to_string());
    }
    if &buf[0..8] != TAP_MAGIC {
        return Err("bad tap magic".to_string());
    }
    let version = LittleEndian::read_u32(&buf[8..12]);
    if version != TAP_VERSION {
        return Err(format!("unsupported tap version {version}"));
    }
    let sample_rate = LittleEndian::read_u32(&buf[12..16]);
    let channels = LittleEndian::read_u16(&buf[16..18]) as usize;
    let frames = LittleEndian::read_u16(&buf[18..20]) as usize;
    let seq = LittleEndian::read_u64(&buf[20..28]);
    let dropped_frames = LittleEndian::read_u64(&buf[28..36]);

    if !(8_000..=384_000).contains(&sample_rate) {
        return Err(format!("invalid sample rate {sample_rate}"));
    }
    if channels == 0 || channels > 64 {
        return Err(format!("invalid channel count {channels}"));
    }
    if frames == 0 {
        return Err("empty packet".to_string());
    }

    let needed = TAP_HEADER_BYTES + frames * channels * std::mem::size_of::<f32>();
    if buf.len() < needed {
        return Err(format!("short payload got={} need={}", buf.len(), needed));
    }

    Ok(TapPacket {
        sample_rate,
        channels,
        frames,
        seq,
        dropped_frames,
        payload: std::borrow::Cow::Borrowed(&buf[TAP_HEADER_BYTES..needed]),
    })
}

fn push_tap_packet(
    packet: &TapPacket<'_>,
    prod: &mut ringbuf::Producer<f32, Arc<HeapRb<f32>>>,
    state: &State,
    scratch: &mut [f32],
) {
    let needed_samples = packet.frames * OUT_CHANNELS;
    if scratch.len() < needed_samples {
        state
            .overflow_frames
            .fetch_add(packet.frames as u64, Ordering::Relaxed);
        state.capture_errors.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if prod.free_len() < needed_samples {
        state
            .overflow_frames
            .fetch_add(packet.frames as u64, Ordering::Relaxed);
        return;
    }

    let mut peak_l = 0.0f32;
    let mut peak_r = 0.0f32;
    let mut sum_l = 0.0f64;
    let mut sum_r = 0.0f64;
    for frame in 0..packet.frames {
        let base = frame * packet.channels * std::mem::size_of::<f32>();
        let left = LittleEndian::read_f32(&packet.payload[base..base + 4]);
        let right = if packet.channels > 1 {
            LittleEndian::read_f32(&packet.payload[base + 4..base + 8])
        } else {
            left
        };
        peak_l = peak_l.max(left.abs());
        peak_r = peak_r.max(right.abs());
        sum_l += (left as f64) * (left as f64);
        sum_r += (right as f64) * (right as f64);
        scratch[frame * OUT_CHANNELS] = left;
        scratch[frame * OUT_CHANNELS + 1] = right;
    }

    let converted = &scratch[..needed_samples];
    let pushed = prod.push_slice(converted);
    state
        .captured_frames
        .fetch_add((pushed / OUT_CHANNELS) as u64, Ordering::Relaxed);
    if pushed != converted.len() {
        state.overflow_frames.fetch_add(
            ((converted.len() - pushed) / OUT_CHANNELS) as u64,
            Ordering::Relaxed,
        );
    }
    state.tap_packets.fetch_add(1, Ordering::Relaxed);
    state
        .tap_reported_drops
        .store(packet.dropped_frames, Ordering::Relaxed);
    store_levels(
        state,
        peak_l,
        peak_r,
        (sum_l / packet.frames as f64).sqrt() as f32,
        (sum_r / packet.frames as f64).sqrt() as f32,
    );
}

fn store_levels(state: &State, peak_l: f32, peak_r: f32, rms_l: f32, rms_r: f32) {
    state.peak_l_milli.store(
        ((peak_l.clamp(0.0, 1.0) * 1000.0).round() as usize).min(1000),
        Ordering::Relaxed,
    );
    state.peak_r_milli.store(
        ((peak_r.clamp(0.0, 1.0) * 1000.0).round() as usize).min(1000),
        Ordering::Relaxed,
    );
    state.rms_l_milli.store(
        ((rms_l.clamp(0.0, 1.0) * 1000.0).round() as usize).min(1000),
        Ordering::Relaxed,
    );
    state.rms_r_milli.store(
        ((rms_r.clamp(0.0, 1.0) * 1000.0).round() as usize).min(1000),
        Ordering::Relaxed,
    );
    state
        .last_audio_unix_ms
        .store(now_unix_ms() as u64, Ordering::Relaxed);
}

struct AudioPump {
    input_rate: u32,
    input_frames_per_chunk: usize,
    input_interleaved: Vec<f32>,
    input_planar: Vec<Vec<f32>>,
    output_planar: Vec<Vec<f32>>,
    output_interleaved_i16: Vec<i16>,
    resampler: Option<FftFixedInOut<f32>>,
}

impl AudioPump {
    fn new(input_rate: u32) -> AppResult<Self> {
        let mut resampler = if input_rate == OUT_RATE {
            None
        } else {
            Some(FftFixedInOut::<f32>::new(
                input_rate as usize,
                OUT_RATE as usize,
                (input_rate as usize * OUT_FRAME_MS as usize) / 1000,
                OUT_CHANNELS,
            )?)
        };

        let input_frames_per_chunk = match resampler.as_mut() {
            Some(r) => r.input_frames_next(),
            None => OUT_FRAMES_PER_PACKET,
        };
        let output_frames_per_chunk = match resampler.as_ref() {
            Some(r) => r.output_frames_next(),
            None => OUT_FRAMES_PER_PACKET,
        };

        Ok(Self {
            input_rate,
            input_frames_per_chunk,
            input_interleaved: vec![0.0; input_frames_per_chunk * OUT_CHANNELS],
            input_planar: vec![vec![0.0; input_frames_per_chunk]; OUT_CHANNELS],
            output_planar: vec![vec![0.0; output_frames_per_chunk]; OUT_CHANNELS],
            output_interleaved_i16: vec![0; output_frames_per_chunk * OUT_CHANNELS],
            resampler,
        })
    }

    async fn send_next_frame(
        &mut self,
        cons: &mut ringbuf::Consumer<f32, Arc<HeapRb<f32>>>,
        source: &NativeAudioSource,
        state: &State,
    ) -> AppResult<()> {
        let needed_samples = self.input_frames_per_chunk * OUT_CHANNELS;
        let buffered_samples = cons.len();
        let buffered_ms = (buffered_samples / OUT_CHANNELS) * 1000 / self.input_rate as usize;
        state.ring_buffer_ms.store(buffered_ms, Ordering::Relaxed);

        if buffered_samples < needed_samples {
            tokio::time::sleep(Duration::from_millis(1)).await;
            return Ok(());
        }

        let popped = cons.pop_slice(&mut self.input_interleaved);
        if popped != needed_samples {
            state.underruns.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(1)).await;
            return Ok(());
        }

        let output_frames = if let Some(resampler) = self.resampler.as_mut() {
            for frame in 0..self.input_frames_per_chunk {
                self.input_planar[0][frame] = self.input_interleaved[frame * OUT_CHANNELS];
                self.input_planar[1][frame] = self.input_interleaved[frame * OUT_CHANNELS + 1];
            }
            let (_, out_frames) =
                resampler.process_into_buffer(&self.input_planar, &mut self.output_planar, None)?;
            out_frames
        } else {
            for frame in 0..OUT_FRAMES_PER_PACKET {
                self.output_planar[0][frame] = self.input_interleaved[frame * OUT_CHANNELS];
                self.output_planar[1][frame] = self.input_interleaved[frame * OUT_CHANNELS + 1];
            }
            OUT_FRAMES_PER_PACKET
        };

        for frame in 0..output_frames {
            self.output_interleaved_i16[frame * OUT_CHANNELS] =
                f32_to_i16(self.output_planar[0][frame]);
            self.output_interleaved_i16[frame * OUT_CHANNELS + 1] =
                f32_to_i16(self.output_planar[1][frame]);
        }

        let sample_count = output_frames * OUT_CHANNELS;
        let frame = AudioFrame {
            data: self.output_interleaved_i16[..sample_count].into(),
            sample_rate: OUT_RATE,
            num_channels: OUT_CHANNELS as u32,
            samples_per_channel: output_frames as u32,
        };
        source.capture_frame(&frame).await?;
        state
            .sent_frames
            .fetch_add(output_frames as u64, Ordering::Relaxed);
        Ok(())
    }
}

async fn run_plain_loop(
    runtime: &RuntimeInfo,
    pump: &mut AudioPump,
    cons: &mut ringbuf::Consumer<f32, Arc<HeapRb<f32>>>,
    native_source: &NativeAudioSource,
    state: &Arc<State>,
    events: &mut UnboundedReceiver<RoomEvent>,
) -> AppResult<LoopExit> {
    let mut status_tick = tokio::time::interval(Duration::from_secs(2));
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    #[cfg(unix)]
    let terminate = sigterm.recv();
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::pin!(terminate);

    loop {
        if termination_requested() {
            eprintln!("Stopping NAB Live after SIGTERM...");
            return Ok(LoopExit::UserQuit);
        }
        tokio::select! {
            _ = &mut ctrl_c => {
                eprintln!("Stopping NAB Live...");
                return Ok(LoopExit::UserQuit);
            }
            _ = &mut terminate => {
                eprintln!("Stopping NAB Live after SIGTERM...");
                return Ok(LoopExit::UserQuit);
            }
            event = events.recv() => {
                let Some(event) = event else {
                    return Ok(LoopExit::Reconnect);
                };
                if handle_room_event(event, state) {
                    return Ok(LoopExit::Reconnect);
                }
            }
            _ = status_tick.tick() => {
                print_status(state, runtime);
            }
            result = pump.send_next_frame(cons, native_source, state) => {
                result?;
            }
        }
    }
}

async fn run_status_tui(
    runtime: &RuntimeInfo,
    pump: &mut AudioPump,
    cons: &mut ringbuf::Consumer<f32, Arc<HeapRb<f32>>>,
    native_source: &NativeAudioSource,
    state: &Arc<State>,
    events: &mut UnboundedReceiver<RoomEvent>,
) -> AppResult<LoopExit> {
    enable_raw_mode()?;
    if let Err(err) = execute!(io::stdout(), EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(err.into());
    }
    let mut terminal = match Terminal::new(CrosstermBackend::new(io::stdout())) {
        Ok(terminal) => terminal,
        Err(err) => {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            return Err(err.into());
        }
    };

    let result = run_status_tui_inner(
        &mut terminal,
        runtime,
        pump,
        cons,
        native_source,
        state,
        events,
    )
    .await;
    let cleanup_result = cleanup_terminal(&mut terminal);
    match (result, cleanup_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(err), _) => Err(err),
        (_, Err(err)) => Err(err),
    }
}

async fn run_status_tui_inner(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    runtime: &RuntimeInfo,
    pump: &mut AudioPump,
    cons: &mut ringbuf::Consumer<f32, Arc<HeapRb<f32>>>,
    native_source: &NativeAudioSource,
    state: &Arc<State>,
    events: &mut UnboundedReceiver<RoomEvent>,
) -> AppResult<LoopExit> {
    let mut draw_tick = tokio::time::interval(Duration::from_millis(100));
    let mut status_tick = tokio::time::interval(Duration::from_secs(1));
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    #[cfg(unix)]
    let terminate = sigterm.recv();
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::pin!(terminate);
    let mut quit_pending = false;
    let mut quit_time = Instant::now();

    loop {
        if termination_requested() {
            return Ok(LoopExit::UserQuit);
        }
        tokio::select! {
            _ = &mut ctrl_c => return Ok(LoopExit::UserQuit),
            _ = &mut terminate => return Ok(LoopExit::UserQuit),
            event = events.recv() => {
                let Some(event) = event else {
                    return Ok(LoopExit::Reconnect);
                };
                if handle_room_event(event, state) {
                    return Ok(LoopExit::Reconnect);
                }
            }
            _ = draw_tick.tick() => {
                if quit_pending && quit_time.elapsed() > Duration::from_secs(1) {
                    quit_pending = false;
                }
                terminal.draw(|f| draw_status(f, runtime, state, quit_pending))?;
                while event::poll(Duration::from_millis(0))? {
                    match event::read()? {
                        Event::Key(key) if key.kind == crossterm::event::KeyEventKind::Press => {
                            match key.code {
                                KeyCode::Char('q') | KeyCode::Esc => {
                                    if quit_pending {
                                        return Ok(LoopExit::UserQuit);
                                    }
                                    quit_pending = true;
                                    quit_time = Instant::now();
                                }
                                _ => {}
                            }
                        }
                        Event::Resize(_, _) => terminal.autoresize()?,
                        _ => {}
                    }
                }
            }
            _ = status_tick.tick() => {
                let _ = write_status_file(state, runtime);
            }
            result = pump.send_next_frame(cons, native_source, state) => {
                result?;
            }
        }
    }
}

fn draw_status(f: &mut ratatui::Frame, runtime: &RuntimeInfo, state: &State, quit_pending: bool) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(f.area());

    f.render_widget(
        Paragraph::new(format!(
            " {}  room={}  identity={}  {}",
            runtime.livekit_url,
            runtime.room,
            runtime.identity,
            connection_label(state.connection_state.load(Ordering::Relaxed))
        ))
        .block(
            Block::default()
                .title("NAB Live")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .style(Style::default().fg(Color::Cyan)),
        rows[0],
    );

    f.render_widget(
        Paragraph::new(format!(
            " Source : {}\n Output : 48 kHz stereo Opus/WebRTC\n Profile: {}  bitrate={} kbps  RED={}  DTX={}  queue={} ms",
            runtime.source_label,
            profile_label(runtime.audio_profile),
            runtime.bitrate / 1000,
            on_off(runtime.red),
            on_off(runtime.dtx),
            runtime.livekit_buffer_ms
        ))
        .block(Block::default().title("Signal").borders(Borders::ALL)),
        rows[1],
    );

    let buffer_ms = state.ring_buffer_ms.load(Ordering::Relaxed);
    let buffer_pct = ((buffer_ms * 100) / 1000).min(100) as u16;
    f.render_widget(
        Gauge::default()
            .block(Block::default().title("Local Buffer").borders(Borders::ALL))
            .gauge_style(Style::default().fg(Color::Cyan))
            .percent(buffer_pct)
            .label(format!("{buffer_ms} ms")),
        rows[2],
    );

    let peak_l = state.peak_l_milli.load(Ordering::Relaxed);
    let peak_r = state.peak_r_milli.load(Ordering::Relaxed);
    let peak_pct = ((peak_l.max(peak_r) * 100) / 1000).min(100) as u16;
    f.render_widget(
        Gauge::default()
            .block(Block::default().title("Input Level").borders(Borders::ALL))
            .gauge_style(Style::default().fg(if peak_pct >= 98 {
                Color::Red
            } else {
                Color::Green
            }))
            .percent(peak_pct)
            .label(format!(
                "L {:.3}  R {:.3}",
                peak_l as f32 / 1000.0,
                peak_r as f32 / 1000.0
            )),
        rows[3],
    );

    let captured = state.captured_frames.load(Ordering::Relaxed);
    let sent = state.sent_frames.load(Ordering::Relaxed);
    let overflow = state.overflow_frames.load(Ordering::Relaxed);
    let underruns = state.underruns.load(Ordering::Relaxed);
    let errors = state.capture_errors.load(Ordering::Relaxed);
    let livekit_errors = state.livekit_errors.load(Ordering::Relaxed);
    let reconnects = state.reconnects.load(Ordering::Relaxed);
    let tap_packets = state.tap_packets.load(Ordering::Relaxed);
    let tap_gaps = state.tap_seq_gaps.load(Ordering::Relaxed);
    let tap_drops = state.tap_reported_drops.load(Ordering::Relaxed);

    f.render_widget(
        Paragraph::new(format!(
            "{} Hz  captured={}  sent={}  overflow={}  underrun={}  inputErr={}  lkErr={}  reconnect={}",
            runtime.input_rate, captured, sent, overflow, underruns, errors, livekit_errors, reconnects
        ))
        .block(Block::default().title("Frames").borders(Borders::ALL))
        .style(Style::default().fg(if overflow > 0 || errors > 0 || livekit_errors > 0 {
            Color::Red
        } else {
            Color::Cyan
        })),
        rows[4],
    );

    let help = if quit_pending {
        "Tap packets: ".to_string()
            + &format!(
                "{tap_packets}  gaps={tap_gaps}  pluginDrops={tap_drops}   press again to quit"
            )
    } else {
        "Tap packets: ".to_string()
            + &format!("{tap_packets}  gaps={tap_gaps}  pluginDrops={tap_drops}   q Quit")
    };
    f.render_widget(
        Paragraph::new(help)
            .block(Block::default().title("Status").borders(Borders::ALL))
            .style(Style::default().fg(if quit_pending {
                Color::Yellow
            } else {
                Color::DarkGray
            })),
        rows[5],
    );
}

fn profile_label(profile: AudioProfileArg) -> &'static str {
    match profile {
        AudioProfileArg::StableMusic => "stable-music",
        AudioProfileArg::HiFiMusic => "hi-fi-music",
        AudioProfileArg::Speech => "speech",
        AudioProfileArg::LowBandwidth => "low-bandwidth",
        AudioProfileArg::MaxQualityLab => "max-quality-lab",
    }
}

fn connection_label(value: usize) -> &'static str {
    match value {
        CONNECTION_CONNECTED => "Connected",
        CONNECTION_RECONNECTING => "Reconnecting",
        CONNECTION_DISCONNECTED => "Disconnected",
        _ => "Connecting",
    }
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

fn validate_coreaudio_selection(
    input_channels: u16,
    left_channel: usize,
    right_channel: usize,
) -> AppResult<()> {
    if input_channels == 0 {
        return Err(io_error("input_channels must be greater than zero").into());
    }
    if left_channel == 0 || right_channel == 0 {
        return Err(io_error("left_channel/right_channel are 1-based").into());
    }
    if left_channel > input_channels as usize || right_channel > input_channels as usize {
        return Err(io_error("left/right channel must be within input_channels").into());
    }
    Ok(())
}

fn list_input_devices(host: &cpal::Host) -> AppResult<()> {
    let devices = host.input_devices()?;
    for device in devices {
        let name = device_name(&device);
        eprintln!("{name}");
        match device.supported_input_configs() {
            Ok(configs) => {
                for cfg in configs {
                    eprintln!(
                        "  {}ch {:?} {}-{} Hz",
                        cfg.channels(),
                        cfg.sample_format(),
                        cfg.min_sample_rate().0,
                        cfg.max_sample_rate().0
                    );
                }
            }
            Err(err) => eprintln!("  supported config error: {err}"),
        }
    }
    Ok(())
}

fn select_input_device(host: &cpal::Host, selected_name: Option<&str>) -> AppResult<cpal::Device> {
    if let Some(name) = selected_name {
        let wanted = name.to_lowercase();
        for device in host.input_devices()? {
            let dev_name = device_name(&device);
            if dev_name.to_lowercase().contains(&wanted) {
                return Ok(device);
            }
        }
        return Err(io_error(format!("input device matching \"{name}\" not found")).into());
    }

    host.default_input_device()
        .ok_or_else(|| io_error("default input device not found").into())
}

fn ensure_input_support(device: &cpal::Device, sample_rate: u32, channels: u16) -> AppResult<()> {
    let mut supported = false;
    for cfg in device.supported_input_configs()? {
        if cfg.channels() == channels
            && cfg.sample_format() == cpal::SampleFormat::F32
            && cfg.min_sample_rate().0 <= sample_rate
            && cfg.max_sample_rate().0 >= sample_rate
        {
            supported = true;
            break;
        }
    }

    if supported {
        Ok(())
    } else {
        Err(io_error(format!(
            "\"{}\" does not advertise F32 {channels}ch at {sample_rate} Hz. Run --list-devices.",
            device_name(device)
        ))
        .into())
    }
}

async fn wait_for_initial_buffer(
    cons: &mut ringbuf::Consumer<f32, Arc<HeapRb<f32>>>,
    input_rate: u32,
) {
    let target_samples = (input_rate as usize / 10) * OUT_CHANNELS;
    for _ in 0..200 {
        if cons.len() >= target_samples {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn print_status(state: &State, runtime: &RuntimeInfo) {
    let captured = state.captured_frames.load(Ordering::Relaxed);
    let sent = state.sent_frames.load(Ordering::Relaxed);
    let overflow = state.overflow_frames.load(Ordering::Relaxed);
    let underruns = state.underruns.load(Ordering::Relaxed);
    let errors = state.capture_errors.load(Ordering::Relaxed);
    let livekit_errors = state.livekit_errors.load(Ordering::Relaxed);
    let reconnects = state.reconnects.load(Ordering::Relaxed);
    let buffer_ms = state.ring_buffer_ms.load(Ordering::Relaxed);

    eprintln!(
        "[{}] {} profile={} bitrate={}kbps red={} dtx={} queue={}ms in={}Hz buffer={}ms captured={} sent={} overflow={} underrun={} inputErr={} lkErr={} reconnect={}",
        runtime.source_label,
        connection_label(state.connection_state.load(Ordering::Relaxed)),
        profile_label(runtime.audio_profile),
        runtime.bitrate / 1000,
        on_off(runtime.red),
        on_off(runtime.dtx),
        runtime.livekit_buffer_ms,
        runtime.input_rate,
        buffer_ms,
        captured,
        sent,
        overflow,
        underruns,
        errors,
        livekit_errors,
        reconnects
    );
    let _ = write_status_file(state, runtime);
}

fn write_status_file(state: &State, runtime: &RuntimeInfo) -> io::Result<()> {
    if let Some(parent) = runtime.status_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let now_ms = now_unix_ms();
    let connection = connection_label(state.connection_state.load(Ordering::Relaxed));
    let captured = state.captured_frames.load(Ordering::Relaxed);
    let sent = state.sent_frames.load(Ordering::Relaxed);
    let overflow = state.overflow_frames.load(Ordering::Relaxed);
    let underruns = state.underruns.load(Ordering::Relaxed);
    let errors = state.capture_errors.load(Ordering::Relaxed);
    let livekit_errors = state.livekit_errors.load(Ordering::Relaxed);
    let reconnects = state.reconnects.load(Ordering::Relaxed);
    let buffer_ms = state.ring_buffer_ms.load(Ordering::Relaxed);
    let peak_l = state.peak_l_milli.load(Ordering::Relaxed);
    let peak_r = state.peak_r_milli.load(Ordering::Relaxed);
    let rms_l = state.rms_l_milli.load(Ordering::Relaxed);
    let rms_r = state.rms_r_milli.load(Ordering::Relaxed);
    let last_audio_ms = state.last_audio_unix_ms.load(Ordering::Relaxed);
    let last_audio_age_ms: i128 = if last_audio_ms == 0 {
        -1
    } else {
        now_ms as i128 - last_audio_ms as i128
    };
    let audio_state = if last_audio_ms == 0 {
        "no-audio-yet"
    } else if last_audio_age_ms > 1_500 {
        "stalled"
    } else if rms_l.max(rms_r) <= 1 {
        "silence"
    } else {
        "active"
    };
    let tap_packets = state.tap_packets.load(Ordering::Relaxed);
    let tap_gaps = state.tap_seq_gaps.load(Ordering::Relaxed);
    let tap_drops = state.tap_reported_drops.load(Ordering::Relaxed);

    let body = format!(
        concat!(
            "{{\n",
            "  \"updated_at_unix_ms\": {now_ms},\n",
            "  \"connection\": {connection},\n",
            "  \"source\": {source},\n",
            "  \"source_kind\": {source_kind},\n",
            "  \"livekit_url\": {livekit_url},\n",
            "  \"listen_url\": \"https://livekit.kenichi-kawabata.com/\",\n",
            "  \"room\": {room},\n",
            "  \"identity\": {identity},\n",
            "  \"track\": \"reaper-master\",\n",
            "  \"input_rate_hz\": {input_rate},\n",
            "  \"output_rate_hz\": 48000,\n",
            "  \"output\": \"48 kHz stereo WebRTC audio\",\n",
            "  \"profile\": {profile},\n",
            "  \"bitrate_bps\": {bitrate},\n",
            "  \"red_enabled\": {red},\n",
            "  \"dtx_enabled\": {dtx},\n",
            "  \"livekit_queue_ms\": {queue},\n",
            "  \"buffer_ms\": {buffer_ms},\n",
            "  \"captured_frames\": {captured},\n",
            "  \"sent_frames\": {sent},\n",
            "  \"tap_packets\": {tap_packets},\n",
            "  \"tap_sequence_gaps\": {tap_gaps},\n",
            "  \"plugin_reported_drops\": {tap_drops},\n",
            "  \"overflow_frames\": {overflow},\n",
            "  \"underruns\": {underruns},\n",
            "  \"input_errors\": {errors},\n",
            "  \"livekit_errors\": {livekit_errors},\n",
            "  \"reconnects\": {reconnects},\n",
            "  \"peak_left_milli\": {peak_l},\n",
            "  \"peak_right_milli\": {peak_r},\n",
            "  \"rms_left_milli\": {rms_l},\n",
            "  \"rms_right_milli\": {rms_r},\n",
            "  \"last_audio_frame_age_ms\": {last_audio_age_ms},\n",
            "  \"audio_state\": {audio_state}\n",
            "}}\n"
        ),
        now_ms = now_ms,
        connection = json_string(connection),
        source = json_string(&runtime.source_label),
        source_kind = json_string(&runtime.source_kind),
        livekit_url = json_string(&runtime.livekit_url),
        room = json_string(&runtime.room),
        identity = json_string(&runtime.identity),
        input_rate = runtime.input_rate,
        profile = json_string(profile_label(runtime.audio_profile)),
        bitrate = runtime.bitrate,
        red = runtime.red,
        dtx = runtime.dtx,
        queue = runtime.livekit_buffer_ms,
        buffer_ms = buffer_ms,
        captured = captured,
        sent = sent,
        tap_packets = tap_packets,
        tap_gaps = tap_gaps,
        tap_drops = tap_drops,
        overflow = overflow,
        underruns = underruns,
        errors = errors,
        livekit_errors = livekit_errors,
        reconnects = reconnects,
        peak_l = peak_l,
        peak_r = peak_r,
        rms_l = rms_l,
        rms_r = rms_r,
        last_audio_age_ms = last_audio_age_ms,
        audio_state = json_string(audio_state),
    );

    let tmp_path = runtime.status_file.with_extension("json.tmp");
    fs::write(&tmp_path, body)?;
    fs::rename(tmp_path, &runtime.status_file)
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", ch as u32));
            }
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

fn f32_to_i16(sample: f32) -> i16 {
    let clamped = sample.clamp(-1.0, 1.0);
    if clamped < 0.0 {
        (clamped * 32768.0) as i16
    } else {
        (clamped * 32767.0) as i16
    }
}

fn load_env(path: &str) -> HashMap<String, String> {
    let path = expand_tilde(path);
    let Ok(contents) = fs::read_to_string(path) else {
        return HashMap::new();
    };

    contents
        .lines()
        .filter_map(parse_env_line)
        .collect::<HashMap<_, _>>()
}

fn parse_env_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let (key, value) = trimmed.split_once('=')?;
    let key = key.trim().trim_start_matches("export ").to_string();
    let value = value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string();
    Some((key, value))
}

fn read_key(file_env: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Ok(value) = std::env::var(key) {
            if !value.trim().is_empty() {
                return Some(value);
            }
        }
        if let Some(value) = file_env.get(*key) {
            if !value.trim().is_empty() {
                return Some(value.clone());
            }
        }
    }
    None
}

fn normalize_livekit_url(url: String) -> String {
    if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        url
    }
}

fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn device_name(device: &cpal::Device) -> String {
    device
        .name()
        .unwrap_or_else(|_| "Unknown input device".to_string())
}

fn io_error(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

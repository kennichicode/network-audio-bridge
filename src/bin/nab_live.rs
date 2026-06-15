use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use livekit::options::{AudioEncoding, TrackPublishOptions};
use livekit::prelude::*;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::prelude::{AudioFrame, AudioSourceOptions};
use livekit_api::access_token::{AccessToken, VideoGrants};
use ringbuf::HeapRb;
use rubato::{FftFixedInOut, Resampler};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[path = "../log.rs"]
mod log;

const OUT_RATE: u32 = 48_000;
const OUT_CHANNELS: usize = 2;
const OUT_FRAME_MS: u32 = 10;
const OUT_FRAMES_PER_PACKET: usize = (OUT_RATE as usize * OUT_FRAME_MS as usize) / 1000;

type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Parser, Debug)]
#[command(name = "nab-live")]
#[command(about = "Capture a CoreAudio stereo input and publish it to LiveKit/WebRTC.")]
struct Args {
    /// List input devices and exit.
    #[arg(long)]
    list_devices: bool,

    /// Input device name substring. If omitted, the default input device is used.
    #[arg(long)]
    input: Option<String>,

    /// CoreAudio input sample rate. Use 96000 for a 96kHz Reaper session.
    #[arg(long, default_value_t = 96_000)]
    input_sample_rate: u32,

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

    /// Audio bitrate requested for the published track.
    #[arg(long, default_value_t = 256_000)]
    bitrate: u64,

    /// LiveKit native source queue in milliseconds.
    #[arg(long, default_value_t = 1000)]
    livekit_buffer_ms: u32,

    /// Local capture ring buffer in seconds.
    #[arg(long, default_value_t = 4)]
    ring_buffer_seconds: usize,
}

#[derive(Default)]
struct State {
    captured_frames: AtomicU64,
    sent_frames: AtomicU64,
    overflow_frames: AtomicU64,
    underruns: AtomicU64,
    capture_errors: AtomicU64,
    ring_buffer_ms: AtomicUsize,
}

#[tokio::main]
async fn main() -> AppResult<()> {
    log::init("nab-live");

    let args = Args::parse();
    let host = cpal::default_host();

    if args.list_devices {
        list_input_devices(&host)?;
        return Ok(());
    }

    validate_channel_selection(&args)?;

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

    let device = select_input_device(&host, args.input.as_deref())?;
    let device_name = device_name(&device);
    ensure_input_support(&device, args.input_sample_rate, args.input_channels)?;

    eprintln!("NAB Live input: {device_name}");
    eprintln!(
        "Capture: {} Hz, {}ch, L={}, R={}",
        args.input_sample_rate, args.input_channels, args.left_channel, args.right_channel
    );
    eprintln!(
        "Publish: {} room={} identity={}",
        livekit_url, args.room, args.identity
    );

    let token = AccessToken::with_api_key(&api_key, &api_secret)
        .with_identity(&args.identity)
        .with_name(&args.identity)
        .with_grants(VideoGrants {
            room_join: true,
            room: args.room.clone(),
            can_publish: true,
            can_subscribe: true,
            ..Default::default()
        })
        .to_jwt()?;

    let (room, mut events) = Room::connect(&livekit_url, &token, RoomOptions::default()).await?;
    let event_state = args.identity.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            log::log(&format!("{event_state}: LiveKit event: {event:?}"));
        }
    });

    let native_source = NativeAudioSource::new(
        AudioSourceOptions::default(),
        OUT_RATE,
        OUT_CHANNELS as u32,
        args.livekit_buffer_ms,
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
                    max_bitrate: args.bitrate,
                }),
                dtx: false,
                red: true,
                source: TrackSource::Microphone,
                ..Default::default()
            },
        )
        .await?;
    eprintln!("Published track SID: {}", publication.sid());

    let state = Arc::new(State::default());
    let ring_capacity = args.input_sample_rate.max(OUT_RATE) as usize
        * OUT_CHANNELS
        * args.ring_buffer_seconds.max(1);
    let rb = HeapRb::<f32>::new(ring_capacity);
    let (mut prod, mut cons) = rb.split();

    let left_index = args.left_channel - 1;
    let right_index = args.right_channel - 1;
    let input_channels = args.input_channels as usize;
    let state_cb = Arc::clone(&state);
    let input_stream = device.build_input_stream(
        &cpal::StreamConfig {
            channels: args.input_channels,
            sample_rate: cpal::SampleRate(args.input_sample_rate),
            buffer_size: cpal::BufferSize::Default,
        },
        move |data: &[f32], _| {
            for frame in data.chunks(input_channels) {
                if frame.len() <= left_index || frame.len() <= right_index {
                    state_cb.capture_errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                if prod.free_len() >= OUT_CHANNELS {
                    let pair = [frame[left_index], frame[right_index]];
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

    let mut pump = AudioPump::new(args.input_sample_rate)?;
    wait_for_initial_buffer(&mut cons, args.input_sample_rate).await;

    let mut status_tick = tokio::time::interval(Duration::from_secs(2));
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                eprintln!("Stopping NAB Live...");
                break;
            }
            _ = status_tick.tick() => {
                print_status(&state, &device_name, args.input_sample_rate);
            }
            result = pump.send_next_frame(&mut cons, &native_source, &state) => {
                result?;
            }
        }
    }

    let _ = room
        .local_participant()
        .unpublish_track(&publication.sid())
        .await;
    let _ = room.close().await;
    drop(input_stream);
    Ok(())
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

fn validate_channel_selection(args: &Args) -> AppResult<()> {
    if args.input_channels == 0 {
        return Err(io_error("input_channels must be greater than zero").into());
    }
    if args.left_channel == 0 || args.right_channel == 0 {
        return Err(io_error("left_channel/right_channel are 1-based").into());
    }
    if args.left_channel > args.input_channels as usize
        || args.right_channel > args.input_channels as usize
    {
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

fn print_status(state: &State, device_name: &str, input_rate: u32) {
    let captured = state.captured_frames.load(Ordering::Relaxed);
    let sent = state.sent_frames.load(Ordering::Relaxed);
    let overflow = state.overflow_frames.load(Ordering::Relaxed);
    let underruns = state.underruns.load(Ordering::Relaxed);
    let errors = state.capture_errors.load(Ordering::Relaxed);
    let buffer_ms = state.ring_buffer_ms.load(Ordering::Relaxed);

    eprintln!(
        "[{device_name}] in={}Hz buffer={}ms captured={} sent={} overflow={} underrun={} errors={}",
        input_rate, buffer_ms, captured, sent, overflow, underruns, errors
    );
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

fn io_error(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, message.into())
}

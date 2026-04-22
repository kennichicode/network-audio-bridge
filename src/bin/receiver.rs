// Network Audio Bridge — Receiver v2
// アダプティブSRCによるクロックドリフト補正

use byteorder::{ByteOrder, LittleEndian};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph},
    Terminal,
};
use ringbuf::HeapRb;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[path = "../log.rs"]
mod log;
#[path = "../netinfo.rs"]
mod netinfo;
#[path = "../netopts.rs"]
mod netopts;
#[path = "../packet.rs"]
mod packet;

use packet::{packet_bytes, parse_header, HEADER_BYTES, PACKET_SAMPLES, ParseError};

const CH: usize = 2;
const PACKET_BYTES: usize = packet_bytes(CH);
const DEFAULT_PORT: u16 = 8000;

// ─── SRC設定 ─────────────────────────────────────────────────────
const SRC_CHUNK: usize = 512;
const P_GAIN_DEFAULT: f64 = 3e-7;
const P_GAIN_MIN: f64 = 1e-8;
const P_GAIN_MAX: f64 = 1e-5;
const MAX_RATIO_DEV: f64 = 0.005;

const SAMPLE_RATES: &[(u32, &str)] = &[
    (44100,  "44.1 kHz"),
    (48000,  "48 kHz   (default)"),
    (88200,  "88.2 kHz"),
    (96000,  "96 kHz"),
    (176400, "176.4 kHz"),
    (192000, "192 kHz"),
];

struct RecvConfig {
    sample_rate: u32,
    output_device: Option<String>,
    port: u16,
}

enum WizardStep {
    SelectSampleRate { cursor: usize },
    SelectOutput { devices: Vec<String>, cursor: usize },
    EnterPort { buf: String },
}

struct State {
    raw_ms: AtomicUsize,
    play_ms: AtomicUsize,
    loss: AtomicU64,
    buf_overflow: AtomicU64,
    ppm: AtomicI64,
    rebuffering: AtomicBool,
    jitter_ms: AtomicUsize,
    p_gain_bits: AtomicU64,
    running: AtomicBool,
}

impl State {
    fn new() -> Self {
        Self {
            raw_ms: AtomicUsize::new(0),
            play_ms: AtomicUsize::new(0),
            loss: AtomicU64::new(0),
            buf_overflow: AtomicU64::new(0),
            ppm: AtomicI64::new(0),
            rebuffering: AtomicBool::new(true),
            jitter_ms: AtomicUsize::new(300),
            p_gain_bits: AtomicU64::new(P_GAIN_DEFAULT.to_bits()),
            running: AtomicBool::new(true),
        }
    }

    fn get_p_gain(&self) -> f64 {
        f64::from_bits(self.p_gain_bits.load(Ordering::Relaxed))
    }

    fn set_p_gain(&self, v: f64) {
        self.p_gain_bits.store(v.to_bits(), Ordering::Relaxed);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    log::init("nab-recv");

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        log::log(&format!("PANIC: {}", info));
        default_hook(info);
    }));

    let host = cpal::default_host();
    let output_devices: Vec<String> = host
        .output_devices()
        .map(|d| d.filter_map(|x| x.name().ok()).collect())
        .unwrap_or_default();

    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;

    let config = match run_wizard(&mut terminal, &output_devices)? {
        Some(c) => c,
        None => {
            let _ = disable_raw_mode();
            let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
            return Ok(());
        }
    };

    let sr = config.sample_rate as usize;
    let port = config.port;
    let sample_rate = config.sample_rate;
    let my_ip = netinfo::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "?.?.?.?".to_string());

    let device = {
        let selected = config.output_device.as_deref().and_then(|name| {
            host.output_devices().ok()?.find(|d| d.name().ok().as_deref() == Some(name))
        });
        selected
            .or_else(|| host.default_output_device())
            .ok_or("出力デバイスが見つかりません")?
    };
    let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());

    let cpal_cfg = cpal::StreamConfig {
        channels: CH as u16,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let state = Arc::new(State::new());

    let buf_cap = sr * 4 * CH;
    let raw_rb = HeapRb::<f32>::new(buf_cap);
    let play_rb = HeapRb::<f32>::new(buf_cap);
    let (raw_prod, raw_cons) = raw_rb.split();
    let (play_prod, mut play_cons) = play_rb.split();

    let stream = {
        let state_cb = Arc::clone(&state);
        device.build_output_stream(
            &cpal_cfg,
            move |data: &mut [f32], _| {
                if state_cb.rebuffering.load(Ordering::Relaxed) {
                    for s in data.iter_mut() { *s = 0.0; }
                    return;
                }
                let n = play_cons.pop_slice(data);
                for s in data[n..].iter_mut() { *s = 0.0; }
                if n == 0 {
                    state_cb.rebuffering.store(true, Ordering::Relaxed);
                }
                let fill_ms = play_cons.len() * 1000 / (sr * CH).max(1);
                state_cb.play_ms.store(fill_ms, Ordering::Relaxed);
            },
            |e| log::log(&format!("output stream error: {}", e)),
            None,
        )?
    };
    stream.play()?;

    let udp_thread = {
        let state_udp = Arc::clone(&state);
        let mut prod = raw_prod;
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        let handle = thread::spawn(move || {
            let socket = match UdpSocket::bind(format!("0.0.0.0:{port}")) {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("UDPバインド失敗: {}", e)));
                    return;
                }
            };
            if let Err(e) = netopts::disable_udp_connreset(&socket) {
                log::log(&format!("disable_udp_connreset (recv) failed: {}", e));
            }
            if let Err(e) = socket.set_read_timeout(Some(Duration::from_millis(100))) {
                let _ = ready_tx.send(Err(format!("読み込みタイムアウト設定失敗: {}", e)));
                return;
            }
            let _ = ready_tx.send(Ok(()));

            let mut pkt = vec![0u8; PACKET_BYTES];
            let mut expected: Option<u32> = None;
            let mut last_mismatch_log = Instant::now() - Duration::from_secs(10);

            while state_udp.running.load(Ordering::Relaxed) {
                match socket.recv_from(&mut pkt) {
                    Ok((amt, _)) if amt == PACKET_BYTES => {
                        let header = match parse_header(&pkt, sr as u32, CH as u8) {
                            Ok(h) => h,
                            Err(e) => {
                                if last_mismatch_log.elapsed() > Duration::from_secs(5) {
                                    match e {
                                        ParseError::BadMagic => log::log("rx: bad magic / alien packet"),
                                        ParseError::UnsupportedVersion(v) => log::log(&format!("rx: unsupported version {}", v)),
                                        ParseError::SampleRateMismatch { got, expected } => log::log(&format!("rx: sample rate mismatch got={} expected={}", got, expected)),
                                        ParseError::ChannelsMismatch { got, expected } => log::log(&format!("rx: channels mismatch got={} expected={}", got, expected)),
                                    }
                                    last_mismatch_log = Instant::now();
                                }
                                continue;
                            }
                        };
                        let seq = header.seq;
                        if let Some(exp) = expected {
                            let diff = seq.wrapping_sub(exp);
                            if diff >= u32::MAX / 2 { continue; }
                            if diff > 0 {
                                let fill = diff.min(32) as usize * PACKET_SAMPLES * CH;
                                let mut dropped = 0u64;
                                for _ in 0..fill {
                                    if prod.push(0.0f32).is_err() { dropped += 1; }
                                }
                                if dropped > 0 {
                                    state_udp.buf_overflow.fetch_add(dropped, Ordering::Relaxed);
                                }
                                state_udp.loss.fetch_add(diff as u64, Ordering::Relaxed);
                            }
                        }
                        expected = Some(seq.wrapping_add(1));
                        let mut dropped = 0u64;
                        for i in 0..(PACKET_SAMPLES * CH) {
                            let s = LittleEndian::read_f32(&pkt[HEADER_BYTES + i * 4..HEADER_BYTES + (i + 1) * 4]);
                            if prod.push(s).is_err() { dropped += 1; }
                        }
                        if dropped > 0 {
                            state_udp.buf_overflow.fetch_add(dropped, Ordering::Relaxed);
                        }
                        let ms = prod.len() * 1000 / (sr * CH).max(1);
                        state_udp.raw_ms.store(ms, Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
        });
        match ready_rx.recv() {
            Ok(Ok(())) => handle,
            Ok(Err(e)) => {
                let _ = handle.join();
                let _ = disable_raw_mode();
                let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
                return Err(e.into());
            }
            Err(_) => {
                let _ = handle.join();
                let _ = disable_raw_mode();
                let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
                return Err("UDPスレッド起動失敗".into());
            }
        }
    };

    let src_thread = {
        let state_src = Arc::clone(&state);
        let mut cons = raw_cons;
        let mut pprod = play_prod;

        thread::spawn(move || {
            let params = SincInterpolationParameters {
                sinc_len: 64,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Linear,
                oversampling_factor: 64,
                window: WindowFunction::BlackmanHarris2,
            };
            let mut resampler = match SincFixedIn::<f32>::new(
                1.0,
                1.0 + MAX_RATIO_DEV + 0.001,
                params,
                SRC_CHUNK,
                CH,
            ) {
                Ok(r) => r,
                Err(e) => {
                    log::log(&format!("リサンプラー初期化失敗: {}", e));
                    state_src.running.store(false, Ordering::Relaxed);
                    return;
                }
            };

            let max_out = (SRC_CHUNK as f64 * (1.0 + MAX_RATIO_DEV) + 16.0) as usize;
            let mut waves_out: Vec<Vec<f32>> = vec![vec![0.0f32; max_out]; CH];
            let mut waves_in: Vec<Vec<f32>> = vec![vec![0.0f32; SRC_CHUNK]; CH];
            let mut interleaved = vec![0.0f32; SRC_CHUNK * CH];
            let mut ratio_timer = Instant::now();

            while state_src.running.load(Ordering::Relaxed) {
                if cons.len() < SRC_CHUNK * CH {
                    thread::sleep(Duration::from_micros(500));
                    continue;
                }

                cons.pop_slice(&mut interleaved);
                for i in 0..SRC_CHUNK {
                    for c in 0..CH {
                        waves_in[c][i] = interleaved[i * CH + c];
                    }
                }

                let n_frames = match resampler.process_into_buffer(&waves_in, &mut waves_out, None) {
                    Ok((_, n)) => n,
                    Err(e) => {
                        log::log(&format!("SRC error: {}", e));
                        continue;
                    }
                };

                let mut dropped = 0u64;
                for i in 0..n_frames {
                    for c in 0..CH {
                        if pprod.push(waves_out[c][i]).is_err() { dropped += 1; }
                    }
                }
                if dropped > 0 {
                    state_src.buf_overflow.fetch_add(dropped, Ordering::Relaxed);
                }

                let play_ms = pprod.len() * 1000 / (sr * CH).max(1);
                state_src.play_ms.store(play_ms, Ordering::Relaxed);

                // ヒステリシス: 解除閾値は target × 1.5
                if state_src.rebuffering.load(Ordering::Relaxed) {
                    let target = state_src.jitter_ms.load(Ordering::Relaxed) * sr * CH / 1000;
                    let release = target * 3 / 2;
                    if pprod.len() >= release {
                        state_src.rebuffering.store(false, Ordering::Relaxed);
                    }
                }

                if ratio_timer.elapsed() >= Duration::from_millis(200) {
                    ratio_timer = Instant::now();
                    let jitter_ms = state_src.jitter_ms.load(Ordering::Relaxed);
                    let target = (jitter_ms * sr * CH / 1000) as f64;
                    let current = cons.len() as f64;
                    let error = current - target;
                    let adj = error * state_src.get_p_gain();
                    let ratio = (1.0 + adj).clamp(1.0 - MAX_RATIO_DEV, 1.0 + MAX_RATIO_DEV);
                    if let Err(e) = resampler.set_resample_ratio(ratio, true) {
                        log::log(&format!("ratio update error: {}", e));
                    }
                    state_src.ppm.store((adj * 1_000_000.0) as i64, Ordering::Relaxed);
                }
            }
        })
    };

    let mut quit_pressed = false;
    let mut quit_timer = Instant::now();

    loop {
        let rebuf   = state.rebuffering.load(Ordering::Relaxed);
        let raw_ms  = state.raw_ms.load(Ordering::Relaxed);
        let play_ms = state.play_ms.load(Ordering::Relaxed);
        let loss    = state.loss.load(Ordering::Relaxed);
        let overflow = state.buf_overflow.load(Ordering::Relaxed);
        let ppm     = state.ppm.load(Ordering::Relaxed);
        let jitter  = state.jitter_ms.load(Ordering::Relaxed);
        let p_gain  = state.get_p_gain();

        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(0),
                ])
                .split(f.area());

            let (status_text, status_color) = if rebuf {
                ("BUFFERING...", Color::Yellow)
            } else {
                ("PLAYING", Color::Green)
            };
            f.render_widget(
                Paragraph::new(format!(
                    "NAB Receiver v2  │  受信アドレス: {}:{}  │  {} Hz  │  {}",
                    my_ip, port, sample_rate, status_text
                ))
                .block(Block::default().borders(Borders::ALL).title(" Status  (送信側でこのアドレスを入力) "))
                .style(Style::default().fg(status_color)),
                chunks[0],
            );

            let raw_pct = (raw_ms * 100 / 4000).min(100) as u16;
            f.render_widget(
                Gauge::default()
                    .block(Block::default().borders(Borders::ALL)
                        .title(format!(" Receive Buffer (UDP → SRC)  {} ms ", raw_ms)))
                    .gauge_style(Style::default().fg(Color::Cyan))
                    .percent(raw_pct),
                chunks[1],
            );

            let play_pct = (play_ms * 100 / 4000).min(100) as u16;
            f.render_widget(
                Gauge::default()
                    .block(Block::default().borders(Borders::ALL)
                        .title(format!(" Playback Buffer (SRC → Output)  {} ms ", play_ms)))
                    .gauge_style(Style::default().fg(Color::Green))
                    .percent(play_pct),
                chunks[2],
            );

            let drift_color = if ppm.abs() < 200 { Color::Green }
                else if ppm.abs() < 1000 { Color::Yellow }
                else { Color::Red };
            f.render_widget(
                Paragraph::new(format!(
                    "Drift: {:+} ppm  │  ratio: {:.7}  │  Jitter: {} ms (+/-)  │  P-Gain: {:.2e} ([/])",
                    ppm,
                    1.0 + ppm as f64 / 1_000_000.0,
                    jitter,
                    p_gain,
                ))
                .block(Block::default().borders(Borders::ALL).title(" Clock Drift "))
                .style(Style::default().fg(drift_color)),
                chunks[3],
            );

            let stats_color = if overflow > 0 { Color::Red } else { Color::Reset };
            f.render_widget(
                Paragraph::new(format!(
                    "Loss: {}  │  Overflow: {}  │  Device: {}  │  q × 2 = quit",
                    loss, overflow, device_name
                ))
                .block(Block::default().borders(Borders::ALL).title(" Stats "))
                .style(Style::default().fg(stats_color)),
                chunks[4],
            );
        })?;

        if event::poll(Duration::from_millis(80))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        let v = state.jitter_ms.load(Ordering::Relaxed);
                        state.jitter_ms.store((v + 50).min(2000), Ordering::Relaxed);
                    }
                    KeyCode::Char('-') => {
                        let v = state.jitter_ms.load(Ordering::Relaxed);
                        state.jitter_ms.store(v.saturating_sub(50).max(50), Ordering::Relaxed);
                    }
                    KeyCode::Char(']') => {
                        let v = (state.get_p_gain() * 2.0).min(P_GAIN_MAX);
                        state.set_p_gain(v);
                    }
                    KeyCode::Char('[') => {
                        let v = (state.get_p_gain() / 2.0).max(P_GAIN_MIN);
                        state.set_p_gain(v);
                    }
                    KeyCode::Char('q') => {
                        if quit_pressed && quit_timer.elapsed() < Duration::from_secs(1) {
                            break;
                        }
                        quit_pressed = true;
                        quit_timer = Instant::now();
                    }
                    _ => { quit_pressed = false; }
                }
            }
        }
    }

    state.running.store(false, Ordering::Relaxed);
    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
    if let Err(e) = udp_thread.join() {
        log::log(&format!("udp thread panic: {:?}", e));
    }
    if let Err(e) = src_thread.join() {
        log::log(&format!("src thread panic: {:?}", e));
    }
    Ok(())
}

fn run_wizard(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    output_devices: &[String],
) -> Result<Option<RecvConfig>, Box<dyn std::error::Error>> {
    let default_cursor = SAMPLE_RATES.iter().position(|(r, _)| *r == 48000).unwrap_or(1);
    let mut step = WizardStep::SelectSampleRate { cursor: default_cursor };
    let mut config = RecvConfig {
        sample_rate: 48000,
        output_device: None,
        port: DEFAULT_PORT,
    };

    loop {
        terminal.draw(|f| draw_wizard(f, &step))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        match event::read()? {
            Event::Resize(_, _) => { terminal.autoresize()?; }
            Event::Key(key) if key.kind == crossterm::event::KeyEventKind::Press => {
                match &mut step {
                    WizardStep::SelectSampleRate { cursor } => match key.code {
                        KeyCode::Up   => *cursor = cursor.saturating_sub(1),
                        KeyCode::Down => *cursor = (*cursor + 1).min(SAMPLE_RATES.len() - 1),
                        KeyCode::Enter => {
                            config.sample_rate = SAMPLE_RATES[*cursor].0;
                            step = WizardStep::SelectOutput {
                                devices: output_devices.to_vec(),
                                cursor: 0,
                            };
                        }
                        KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
                        _ => {}
                    },
                    WizardStep::SelectOutput { devices, cursor } => match key.code {
                        KeyCode::Up   => *cursor = cursor.saturating_sub(1),
                        KeyCode::Down => *cursor = (*cursor + 1).min(devices.len().saturating_sub(1)),
                        KeyCode::Enter => {
                            config.output_device = devices.get(*cursor).cloned();
                            step = WizardStep::EnterPort { buf: String::new() };
                        }
                        KeyCode::Esc => return Ok(None),
                        _ => {}
                    },
                    WizardStep::EnterPort { buf } => match key.code {
                        KeyCode::Enter => {
                            config.port = if buf.is_empty() {
                                DEFAULT_PORT
                            } else {
                                buf.parse().unwrap_or(DEFAULT_PORT)
                            };
                            return Ok(Some(config));
                        }
                        KeyCode::Backspace => { buf.pop(); }
                        KeyCode::Char(c) if c.is_ascii_digit() => buf.push(c),
                        KeyCode::Esc => return Ok(None),
                        _ => {}
                    },
                }
            }
            _ => {}
        }
    }
}

fn draw_wizard(f: &mut ratatui::Frame, step: &WizardStep) {
    let outer = Block::default()
        .title(" Network Audio Bridge — Receiver v2 Setup ")
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
        WizardStep::SelectSampleRate { cursor } => {
            f.render_widget(
                Paragraph::new("Select sample rate  * 送信機側と同じレートを選ぶこと")
                    .style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            let items: Vec<ListItem> = SAMPLE_RATES.iter().enumerate()
                .map(|(i, (_, label))| {
                    let item = ListItem::new(format!("  {}", label));
                    if i == *cursor {
                        item.style(Style::default().fg(Color::Black).bg(Color::Cyan))
                    } else { item }
                })
                .collect();
            let mut s = ListState::default();
            s.select(Some(*cursor));
            f.render_stateful_widget(List::new(items), layout[1], &mut s);
            f.render_widget(
                Paragraph::new("↑↓ Select   Enter Confirm   Esc Quit")
                    .style(Style::default().fg(Color::DarkGray)),
                layout[2],
            );
        }
        WizardStep::SelectOutput { devices, cursor } => {
            f.render_widget(
                Paragraph::new("Select output device (speakers / interface)")
                    .style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            if devices.is_empty() {
                f.render_widget(
                    Paragraph::new("  ⚠  出力デバイスが見つかりません。システムのデフォルトデバイスを使用します。\n  Enter で続行してください。")
                        .style(Style::default().fg(Color::Red)),
                    layout[1],
                );
            } else {
                let items: Vec<ListItem> = devices.iter().enumerate()
                    .map(|(i, n)| {
                        let item = ListItem::new(format!("  {}", n));
                        if i == *cursor {
                            item.style(Style::default().fg(Color::Black).bg(Color::Cyan))
                        } else { item }
                    })
                    .collect();
                let mut s = ListState::default();
                s.select(Some(*cursor));
                f.render_stateful_widget(List::new(items), layout[1], &mut s);
            }
            f.render_widget(
                Paragraph::new("↑↓ Select   Enter Confirm   Esc Quit")
                    .style(Style::default().fg(Color::DarkGray)),
                layout[2],
            );
        }
        WizardStep::EnterPort { buf } => {
            let my_ip = netinfo::local_ip()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "?.?.?.?".to_string());
            f.render_widget(
                Paragraph::new(format!(
                    "待ち受けるポート番号（IPアドレスではありません）\n\
                     送信側には「{}:{}」を入力してもらう想定です。\n\
                     番号を変える必要がなければ Enter だけでOK（既定 {}）。",
                    my_ip, DEFAULT_PORT, DEFAULT_PORT
                ))
                .style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            f.render_widget(
                Paragraph::new(format!("  Port: {}█", buf))
                    .block(Block::default().borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan))),
                layout[1],
            );
            f.render_widget(
                Paragraph::new("Enter で決定（空欄なら 8000）   Esc で中止")
                    .style(Style::default().fg(Color::DarkGray)),
                layout[2],
            );
        }
    }
}

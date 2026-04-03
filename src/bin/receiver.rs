// Network Audio Bridge — Receiver v2
// アダプティブSRCによるクロックドリフト補正
// 送信機（nab）とのパケット互換維持・送信機コードは一切変更なし

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
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

// ─── パケット定数（送信機と同一） ──────────────────────────────────
const PACKET_SAMPLES: usize = 128;
const CH: usize = 2;
const PACKET_BYTES: usize = 4 + PACKET_SAMPLES * CH * 4;
const DEFAULT_PORT: u16 = 8000;

// ─── SRC設定 ─────────────────────────────────────────────────────
const SRC_CHUNK: usize = 512;           // 1回のSRC処理フレーム数（チャンネルあたり）
const P_GAIN_DEFAULT: f64 = 3e-7;       // 比例ゲイン初期値
const P_GAIN_MIN: f64 = 1e-8;           // 最小ゲイン
const P_GAIN_MAX: f64 = 1e-5;           // 最大ゲイン
const MAX_RATIO_DEV: f64 = 0.005;       // 最大レシオ偏差 ±0.5%

// 対応サンプルレート（全プロ用レート）
const SAMPLE_RATES: &[(u32, &str)] = &[
    (44100,  "44.1 kHz"),
    (48000,  "48 kHz   (default)"),
    (88200,  "88.2 kHz"),
    (96000,  "96 kHz"),
    (176400, "176.4 kHz"),
    (192000, "192 kHz"),
];

// ─── Wizard設定 ───────────────────────────────────────────────────
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

// ─── 共有状態 ─────────────────────────────────────────────────────
struct State {
    raw_ms: AtomicUsize,
    play_ms: AtomicUsize,
    loss: AtomicU64,
    ppm: AtomicI64,
    rebuffering: AtomicBool,
    jitter_ms: AtomicUsize,
    p_gain_bits: AtomicU64,   // f64をビットとして保存（AtomicF64が安定版にないため）
    running: AtomicBool,
}

impl State {
    fn new() -> Self {
        Self {
            raw_ms: AtomicUsize::new(0),
            play_ms: AtomicUsize::new(0),
            loss: AtomicU64::new(0),
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

// ─── main ─────────────────────────────────────────────────────────
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
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

    // デバイス取得
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

    // ─── リングバッファ（4秒分）───────────────────────────────────
    let buf_cap = sr * 4 * CH;
    let raw_rb = HeapRb::<f32>::new(buf_cap);
    let play_rb = HeapRb::<f32>::new(buf_cap);
    let (raw_prod, raw_cons) = raw_rb.split();
    let (play_prod, mut play_cons) = play_rb.split();

    // ─── CPAL出力ストリーム ────────────────────────────────────────
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
            |e| eprintln!("出力エラー: {e}"),
            None,
        )?
    };
    stream.play()?;

    // ─── UDPスレッド ───────────────────────────────────────────────
    let udp_thread = {
        let state_udp = Arc::clone(&state);
        let mut prod = raw_prod;
        thread::spawn(move || {
            let socket = UdpSocket::bind(format!("0.0.0.0:{port}"))
                .expect("UDPバインド失敗");
            socket.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
            let mut pkt = vec![0u8; PACKET_BYTES];
            let mut expected: Option<u32> = None;

            while state_udp.running.load(Ordering::Relaxed) {
                match socket.recv_from(&mut pkt) {
                    Ok((amt, _)) if amt == PACKET_BYTES => {
                        let seq = LittleEndian::read_u32(&pkt[0..4]);
                        if let Some(exp) = expected {
                            let diff = seq.wrapping_sub(exp);
                            if diff >= u32::MAX / 2 { continue; }
                            if diff > 0 {
                                let fill = diff.min(32) as usize * PACKET_SAMPLES * CH;
                                for _ in 0..fill { let _ = prod.push(0.0f32); }
                                state_udp.loss.fetch_add(diff as u64, Ordering::Relaxed);
                            }
                        }
                        expected = Some(seq.wrapping_add(1));
                        for i in 0..(PACKET_SAMPLES * CH) {
                            let s = LittleEndian::read_f32(&pkt[4 + i * 4..4 + (i + 1) * 4]);
                            let _ = prod.push(s);
                        }
                        let ms = prod.len() * 1000 / (sr * CH).max(1);
                        state_udp.raw_ms.store(ms, Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
        })
    };

    // ─── SRCスレッド（ドリフト補正） ─────────────────────────────
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
            let mut resampler = SincFixedIn::<f32>::new(
                1.0,
                1.0 + MAX_RATIO_DEV + 0.001,
                params,
                SRC_CHUNK,
                CH,
            ).expect("リサンプラー初期化失敗");

            let max_out = (SRC_CHUNK as f64 * (1.0 + MAX_RATIO_DEV) + 16.0) as usize;
            let mut waves_out: Vec<Vec<f32>> = vec![vec![0.0f32; max_out]; CH];
            let mut waves_in: Vec<Vec<f32>> = vec![vec![0.0f32; SRC_CHUNK]; CH];
            let mut ratio_timer = Instant::now();

            while state_src.running.load(Ordering::Relaxed) {
                if cons.len() < SRC_CHUNK * CH {
                    thread::sleep(Duration::from_micros(500));
                    continue;
                }

                // インターリーブ読み出し → プレーナー変換
                let mut interleaved = vec![0.0f32; SRC_CHUNK * CH];
                cons.pop_slice(&mut interleaved);
                for i in 0..SRC_CHUNK {
                    for c in 0..CH {
                        waves_in[c][i] = interleaved[i * CH + c];
                    }
                }

                // リサンプリング
                let n_frames = match resampler.process_into_buffer(&waves_in, &mut waves_out, None) {
                    Ok((_, n)) => n,
                    Err(e) => { eprintln!("SRCエラー: {e}"); continue; }
                };

                // プレーナー → インターリーブ → play_bufへ
                for i in 0..n_frames {
                    for c in 0..CH {
                        let _ = pprod.push(waves_out[c][i]);
                    }
                }

                let play_ms = pprod.len() * 1000 / (sr * CH).max(1);
                state_src.play_ms.store(play_ms, Ordering::Relaxed);

                // リバッファリング解除チェック
                if state_src.rebuffering.load(Ordering::Relaxed) {
                    let target = state_src.jitter_ms.load(Ordering::Relaxed) * sr * CH / 1000;
                    if pprod.len() >= target {
                        state_src.rebuffering.store(false, Ordering::Relaxed);
                    }
                }

                // ドリフト補正レシオ更新（200ms毎）
                if ratio_timer.elapsed() >= Duration::from_millis(200) {
                    ratio_timer = Instant::now();
                    let jitter_ms = state_src.jitter_ms.load(Ordering::Relaxed);
                    let target = (jitter_ms * sr * CH / 1000) as f64;
                    let current = cons.len() as f64;
                    let error = current - target;
                    let adj = error * state_src.get_p_gain();
                    let ratio = (1.0 + adj).clamp(1.0 - MAX_RATIO_DEV, 1.0 + MAX_RATIO_DEV);
                    if let Err(e) = resampler.set_resample_ratio(ratio, true) {
                        eprintln!("レシオ更新エラー: {e}");
                    }
                    state_src.ppm.store((adj * 1_000_000.0) as i64, Ordering::Relaxed);
                }
            }
        })
    };

    // ─── TUI（メインループ） ──────────────────────────────────────
    let mut quit_pressed = false;
    let mut quit_timer = Instant::now();

    loop {
        let rebuf   = state.rebuffering.load(Ordering::Relaxed);
        let raw_ms  = state.raw_ms.load(Ordering::Relaxed);
        let play_ms = state.play_ms.load(Ordering::Relaxed);
        let loss    = state.loss.load(Ordering::Relaxed);
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

            // ステータス
            let (status_text, status_color) = if rebuf {
                ("BUFFERING...", Color::Yellow)
            } else {
                ("PLAYING", Color::Green)
            };
            f.render_widget(
                Paragraph::new(format!(
                    "Network Audio Bridge — Receiver v2  │  Port: {}  │  {} Hz  │  {}",
                    port, sample_rate, status_text
                ))
                .block(Block::default().borders(Borders::ALL).title(" Status "))
                .style(Style::default().fg(status_color)),
                chunks[0],
            );

            // raw buffer
            let raw_pct = (raw_ms * 100 / 4000).min(100) as u16;
            f.render_widget(
                Gauge::default()
                    .block(Block::default().borders(Borders::ALL)
                        .title(format!(" Receive Buffer (UDP → SRC)  {} ms ", raw_ms)))
                    .gauge_style(Style::default().fg(Color::Cyan))
                    .percent(raw_pct),
                chunks[1],
            );

            // play buffer
            let play_pct = (play_ms * 100 / 4000).min(100) as u16;
            f.render_widget(
                Gauge::default()
                    .block(Block::default().borders(Borders::ALL)
                        .title(format!(" Playback Buffer (SRC → Output)  {} ms ", play_ms)))
                    .gauge_style(Style::default().fg(Color::Green))
                    .percent(play_pct),
                chunks[2],
            );

            // ドリフト補正
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

            // 統計
            f.render_widget(
                Paragraph::new(format!(
                    "Packet loss: {}  │  Device: {}  │  q × 2 = quit",
                    loss, device_name
                ))
                .block(Block::default().borders(Borders::ALL).title(" Stats ")),
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
    let _ = udp_thread.join();
    let _ = src_thread.join();
    Ok(())
}

// ─── Wizard ───────────────────────────────────────────────────────
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
            f.render_widget(
                Paragraph::new(format!(
                    "Listen port  (Enter で {} を使用)",
                    DEFAULT_PORT
                ))
                .style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            f.render_widget(
                Paragraph::new(format!("  {}█", buf))
                    .block(Block::default().borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan))),
                layout[1],
            );
            f.render_widget(
                Paragraph::new("Enter Confirm (空白でデフォルト8000)   Esc Quit")
                    .style(Style::default().fg(Color::DarkGray)),
                layout[2],
            );
        }
    }
}

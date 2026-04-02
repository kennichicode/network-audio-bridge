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
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

// MTU(1500) - IP(20) - UDP(8) - seq(4) = 1468 bytes → 128 stereo f32 = 1028 bytes < 1500 ✓
const PACKET_SAMPLES: usize = 128;
const CHANNELS: u16 = 2;
const SAMPLE_RATE: u32 = 48000;
const DEFAULT_PORT: u16 = 8000;
const PACKET_BYTES: usize = 4 + (PACKET_SAMPLES * CHANNELS as usize * 4);

// ───────────────────────────────────────────────
// データ型
// ───────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum RunMode {
    Send,
    Recv,
    Duplex,
}

#[derive(Clone)]
struct RunConfig {
    mode: RunMode,
    input_device: Option<String>,
    output_device: Option<String>,
    remote_addr: String,
    listen_addr: String,
}

enum WizardStep {
    SelectMode { cursor: usize },
    SelectInput { devices: Vec<String>, cursor: usize },
    SelectOutput { devices: Vec<String>, cursor: usize },
    EnterIP { buf: String },
}

struct SharedState {
    packets_sent: AtomicU64,
    send_errors: AtomicU64,
    packet_loss: AtomicU64,
    send_buffer_pct: AtomicUsize,
    recv_buffer_pct: AtomicUsize,
    jitter_buffer_ms: AtomicUsize,
    recv_buffer_ms: AtomicUsize,
    bandwidth_kbps: AtomicUsize,
    running: AtomicBool,
}
impl SharedState {
    fn new() -> Self {
        Self {
            packets_sent: AtomicU64::new(0),
            send_errors: AtomicU64::new(0),
            packet_loss: AtomicU64::new(0),
            send_buffer_pct: AtomicUsize::new(0),
            recv_buffer_pct: AtomicUsize::new(0),
            jitter_buffer_ms: AtomicUsize::new(100),
            recv_buffer_ms: AtomicUsize::new(0),
            bandwidth_kbps: AtomicUsize::new(0),
            running: AtomicBool::new(true),
        }
    }
}

// ───────────────────────────────────────────────
// main
// ───────────────────────────────────────────────

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // パニック時にターミナルを必ず復旧する
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));

    #[cfg(all(feature = "asio", target_os = "windows"))]
    let host = {
        let asio_id = cpal::available_hosts()
            .into_iter()
            .find(|id| *id == cpal::HostId::Asio);
        match asio_id {
            Some(id) => cpal::host_from_id(id).unwrap_or_else(|_| cpal::default_host()),
            None => cpal::default_host(),
        }
    };
    #[cfg(not(all(feature = "asio", target_os = "windows")))]
    let host = cpal::default_host();

    let input_devices: Vec<String> = host
        .input_devices()
        .map(|d| d.filter_map(|x| x.name().ok()).collect())
        .unwrap_or_default();
    let output_devices: Vec<String> = host
        .output_devices()
        .map(|d| d.filter_map(|x| x.name().ok()).collect())
        .unwrap_or_default();

    enable_raw_mode()?;
    if let Err(e) = execute!(std::io::stdout(), EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(e.into());
    }

    let mut terminal = match Terminal::new(CrosstermBackend::new(std::io::stdout())) {
        Ok(terminal) => terminal,
        Err(e) => {
            let _ = disable_raw_mode();
            let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
            return Err(e.into());
        }
    };

    let app_result = run_app(&host, &input_devices, &output_devices, &mut terminal);
    if let Err(e) = cleanup(&mut terminal) {
        eprintln!("ターミナル復旧失敗: {}", e);
    }
    if let Err(e) = app_result {
        eprintln!("{}", e);
        std::process::exit(1);
    }
    Ok(())
}

fn cleanup(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

fn run_app(
    host: &cpal::Host,
    input_devices: &[String],
    output_devices: &[String],
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<(), String> {
    let config =
        match run_wizard(terminal, input_devices, output_devices).map_err(|e| e.to_string())? {
            Some(c) => c,
            None => return Ok(()),
        };

    let state = Arc::new(SharedState::new());
    let is_send = matches!(config.mode, RunMode::Send | RunMode::Duplex);
    let is_recv = matches!(config.mode, RunMode::Recv | RunMode::Duplex);
    let mut send_thread = None;
    let mut recv_thread = None;

    let result = (|| -> Result<(), String> {
        if is_send {
            let dev = select_input_device(host, config.input_device.as_deref())?;
            ensure_input_support(&dev)?;
            let addr = config.remote_addr.clone();
            let s = Arc::clone(&state);
            send_thread = Some(spawn_sender(dev, stream_config(), addr, s)?);
        }

        if is_recv {
            let dev = select_output_device(host, config.output_device.as_deref())?;
            ensure_output_support(&dev)?;
            let addr = config.listen_addr.clone();
            let s = Arc::clone(&state);
            recv_thread = Some(spawn_receiver(dev, stream_config(), addr, s)?);
        }

        run_tui(terminal, &config, &state).map_err(|e| e.to_string())
    })();

    state.running.store(false, Ordering::Relaxed);
    if let Some(t) = send_thread {
        let _ = t.join();
    }
    if let Some(t) = recv_thread {
        let _ = t.join();
    }

    result
}

fn stream_config() -> cpal::StreamConfig {
    cpal::StreamConfig {
        channels: CHANNELS,
        sample_rate: cpal::SampleRate(SAMPLE_RATE),
        buffer_size: cpal::BufferSize::Default,
    }
}

fn device_name(device: &cpal::Device) -> String {
    device
        .name()
        .unwrap_or_else(|_| "不明なデバイス".to_string())
}

fn select_input_device(
    host: &cpal::Host,
    selected_name: Option<&str>,
) -> Result<cpal::Device, String> {
    let selected = match selected_name {
        Some(name) => host
            .input_devices()
            .map_err(|e| format!("入力デバイス一覧の取得に失敗: {}", e))?
            .find(|d| d.name().ok().as_deref() == Some(name)),
        None => None,
    };

    selected
        .or_else(|| host.default_input_device())
        .ok_or_else(|| "エラー: 入力デバイスが見つかりません".to_string())
}

fn select_output_device(
    host: &cpal::Host,
    selected_name: Option<&str>,
) -> Result<cpal::Device, String> {
    let selected = match selected_name {
        Some(name) => host
            .output_devices()
            .map_err(|e| format!("出力デバイス一覧の取得に失敗: {}", e))?
            .find(|d| d.name().ok().as_deref() == Some(name)),
        None => None,
    };

    selected
        .or_else(|| host.default_output_device())
        .ok_or_else(|| "エラー: 出力デバイスが見つかりません".to_string())
}

fn ensure_input_support(device: &cpal::Device) -> Result<(), String> {
    let mut configs = device.supported_input_configs().map_err(|e| {
        format!(
            "入力デバイス「{}」の対応フォーマット取得に失敗: {}",
            device_name(device),
            e
        )
    })?;
    let supported = configs.any(|r| {
        r.channels() == CHANNELS
            && r.sample_format() == cpal::SampleFormat::F32
            && r.min_sample_rate().0 <= SAMPLE_RATE
            && r.max_sample_rate().0 >= SAMPLE_RATE
    });

    if supported {
        Ok(())
    } else {
        Err(format!(
            "エラー: 入力デバイス「{}」は 48kHz / ステレオ / f32 に対応していません。\nオーディオインターフェースのサンプルレートやフォーマット設定を確認してください。",
            device_name(device)
        ))
    }
}

fn ensure_output_support(device: &cpal::Device) -> Result<(), String> {
    let mut configs = device.supported_output_configs().map_err(|e| {
        format!(
            "出力デバイス「{}」の対応フォーマット取得に失敗: {}",
            device_name(device),
            e
        )
    })?;
    let supported = configs.any(|r| {
        r.channels() == CHANNELS
            && r.sample_format() == cpal::SampleFormat::F32
            && r.min_sample_rate().0 <= SAMPLE_RATE
            && r.max_sample_rate().0 >= SAMPLE_RATE
    });

    if supported {
        Ok(())
    } else {
        Err(format!(
            "エラー: 出力デバイス「{}」は 48kHz / ステレオ / f32 に対応していません。\nオーディオインターフェースのサンプルレートやフォーマット設定を確認してください。",
            device_name(device)
        ))
    }
}

// ───────────────────────────────────────────────
// ウィザード
// ───────────────────────────────────────────────

fn run_wizard(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    input_devices: &[String],
    output_devices: &[String],
) -> Result<Option<RunConfig>, Box<dyn std::error::Error>> {
    let mut step = WizardStep::SelectMode { cursor: 0 };
    let mut config = RunConfig {
        mode: RunMode::Send,
        input_device: None,
        output_device: None,
        remote_addr: String::new(),
        listen_addr: format!("0.0.0.0:{}", DEFAULT_PORT),
    };

    loop {
        terminal.draw(|f| draw_wizard(f, &step))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        match event::read()? {
            Event::Resize(_, _) => {
                terminal.autoresize()?;
            }
            Event::Key(key) if key.kind == crossterm::event::KeyEventKind::Press => match &mut step {
                WizardStep::SelectMode { cursor } => match key.code {
                    KeyCode::Up => *cursor = cursor.saturating_sub(1),
                    KeyCode::Down => *cursor = (*cursor + 1).min(2),
                    KeyCode::Enter => {
                        config.mode = match *cursor {
                            0 => RunMode::Send,
                            1 => RunMode::Recv,
                            _ => RunMode::Duplex,
                        };
                        step = if config.mode != RunMode::Recv {
                            WizardStep::SelectInput {
                                devices: input_devices.to_vec(),
                                cursor: 0,
                            }
                        } else {
                            WizardStep::SelectOutput {
                                devices: output_devices.to_vec(),
                                cursor: 0,
                            }
                        };
                    }
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
                    _ => {}
                },

                WizardStep::SelectInput { devices, cursor } => match key.code {
                    KeyCode::Up => *cursor = cursor.saturating_sub(1),
                    KeyCode::Down => *cursor = (*cursor + 1).min(devices.len().saturating_sub(1)),
                    KeyCode::Enter => {
                        config.input_device = devices.get(*cursor).cloned();
                        step = if config.mode == RunMode::Duplex {
                            WizardStep::SelectOutput {
                                devices: output_devices.to_vec(),
                                cursor: 0,
                            }
                        } else {
                            WizardStep::EnterIP { buf: String::new() }
                        };
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },

                WizardStep::SelectOutput { devices, cursor } => match key.code {
                    KeyCode::Up => *cursor = cursor.saturating_sub(1),
                    KeyCode::Down => *cursor = (*cursor + 1).min(devices.len().saturating_sub(1)),
                    KeyCode::Enter => {
                        config.output_device = devices.get(*cursor).cloned();
                        if config.mode == RunMode::Recv {
                            return Ok(Some(config));
                        }
                        step = WizardStep::EnterIP { buf: String::new() };
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },

                WizardStep::EnterIP { buf } => match key.code {
                    KeyCode::Enter if !buf.is_empty() => {
                        config.remote_addr = if buf.contains(':') {
                            buf.clone()
                        } else {
                            format!("{}:{}", buf, DEFAULT_PORT)
                        };
                        return Ok(Some(config));
                    }
                    KeyCode::Backspace => {
                        buf.pop();
                    }
                    KeyCode::Char(c) if c.is_ascii_graphic() => buf.push(c),
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },
            },
            _ => {}
        }
    }
}

fn draw_wizard(f: &mut ratatui::Frame, step: &WizardStep) {
    let outer = Block::default()
        .title(" Network Audio Bridge — セットアップ ")
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
        WizardStep::SelectMode { cursor } => {
            f.render_widget(
                Paragraph::new("モードを選択").style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            let modes = [
                "送信   — このデバイスの音を相手に送る",
                "受信   — 相手の音をこのデバイスで聴く",
                "双方向 — 送受信を同時に行う",
            ];
            let items: Vec<ListItem> = modes
                .iter()
                .enumerate()
                .map(|(i, &m)| {
                    let item = ListItem::new(format!("  {}", m));
                    if i == *cursor {
                        item.style(Style::default().fg(Color::Black).bg(Color::Cyan))
                    } else {
                        item
                    }
                })
                .collect();
            let mut s = ListState::default();
            s.select(Some(*cursor));
            f.render_stateful_widget(List::new(items), layout[1], &mut s);
            f.render_widget(
                Paragraph::new("↑↓ 選択   Enter 決定   Esc 終了")
                    .style(Style::default().fg(Color::DarkGray)),
                layout[2],
            );
        }
        WizardStep::SelectInput { devices, cursor } => {
            f.render_widget(
                Paragraph::new("入力デバイス（マイク・インターフェース）を選択")
                    .style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            let items: Vec<ListItem> = devices
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let item = ListItem::new(format!("  {}", n));
                    if i == *cursor {
                        item.style(Style::default().fg(Color::Black).bg(Color::Cyan))
                    } else {
                        item
                    }
                })
                .collect();
            let mut s = ListState::default();
            s.select(Some(*cursor));
            f.render_stateful_widget(List::new(items), layout[1], &mut s);
            f.render_widget(
                Paragraph::new("↑↓ 選択   Enter 決定   Esc 終了")
                    .style(Style::default().fg(Color::DarkGray)),
                layout[2],
            );
        }
        WizardStep::SelectOutput { devices, cursor } => {
            f.render_widget(
                Paragraph::new("出力デバイス（スピーカー・インターフェース）を選択")
                    .style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            let items: Vec<ListItem> = devices
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let item = ListItem::new(format!("  {}", n));
                    if i == *cursor {
                        item.style(Style::default().fg(Color::Black).bg(Color::Cyan))
                    } else {
                        item
                    }
                })
                .collect();
            let mut s = ListState::default();
            s.select(Some(*cursor));
            f.render_stateful_widget(List::new(items), layout[1], &mut s);
            f.render_widget(
                Paragraph::new("↑↓ 選択   Enter 決定   Esc 終了")
                    .style(Style::default().fg(Color::DarkGray)),
                layout[2],
            );
        }
        WizardStep::EnterIP { buf } => {
            f.render_widget(
                Paragraph::new("相手のIPアドレスを入力  （例: 192.168.1.100）")
                    .style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            f.render_widget(
                Paragraph::new(format!("  {}█", buf)).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                ),
                layout[1],
            );
            f.render_widget(
                Paragraph::new("Enter 決定   Esc 終了").style(Style::default().fg(Color::DarkGray)),
                layout[2],
            );
        }
    }
}

// ───────────────────────────────────────────────
// メインTUI（動作中）
// ───────────────────────────────────────────────

fn run_tui(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    config: &RunConfig,
    state: &Arc<SharedState>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mode_label = match config.mode {
        RunMode::Send => "送信",
        RunMode::Recv => "受信",
        RunMode::Duplex => "双方向",
    };
    let is_send = matches!(config.mode, RunMode::Send | RunMode::Duplex);
    let is_recv = matches!(config.mode, RunMode::Recv | RunMode::Duplex);
    let mut quit_pending = false;
    let mut quit_time = std::time::Instant::now();

    loop {
        if quit_pending && quit_time.elapsed() > Duration::from_secs(1) {
            quit_pending = false;
        }
        let sent = state.packets_sent.load(Ordering::Relaxed);
        let send_err = state.send_errors.load(Ordering::Relaxed);
        let loss = state.packet_loss.load(Ordering::Relaxed);
        let send_p = state.send_buffer_pct.load(Ordering::Relaxed).min(100) as u16;
        let recv_p = state.recv_buffer_pct.load(Ordering::Relaxed).min(100) as u16;
        let jitter_ms = state.jitter_buffer_ms.load(Ordering::Relaxed);
        let buf_ms = state.recv_buffer_ms.load(Ordering::Relaxed);
        let bw_kbps = state.bandwidth_kbps.load(Ordering::Relaxed);

        terminal.draw(|f| {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(4),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                ])
                .split(f.area());

            let conn = if is_send {
                format!("→ {}", config.remote_addr)
            } else {
                format!("← {}", config.listen_addr)
            };
            f.render_widget(
                Paragraph::new(format!(" {}  {}  ● 動作中", mode_label, conn))
                    .block(
                        Block::default()
                            .title("Network Audio Bridge")
                            .borders(Borders::ALL),
                    )
                    .style(Style::default().fg(Color::Cyan)),
                rows[0],
            );

            f.render_widget(
                Paragraph::new(format!(
                    " 入力 : {}\n 出力 : {}",
                    config.input_device.as_deref().unwrap_or("—"),
                    config.output_device.as_deref().unwrap_or("—"),
                ))
                .block(Block::default().title("デバイス").borders(Borders::ALL)),
                rows[1],
            );

            f.render_widget(
                Gauge::default()
                    .block(
                        Block::default()
                            .title(if is_send {
                                "送信バッファ"
                            } else {
                                "送信（未使用）"
                            })
                            .borders(Borders::ALL),
                    )
                    .gauge_style(Style::default().fg(if is_send {
                        Color::Cyan
                    } else {
                        Color::DarkGray
                    }))
                    .percent(if is_send { send_p } else { 0 }),
                rows[2],
            );

            f.render_widget(
                Gauge::default()
                    .block(
                        Block::default()
                            .title(if is_recv {
                                "受信バッファ"
                            } else {
                                "受信（未使用）"
                            })
                            .borders(Borders::ALL),
                    )
                    .gauge_style(Style::default().fg(if is_recv {
                        Color::Green
                    } else {
                        Color::DarkGray
                    }))
                    .percent(if is_recv { recv_p } else { 0 }),
                rows[3],
            );

            let bw_str = if bw_kbps >= 1000 {
                format!("{:.1} Mbps", bw_kbps as f64 / 1000.0)
            } else {
                format!("{} kbps", bw_kbps)
            };
            f.render_widget(
                Paragraph::new(format!(
                    " 目標レイテンシー: {}ms   バッファ: {}ms   帯域: {}   [+][-] バッファ調整",
                    jitter_ms, buf_ms, bw_str
                ))
                .block(Block::default().title("ネットワーク").borders(Borders::ALL))
                .style(Style::default().fg(Color::Cyan)),
                rows[4],
            );

            let stats_color = if send_err > 0 {
                Color::Red
            } else {
                Color::Reset
            };
            let stats_line = if quit_pending {
                format!(" 送信: {:>8}pkt   エラー: {:>4}   ロス: {:>4}pkt   [q] もう一度で終了", sent, send_err, loss)
            } else {
                format!(" 送信: {:>8}pkt   エラー: {:>4}   ロス: {:>4}pkt   [q] 終了", sent, send_err, loss)
            };
            f.render_widget(
                Paragraph::new(stats_line)
                .block(Block::default().title("統計").borders(Borders::ALL))
                .style(Style::default().fg(if quit_pending { Color::Yellow } else { stats_color })),
                rows[5],
            );
        })?;

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(k) if k.kind == crossterm::event::KeyEventKind::Press && k.code == KeyCode::Char('q') => {
                    if quit_pending {
                        break;
                    } else {
                        quit_pending = true;
                        quit_time = std::time::Instant::now();
                    }
                }
                Event::Key(k) if k.kind == crossterm::event::KeyEventKind::Press && (k.code == KeyCode::Char('+') || k.code == KeyCode::Char('=')) => {
                    let cur = state.jitter_buffer_ms.load(Ordering::Relaxed);
                    state.jitter_buffer_ms.store((cur + 10).min(500), Ordering::Relaxed);
                }
                Event::Key(k) if k.kind == crossterm::event::KeyEventKind::Press && k.code == KeyCode::Char('-') => {
                    let cur = state.jitter_buffer_ms.load(Ordering::Relaxed);
                    state.jitter_buffer_ms.store(cur.saturating_sub(10).max(50), Ordering::Relaxed);
                }
                Event::Resize(_, _) => terminal.autoresize()?,
                _ => {}
            }
        }
    }
    Ok(())
}

// ───────────────────────────────────────────────
// オーディオ送信
// ───────────────────────────────────────────────

fn spawn_sender(
    device: cpal::Device,
    cfg: cpal::StreamConfig,
    target: String,
    state: Arc<SharedState>,
) -> Result<thread::JoinHandle<()>, String> {
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || run_sender(device, cfg, target, state, ready_tx));

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(err)) => {
            let _ = handle.join();
            Err(err)
        }
        Err(_) => {
            let _ = handle.join();
            Err("送信スレッドの起動確認に失敗しました".to_string())
        }
    }
}

fn run_sender(
    device: cpal::Device,
    cfg: cpal::StreamConfig,
    target: String,
    state: Arc<SharedState>,
    ready_tx: mpsc::SyncSender<Result<(), String>>,
) {
    let ch = CHANNELS as usize;
    let capacity = SAMPLE_RATE as usize * 2 * ch;
    let rb = HeapRb::<f32>::new(capacity);
    let (mut prod, mut cons) = rb.split();

    let stream = match device.build_input_stream(
        &cfg,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            for &s in data {
                let _ = prod.push(s);
            }
        },
        |e| eprintln!("入力エラー: {}", e),
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            let _ = ready_tx.send(Err(format!("入力ストリーム構築失敗: {}", e)));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready_tx.send(Err(format!("入力ストリーム開始失敗: {}", e)));
        return;
    }

    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => {
            let _ = ready_tx.send(Err(format!("送信ソケット失敗: {}", e)));
            return;
        }
    };
    let _ = ready_tx.send(Ok(()));

    let frame = PACKET_SAMPLES * ch;
    let mut buf = vec![0.0f32; frame];
    let mut pkt = vec![0u8; PACKET_BYTES];
    let mut seq: u32 = 0;
    let mut bw_timer = std::time::Instant::now();
    let mut bw_last_sent: u64 = 0;

    while state.running.load(Ordering::Relaxed) {
        state
            .send_buffer_pct
            .store(cons.len() * 100 / capacity, Ordering::Relaxed);
        if cons.len() >= frame {
            let read = cons.pop_slice(&mut buf);
            for s in buf.iter_mut().skip(read) {
                *s = 0.0;
            }
            LittleEndian::write_u32(&mut pkt[0..4], seq);
            for (i, &s) in buf.iter().enumerate() {
                LittleEndian::write_f32(&mut pkt[4 + i * 4..4 + (i + 1) * 4], s);
            }
            // 送信成功時のみカウント、失敗は送信エラーとしてカウント
            match socket.send_to(&pkt, &target) {
                Ok(_) => {
                    state.packets_sent.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    state.send_errors.fetch_add(1, Ordering::Relaxed);
                }
            }
            seq = seq.wrapping_add(1);
        } else {
            thread::sleep(Duration::from_micros(500));
        }

        // 帯域計算（1秒ごとに更新）
        if bw_timer.elapsed() >= Duration::from_secs(1) {
            let current = state.packets_sent.load(Ordering::Relaxed);
            let kbps = current.saturating_sub(bw_last_sent) * PACKET_BYTES as u64 * 8 / 1000;
            state.bandwidth_kbps.store(kbps as usize, Ordering::Relaxed);
            bw_timer = std::time::Instant::now();
            bw_last_sent = current;
        }
    }
}

// ───────────────────────────────────────────────
// オーディオ受信
// ───────────────────────────────────────────────

fn spawn_receiver(
    device: cpal::Device,
    cfg: cpal::StreamConfig,
    listen: String,
    state: Arc<SharedState>,
) -> Result<thread::JoinHandle<()>, String> {
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || run_receiver(device, cfg, listen, state, ready_tx));

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(err)) => {
            let _ = handle.join();
            Err(err)
        }
        Err(_) => {
            let _ = handle.join();
            Err("受信スレッドの起動確認に失敗しました".to_string())
        }
    }
}

fn run_receiver(
    device: cpal::Device,
    cfg: cpal::StreamConfig,
    listen: String,
    state: Arc<SharedState>,
    ready_tx: mpsc::SyncSender<Result<(), String>>,
) {
    let ch = CHANNELS as usize;
    let socket = match UdpSocket::bind(&listen) {
        Ok(s) => s,
        Err(e) => {
            let _ = ready_tx.send(Err(format!("受信ソケット失敗: {}", e)));
            return;
        }
    };
    if let Err(e) = socket.set_read_timeout(Some(Duration::from_millis(100))) {
        let _ = ready_tx.send(Err(format!("受信ソケット設定失敗: {}", e)));
        return;
    }

    let rb = HeapRb::<f32>::new(SAMPLE_RATE as usize * 2 * ch);
    let (mut prod, mut cons) = rb.split();

    let rebuffering = Arc::new(AtomicBool::new(true));
    let rebuffering_cb = Arc::clone(&rebuffering);
    let state_cb = Arc::clone(&state);

    let stream = match device.build_output_stream(
        &cfg,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            if rebuffering_cb.load(Ordering::Relaxed) {
                for s in data.iter_mut() { *s = 0.0; }
                state_cb.recv_buffer_pct.store(0, Ordering::Relaxed);
                return;
            }
            let read = cons.pop_slice(data);
            for s in data.iter_mut().skip(read) { *s = 0.0; }
            if read == 0 {
                // アンダーラン — 再バッファリング開始
                rebuffering_cb.store(true, Ordering::Relaxed);
            }
            state_cb.recv_buffer_pct.store(read * 100 / data.len().max(1), Ordering::Relaxed);
        },
        |e| eprintln!("出力エラー: {}", e),
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            let _ = ready_tx.send(Err(format!("出力ストリーム構築失敗: {}", e)));
            return;
        }
    };
    let _ = ready_tx.send(Ok(()));

    let mut pkt = vec![0u8; PACKET_BYTES];
    let mut expected: Option<u32> = None;
    let mut playing = false;

    while state.running.load(Ordering::Relaxed) {
        match socket.recv_from(&mut pkt) {
            Ok((amt, _)) if amt == PACKET_BYTES => {
                let seq = LittleEndian::read_u32(&pkt[0..4]);
                if let Some(exp) = expected {
                    let diff = seq.wrapping_sub(exp);
                    if diff >= u32::MAX / 2 {
                        // 遅延/重複パケット — 音声データだけ捨てる
                        continue;
                    }
                    if diff > 0 {
                        // ロス補完: 最大32パケット分の無音を挿入（無限ループ防止）
                        let fill = diff.min(32) as usize * PACKET_SAMPLES * ch;
                        for _ in 0..fill { let _ = prod.push(0.0); }
                        state.packet_loss.fetch_add(diff as u64, Ordering::Relaxed);
                    }
                }
                expected = Some(seq.wrapping_add(1));
                for i in 0..(PACKET_SAMPLES * ch) {
                    let _ = prod.push(LittleEndian::read_f32(&pkt[4 + i * 4..4 + (i + 1) * 4]));
                }
            }
            _ => {}
        }

        // バッファ残量をmsに変換して報告
        let buf_ms = prod.len() * 1000 / (SAMPLE_RATE as usize * ch);
        state.recv_buffer_ms.store(buf_ms, Ordering::Relaxed);

        // ジッターバッファ分溜まったら再生開始（または再開）
        let jitter_samples = state.jitter_buffer_ms.load(Ordering::Relaxed) * SAMPLE_RATE as usize / 1000 * ch;
        if rebuffering.load(Ordering::Relaxed) && prod.len() >= jitter_samples {
            rebuffering.store(false, Ordering::Relaxed);
            if !playing {
                if let Err(e) = stream.play() {
                    eprintln!("出力ストリーム開始失敗: {}", e);
                    break;
                }
                playing = true;
            }
        }
    }
}

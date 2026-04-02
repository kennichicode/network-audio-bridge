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
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const PACKET_SAMPLES: usize = 256;
const CHANNELS: u16 = 2;
const SAMPLE_RATE: u32 = 48000;
const DEFAULT_PORT: u16 = 8000;
const PACKET_BYTES: usize = 4 + (PACKET_SAMPLES * CHANNELS as usize * 4);

// ───────────────────────────────────────────────
// データ型
// ───────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum RunMode { Send, Recv, Duplex }

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
    SelectInput  { devices: Vec<String>, cursor: usize },
    SelectOutput { devices: Vec<String>, cursor: usize },
    EnterIP { buf: String },
}

struct SharedState {
    packets_sent:    AtomicU64,
    packet_loss:     AtomicU64,
    send_buffer_pct: AtomicUsize,
    recv_buffer_pct: AtomicUsize,
    running:         AtomicBool,
}
impl SharedState {
    fn new() -> Self {
        Self {
            packets_sent:    AtomicU64::new(0),
            packet_loss:     AtomicU64::new(0),
            send_buffer_pct: AtomicUsize::new(0),
            recv_buffer_pct: AtomicUsize::new(0),
            running:         AtomicBool::new(true),
        }
    }
}

// ───────────────────────────────────────────────
// main
// ───────────────────────────────────────────────

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "asio")]
    let host = {
        let asio_id = cpal::available_hosts()
            .into_iter()
            .find(|id| *id == cpal::HostId::Asio);
        match asio_id {
            Some(id) => cpal::host_from_id(id).unwrap_or_else(|_| cpal::default_host()),
            None     => cpal::default_host(),
        }
    };
    #[cfg(not(feature = "asio"))]
    let host = cpal::default_host();

    let input_devices: Vec<String> = host.input_devices()
        .map(|d| d.filter_map(|x| x.name().ok()).collect())
        .unwrap_or_default();
    let output_devices: Vec<String> = host.output_devices()
        .map(|d| d.filter_map(|x| x.name().ok()).collect())
        .unwrap_or_default();

    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;

    // ウィザード
    let config = match run_wizard(&mut terminal, &input_devices, &output_devices)? {
        Some(c) => c,
        None => {
            cleanup(&mut terminal)?;
            return Ok(());
        }
    };

    // オーディオスレッド起動
    let state = Arc::new(SharedState::new());
    let is_send = matches!(config.mode, RunMode::Send | RunMode::Duplex);
    let is_recv = matches!(config.mode, RunMode::Recv | RunMode::Duplex);
    let mut send_thread = None;
    let mut recv_thread = None;

    if is_send {
        let dev = host.input_devices().unwrap()
            .find(|d| d.name().ok().as_deref() == config.input_device.as_deref())
            .or_else(|| host.default_input_device())
            .expect("入力デバイスなし");
        let addr = config.remote_addr.clone();
        let s = Arc::clone(&state);
        send_thread = Some(thread::spawn(move || run_sender(dev, addr, s)));
    }
    if is_recv {
        let dev = host.output_devices().unwrap()
            .find(|d| d.name().ok().as_deref() == config.output_device.as_deref())
            .or_else(|| host.default_output_device())
            .expect("出力デバイスなし");
        let addr = config.listen_addr.clone();
        let s = Arc::clone(&state);
        recv_thread = Some(thread::spawn(move || run_receiver(dev, addr, s)));
    }

    // メインTUI
    run_tui(&mut terminal, &config, &state)?;

    state.running.store(false, Ordering::Relaxed);
    cleanup(&mut terminal)?;
    if let Some(t) = send_thread { let _ = t.join(); }
    if let Some(t) = recv_thread { let _ = t.join(); }
    Ok(())
}

fn cleanup(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<(), Box<dyn std::error::Error>> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
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

        if !event::poll(Duration::from_millis(100))? { continue; }
        match event::read()? {
            Event::Resize(_, _) => { terminal.autoresize()?; }
            Event::Key(key) => match &mut step {

                WizardStep::SelectMode { cursor } => match key.code {
                    KeyCode::Up    => *cursor = cursor.saturating_sub(1),
                    KeyCode::Down  => *cursor = (*cursor + 1).min(2),
                    KeyCode::Enter => {
                        config.mode = match *cursor { 0 => RunMode::Send, 1 => RunMode::Recv, _ => RunMode::Duplex };
                        step = if config.mode != RunMode::Recv {
                            WizardStep::SelectInput  { devices: input_devices.to_vec(),  cursor: 0 }
                        } else {
                            WizardStep::SelectOutput { devices: output_devices.to_vec(), cursor: 0 }
                        };
                    }
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(None),
                    _ => {}
                },

                WizardStep::SelectInput { devices, cursor } => match key.code {
                    KeyCode::Up    => *cursor = cursor.saturating_sub(1),
                    KeyCode::Down  => *cursor = (*cursor + 1).min(devices.len().saturating_sub(1)),
                    KeyCode::Enter => {
                        config.input_device = devices.get(*cursor).cloned();
                        step = if config.mode == RunMode::Duplex {
                            WizardStep::SelectOutput { devices: output_devices.to_vec(), cursor: 0 }
                        } else {
                            WizardStep::EnterIP { buf: String::new() }
                        };
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },

                WizardStep::SelectOutput { devices, cursor } => match key.code {
                    KeyCode::Up    => *cursor = cursor.saturating_sub(1),
                    KeyCode::Down  => *cursor = (*cursor + 1).min(devices.len().saturating_sub(1)),
                    KeyCode::Enter => {
                        config.output_device = devices.get(*cursor).cloned();
                        if config.mode == RunMode::Recv {
                            return Ok(Some(config));   // 受信のみはIPアドレス不要
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
                    KeyCode::Backspace       => { buf.pop(); }
                    KeyCode::Char(c) if c.is_ascii_graphic() => buf.push(c),
                    KeyCode::Esc             => return Ok(None),
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
        .constraints([Constraint::Length(2), Constraint::Min(4), Constraint::Length(2)])
        .split(f.area());

    match step {
        WizardStep::SelectMode { cursor } => {
            f.render_widget(
                Paragraph::new("モードを選択").style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            let modes = ["送信   — このデバイスの音を相手に送る", "受信   — 相手の音をこのデバイスで聴く", "双方向 — 送受信を同時に行う"];
            let items: Vec<ListItem> = modes.iter().enumerate().map(|(i, &m)| {
                let item = ListItem::new(format!("  {}", m));
                if i == *cursor { item.style(Style::default().fg(Color::Black).bg(Color::Cyan)) } else { item }
            }).collect();
            let mut state = ListState::default();
            state.select(Some(*cursor));
            f.render_stateful_widget(List::new(items), layout[1], &mut state);
            f.render_widget(Paragraph::new("↑↓ 選択   Enter 決定   Esc 終了").style(Style::default().fg(Color::DarkGray)), layout[2]);
        }

        WizardStep::SelectInput { devices, cursor } => {
            f.render_widget(
                Paragraph::new("入力デバイス（マイク・インターフェース）を選択").style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            let items: Vec<ListItem> = devices.iter().enumerate().map(|(i, name)| {
                let item = ListItem::new(format!("  {}", name));
                if i == *cursor { item.style(Style::default().fg(Color::Black).bg(Color::Cyan)) } else { item }
            }).collect();
            let mut state = ListState::default();
            state.select(Some(*cursor));
            f.render_stateful_widget(List::new(items), layout[1], &mut state);
            f.render_widget(Paragraph::new("↑↓ 選択   Enter 決定   Esc 終了").style(Style::default().fg(Color::DarkGray)), layout[2]);
        }

        WizardStep::SelectOutput { devices, cursor } => {
            f.render_widget(
                Paragraph::new("出力デバイス（スピーカー・インターフェース）を選択").style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            let items: Vec<ListItem> = devices.iter().enumerate().map(|(i, name)| {
                let item = ListItem::new(format!("  {}", name));
                if i == *cursor { item.style(Style::default().fg(Color::Black).bg(Color::Cyan)) } else { item }
            }).collect();
            let mut state = ListState::default();
            state.select(Some(*cursor));
            f.render_stateful_widget(List::new(items), layout[1], &mut state);
            f.render_widget(Paragraph::new("↑↓ 選択   Enter 決定   Esc 終了").style(Style::default().fg(Color::DarkGray)), layout[2]);
        }

        WizardStep::EnterIP { buf } => {
            f.render_widget(
                Paragraph::new("相手のIPアドレスを入力  （例: 192.168.1.100）").style(Style::default().fg(Color::Yellow)),
                layout[0],
            );
            f.render_widget(
                Paragraph::new(format!("  {}█", buf))
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan))),
                layout[1],
            );
            f.render_widget(Paragraph::new("Enter 決定   Esc 終了").style(Style::default().fg(Color::DarkGray)), layout[2]);
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
    let mode_label = match config.mode { RunMode::Send => "送信", RunMode::Recv => "受信", RunMode::Duplex => "双方向" };
    let is_send = matches!(config.mode, RunMode::Send | RunMode::Duplex);
    let is_recv = matches!(config.mode, RunMode::Recv | RunMode::Duplex);

    loop {
        let sent    = state.packets_sent.load(Ordering::Relaxed);
        let loss    = state.packet_loss.load(Ordering::Relaxed);
        let send_p  = state.send_buffer_pct.load(Ordering::Relaxed).min(100) as u16;
        let recv_p  = state.recv_buffer_pct.load(Ordering::Relaxed).min(100) as u16;

        terminal.draw(|f| {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(3),  // ヘッダー
                    Constraint::Length(4),  // デバイス
                    Constraint::Length(3),  // 送信バッファ
                    Constraint::Length(3),  // 受信バッファ
                    Constraint::Length(3),  // 統計
                ])
                .split(f.area());

            // ヘッダー
            let conn = if is_send { format!("→ {}", config.remote_addr) } else { format!("← {}", config.listen_addr) };
            f.render_widget(
                Paragraph::new(format!(" {}  {}  ● 動作中", mode_label, conn))
                    .block(Block::default().title("Network Audio Bridge").borders(Borders::ALL))
                    .style(Style::default().fg(Color::Cyan)),
                rows[0],
            );

            // デバイス
            f.render_widget(
                Paragraph::new(format!(
                    " 入力 : {}\n 出力 : {}",
                    config.input_device.as_deref().unwrap_or("—"),
                    config.output_device.as_deref().unwrap_or("—"),
                ))
                .block(Block::default().title("デバイス").borders(Borders::ALL)),
                rows[1],
            );

            // 送信バッファ
            f.render_widget(
                Gauge::default()
                    .block(Block::default().title(if is_send { "送信バッファ" } else { "送信バッファ（未使用）" }).borders(Borders::ALL))
                    .gauge_style(Style::default().fg(if is_send { Color::Cyan } else { Color::DarkGray }))
                    .percent(if is_send { send_p } else { 0 }),
                rows[2],
            );

            // 受信バッファ
            f.render_widget(
                Gauge::default()
                    .block(Block::default().title(if is_recv { "受信バッファ" } else { "受信バッファ（未使用）" }).borders(Borders::ALL))
                    .gauge_style(Style::default().fg(if is_recv { Color::Green } else { Color::DarkGray }))
                    .percent(if is_recv { recv_p } else { 0 }),
                rows[3],
            );

            // 統計
            f.render_widget(
                Paragraph::new(format!(" 送信済み: {:>10}   パケットロス: {:>6}   [q] 終了", sent, loss))
                    .block(Block::default().title("統計").borders(Borders::ALL)),
                rows[4],
            );
        })?;

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(k) if k.code == KeyCode::Char('q') => break,
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

fn run_sender(device: cpal::Device, target: String, state: Arc<SharedState>) {
    let ch = CHANNELS as usize;
    let cfg = cpal::StreamConfig {
        channels: CHANNELS,
        sample_rate: cpal::SampleRate(SAMPLE_RATE),
        buffer_size: cpal::BufferSize::Default,
    };
    let socket   = UdpSocket::bind("0.0.0.0:0").expect("送信ソケット失敗");
    let capacity = SAMPLE_RATE as usize * 2 * ch;
    let rb       = HeapRb::<f32>::new(capacity);
    let (mut prod, mut cons) = rb.split();

    let stream = device.build_input_stream(
        &cfg,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            for &s in data { let _ = prod.push(s); }
        },
        |e| eprintln!("入力エラー: {}", e),
        None,
    ).expect("入力ストリーム構築失敗");
    stream.play().expect("入力ストリーム開始失敗");

    let frame = PACKET_SAMPLES * ch;
    let mut buf = vec![0.0f32; frame];
    let mut pkt = vec![0u8; PACKET_BYTES];
    let mut seq: u32 = 0;

    while state.running.load(Ordering::Relaxed) {
        state.send_buffer_pct.store(cons.len() * 100 / capacity, Ordering::Relaxed);
        if cons.len() >= frame {
            let read = cons.pop_slice(&mut buf);
            for s in buf.iter_mut().skip(read) { *s = 0.0; }
            LittleEndian::write_u32(&mut pkt[0..4], seq);
            for (i, &s) in buf.iter().enumerate() {
                LittleEndian::write_f32(&mut pkt[4 + i * 4..4 + (i + 1) * 4], s);
            }
            if socket.send_to(&pkt, &target).is_err() {}
            state.packets_sent.fetch_add(1, Ordering::Relaxed);
            seq = seq.wrapping_add(1);
        } else {
            thread::sleep(Duration::from_micros(500));
        }
    }
}

// ───────────────────────────────────────────────
// オーディオ受信
// ───────────────────────────────────────────────

fn run_receiver(device: cpal::Device, listen: String, state: Arc<SharedState>) {
    let ch = CHANNELS as usize;
    let cfg = cpal::StreamConfig {
        channels: CHANNELS,
        sample_rate: cpal::SampleRate(SAMPLE_RATE),
        buffer_size: cpal::BufferSize::Default,
    };
    let socket = UdpSocket::bind(&listen).expect("受信ソケット失敗");
    socket.set_read_timeout(Some(Duration::from_millis(100))).unwrap();

    let rb = HeapRb::<f32>::new(SAMPLE_RATE as usize * 2 * ch);
    let (mut prod, mut cons) = rb.split();

    let state_cb = Arc::clone(&state);
    let stream = device.build_output_stream(
        &cfg,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let read = cons.pop_slice(data);
            for s in data.iter_mut().skip(read) { *s = 0.0; }
            state_cb.recv_buffer_pct.store(read * 100 / data.len().max(1), Ordering::Relaxed);
        },
        |e| eprintln!("出力エラー: {}", e),
        None,
    ).expect("出力ストリーム構築失敗");
    stream.play().expect("出力ストリーム開始失敗");

    let mut pkt = vec![0u8; PACKET_BYTES];
    let mut expected: Option<u32> = None;

    while state.running.load(Ordering::Relaxed) {
        match socket.recv_from(&mut pkt) {
            Ok((amt, _)) if amt == PACKET_BYTES => {
                let seq = LittleEndian::read_u32(&pkt[0..4]);
                if let Some(exp) = expected {
                    let diff = seq.wrapping_sub(exp);
                    if diff > 0 && diff < u32::MAX / 2 {
                        for _ in 0..(diff as usize * PACKET_SAMPLES * ch) { let _ = prod.push(0.0); }
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
    }
}

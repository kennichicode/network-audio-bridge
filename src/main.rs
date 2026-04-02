use byteorder::{ByteOrder, LittleEndian};
use clap::{Parser, ValueEnum};
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
    widgets::{Block, Borders, Gauge, Paragraph},
    Terminal,
};
use ringbuf::HeapRb;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const PACKET_SAMPLES: usize = 256;

struct SharedState {
    packets_sent: AtomicU64,
    packet_loss: AtomicU64,
    send_buffer_pct: AtomicUsize,
    recv_buffer_pct: AtomicUsize,
    running: AtomicBool,
}

impl SharedState {
    fn new() -> Self {
        Self {
            packets_sent: AtomicU64::new(0),
            packet_loss: AtomicU64::new(0),
            send_buffer_pct: AtomicUsize::new(0),
            recv_buffer_pct: AtomicUsize::new(0),
            running: AtomicBool::new(true),
        }
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about = "Rock-solid Network Audio Bridge", long_about = None)]
struct Args {
    #[arg(short, long)]
    mode: Mode,

    #[arg(long, default_value_t = 48000)]
    sample_rate: u32,

    /// チャンネル数（1=モノ、2=ステレオ、8=7.1ch など）
    #[arg(long, default_value_t = 2)]
    channels: u16,

    #[arg(long, default_value = "127.0.0.1:8000")]
    send_to: String,

    #[arg(long, default_value = "0.0.0.0:8000")]
    listen_on: String,

    /// 送信に使うローカルIPアドレス（WiFiなど特定のインターフェースを選ぶ場合）
    #[arg(long, default_value = "0.0.0.0")]
    bind: String,

    #[arg(long)]
    input_device: Option<String>,

    #[arg(long)]
    output_device: Option<String>,

    #[arg(long, default_value_t = false)]
    list_devices: bool,

    /// ASIO ホストを使用する（Windows専用、--features asio でビルドが必要）
    #[cfg(feature = "asio")]
    #[arg(long, default_value_t = false)]
    asio: bool,
}

#[derive(ValueEnum, Clone, Debug, PartialEq)]
enum Mode {
    Send,
    Recv,
    Duplex,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // ホスト選択（ASIO or デフォルト）
    #[cfg(feature = "asio")]
    let host = if args.asio {
        let asio_id = cpal::available_hosts()
            .into_iter()
            .find(|id| *id == cpal::HostId::Asio)
            .expect("ASIOホストが見つかりません。ASIO4ALL等をインストールしてください。");
        cpal::host_from_id(asio_id).expect("ASIOホストの初期化に失敗しました")
    } else {
        cpal::default_host()
    };

    #[cfg(not(feature = "asio"))]
    let host = cpal::default_host();

    if args.list_devices {
        println!("=== ホスト: {} ===", host.id().name());
        println!("--- 入力デバイス ---");
        if let Ok(devices) = host.input_devices() {
            for (i, dev) in devices.enumerate() {
                let name = dev.name().unwrap_or_else(|_| "Unknown".to_string());
                let configs = dev.supported_input_configs()
                    .map(|c| {
                        c.map(|cfg| format!("{}ch", cfg.channels()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                println!("{}: {} [{}]", i, name, configs);
            }
        }
        println!("--- 出力デバイス ---");
        if let Ok(devices) = host.output_devices() {
            for (i, dev) in devices.enumerate() {
                let name = dev.name().unwrap_or_else(|_| "Unknown".to_string());
                let configs = dev.supported_output_configs()
                    .map(|c| {
                        c.map(|cfg| format!("{}ch", cfg.channels()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                println!("{}: {} [{}]", i, name, configs);
            }
        }
        return Ok(());
    }

    let channels = args.channels;
    let is_send = args.mode == Mode::Send || args.mode == Mode::Duplex;
    let is_recv = args.mode == Mode::Recv || args.mode == Mode::Duplex;

    let state = Arc::new(SharedState::new());
    let mode_str = format!("{:?}", args.mode).to_uppercase();

    let mut send_thread = None;
    let mut recv_thread = None;
    let mut input_device_name = "-".to_string();
    let mut output_device_name = "-".to_string();

    if is_send {
        let send_ip = args.send_to.clone();
        let bind_addr = args.bind.clone();
        let sample_rate = args.sample_rate;
        let state_clone = Arc::clone(&state);

        let device = if let Some(name) = args.input_device.clone() {
            host.input_devices()
                .unwrap()
                .find(|x| x.name().unwrap_or_default() == name)
                .expect("指定した入力デバイスが見つかりません")
        } else {
            host.default_input_device().expect("デフォルト入力デバイスなし")
        };
        input_device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());

        send_thread = Some(thread::spawn(move || {
            run_sender(device, send_ip, bind_addr, sample_rate, channels, state_clone);
        }));
    }

    if is_recv {
        let listen_ip = args.listen_on.clone();
        let sample_rate = args.sample_rate;
        let state_clone = Arc::clone(&state);

        let device = if let Some(name) = args.output_device.clone() {
            host.output_devices()
                .unwrap()
                .find(|x| x.name().unwrap_or_default() == name)
                .expect("指定した出力デバイスが見つかりません")
        } else {
            host.default_output_device().expect("デフォルト出力デバイスなし")
        };
        output_device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());

        recv_thread = Some(thread::spawn(move || {
            run_receiver(device, listen_ip, sample_rate, channels, state_clone);
        }));
    }

    // TUI
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        let packets_sent = state.packets_sent.load(Ordering::Relaxed);
        let packet_loss = state.packet_loss.load(Ordering::Relaxed);
        let send_buf = state.send_buffer_pct.load(Ordering::Relaxed).min(100);
        let recv_buf = state.recv_buffer_pct.load(Ordering::Relaxed).min(100);

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
                    Constraint::Length(3),
                ])
                .split(f.area());

            let title = Paragraph::new(format!(
                " Mode: {}  Channels: {}ch  Status: ● RUNNING",
                mode_str, channels
            ))
            .block(Block::default().title("Network Audio Bridge").borders(Borders::ALL));
            f.render_widget(title, chunks[0]);

            let devices = Paragraph::new(format!(
                " Input:  {}\n Output: {}",
                input_device_name, output_device_name
            ))
            .block(Block::default().title("Devices").borders(Borders::ALL));
            f.render_widget(devices, chunks[1]);

            let send_gauge = Gauge::default()
                .block(Block::default().title("Send Buffer").borders(Borders::ALL))
                .gauge_style(Style::default().fg(Color::Cyan))
                .percent(send_buf as u16);
            f.render_widget(send_gauge, chunks[2]);

            let recv_gauge = Gauge::default()
                .block(Block::default().title("Recv Buffer").borders(Borders::ALL))
                .gauge_style(Style::default().fg(Color::Green))
                .percent(recv_buf as u16);
            f.render_widget(recv_gauge, chunks[3]);

            let stats = Paragraph::new(format!(
                " Packets sent: {:>12}    Packet loss: {:>8}",
                packets_sent, packet_loss
            ))
            .block(Block::default().title("Stats").borders(Borders::ALL));
            f.render_widget(stats, chunks[4]);

            let help = Paragraph::new(" [q] Quit")
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(help, chunks[5]);
        })?;

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.code == KeyCode::Char('q') => {
                    state.running.store(false, Ordering::Relaxed);
                    break;
                }
                Event::Resize(_, _) => {
                    terminal.autoresize()?;
                }
                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    if let Some(t) = send_thread {
        let _ = t.join();
    }
    if let Some(t) = recv_thread {
        let _ = t.join();
    }

    Ok(())
}

fn run_sender(
    device: cpal::Device,
    target_addr: String,
    bind_addr: String,
    sample_rate: u32,
    channels: u16,
    state: Arc<SharedState>,
) {
    let ch = channels as usize;
    let packet_bytes_size = 4 + (PACKET_SAMPLES * ch * 4);

    let config = cpal::StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let socket = UdpSocket::bind(format!("{}:0", bind_addr))
        .expect("送信ソケットのバインドに失敗しました");

    let capacity = sample_rate as usize * 2 * ch;
    let rb = HeapRb::<f32>::new(capacity);
    let (mut prod, mut cons) = rb.split();

    let err_fn = |err| eprintln!("入力ストリームエラー: {}", err);

    let stream = device
        .build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                for &sample in data {
                    let _ = prod.push(sample);
                }
            },
            err_fn,
            None,
        )
        .expect("入力ストリームのビルドに失敗しました");

    stream.play().expect("入力ストリームの再生開始に失敗しました");

    let frame_size = PACKET_SAMPLES * ch;
    let mut local_buf = vec![0.0f32; frame_size];
    let mut packet_bytes = vec![0u8; packet_bytes_size];
    let mut seq_num: u32 = 0;

    while state.running.load(Ordering::Relaxed) {
        state
            .send_buffer_pct
            .store(cons.len() * 100 / capacity, Ordering::Relaxed);

        if cons.len() >= frame_size {
            let read = cons.pop_slice(&mut local_buf);
            // pop_sliceが返した分だけ送信（不足分は前フレームのデータではなくゼロ埋め）
            for i in read..frame_size {
                local_buf[i] = 0.0;
            }

            LittleEndian::write_u32(&mut packet_bytes[0..4], seq_num);
            for (i, &sample) in local_buf.iter().enumerate() {
                LittleEndian::write_f32(&mut packet_bytes[4 + i * 4..4 + (i + 1) * 4], sample);
            }

            if let Err(e) = socket.send_to(&packet_bytes, &target_addr) {
                eprintln!("UDPパケット送信失敗: {}", e);
            }
            state.packets_sent.fetch_add(1, Ordering::Relaxed);
            seq_num = seq_num.wrapping_add(1);
        } else {
            thread::sleep(Duration::from_micros(500));
        }
    }
}

fn run_receiver(
    device: cpal::Device,
    listen_addr: String,
    sample_rate: u32,
    channels: u16,
    state: Arc<SharedState>,
) {
    let ch = channels as usize;
    let packet_bytes_size = 4 + (PACKET_SAMPLES * ch * 4);

    let config = cpal::StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let socket = UdpSocket::bind(&listen_addr).expect("受信ソケットのバインドに失敗しました");
    socket
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();

    let capacity = sample_rate as usize * 2 * ch;
    let rb = HeapRb::<f32>::new(capacity);
    let (mut prod, mut cons) = rb.split();

    let state_for_cb = Arc::clone(&state);
    let err_fn = |err| eprintln!("出力ストリームエラー: {}", err);

    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let read_len = cons.pop_slice(data);
                for sample in data.iter_mut().skip(read_len) {
                    *sample = 0.0;
                }
                // 満足率：要求サンプルのうち何%をリングバッファから供給できたか
                let pct = read_len * 100 / data.len().max(1);
                state_for_cb
                    .recv_buffer_pct
                    .store(pct.min(100), Ordering::Relaxed);
            },
            err_fn,
            None,
        )
        .expect("出力ストリームのビルドに失敗しました");

    stream.play().expect("出力ストリームの再生開始に失敗しました");

    let mut packet_bytes = vec![0u8; packet_bytes_size];
    let mut expected_seq: Option<u32> = None;

    while state.running.load(Ordering::Relaxed) {
        match socket.recv_from(&mut packet_bytes) {
            Ok((amt, _)) if amt == packet_bytes_size => {
                let seq_num = LittleEndian::read_u32(&packet_bytes[0..4]);

                if let Some(exp) = expected_seq {
                    // wrap-around を考慮した差分計算
                    let diff = seq_num.wrapping_sub(exp);
                    if diff > 0 && diff < u32::MAX / 2 {
                        let lost_samples = (diff as usize) * PACKET_SAMPLES * ch;
                        for _ in 0..lost_samples {
                            let _ = prod.push(0.0);
                        }
                        state
                            .packet_loss
                            .fetch_add(diff as u64, Ordering::Relaxed);
                    }
                }

                expected_seq = Some(seq_num.wrapping_add(1));

                for i in 0..(PACKET_SAMPLES * ch) {
                    let sample =
                        LittleEndian::read_f32(&packet_bytes[4 + i * 4..4 + (i + 1) * 4]);
                    let _ = prod.push(sample);
                }
            }
            Ok(_) => {} // サイズ不一致は無視
            Err(_) => {} // timeout → running フラグを再チェック
        }
    }
}

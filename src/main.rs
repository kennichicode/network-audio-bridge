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
const CHANNELS: usize = 2;
const PACKET_BYTES: usize = 4 + (PACKET_SAMPLES * CHANNELS * 4);

struct SharedState {
    packets_sent: AtomicU64,
    packet_loss: AtomicU64,
    buffer_fill_pct: AtomicUsize,
    running: AtomicBool,
}

impl SharedState {
    fn new() -> Self {
        Self {
            packets_sent: AtomicU64::new(0),
            packet_loss: AtomicU64::new(0),
            buffer_fill_pct: AtomicUsize::new(0),
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

    #[arg(long, default_value = "127.0.0.1:8000")]
    send_to: String,

    #[arg(long, default_value = "0.0.0.0:8000")]
    listen_on: String,

    #[arg(long)]
    input_device: Option<String>,

    #[arg(long)]
    output_device: Option<String>,

    #[arg(long, default_value_t = false)]
    list_devices: bool,
}

#[derive(ValueEnum, Clone, Debug, PartialEq)]
enum Mode {
    Send,
    Recv,
    Duplex,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let host = cpal::default_host();

    if args.list_devices {
        println!("--- Input Devices ---");
        if let Ok(devices) = host.input_devices() {
            for (i, dev) in devices.enumerate() {
                println!("{}: {}", i, dev.name().unwrap_or_else(|_| "Unknown".to_string()));
            }
        }
        println!("--- Output Devices ---");
        if let Ok(devices) = host.output_devices() {
            for (i, dev) in devices.enumerate() {
                println!("{}: {}", i, dev.name().unwrap_or_else(|_| "Unknown".to_string()));
            }
        }
        return Ok(());
    }

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
        let sample_rate = args.sample_rate;
        let state_clone = Arc::clone(&state);

        let device = if let Some(name) = args.input_device.clone() {
            host.input_devices()
                .unwrap()
                .find(|x| x.name().unwrap_or_default() == name)
                .expect("Input device not found")
        } else {
            host.default_input_device().expect("No default input device")
        };
        input_device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());

        send_thread = Some(thread::spawn(move || {
            run_sender(device, send_ip, sample_rate, state_clone);
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
                .expect("Output device not found")
        } else {
            host.default_output_device().expect("No default output device")
        };
        output_device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());

        recv_thread = Some(thread::spawn(move || {
            run_receiver(device, listen_ip, sample_rate, state_clone);
        }));
    }

    // TUI setup
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        let packets_sent = state.packets_sent.load(Ordering::Relaxed);
        let packet_loss = state.packet_loss.load(Ordering::Relaxed);
        let buffer_fill = state.buffer_fill_pct.load(Ordering::Relaxed).min(100);

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
                ])
                .split(f.area());

            let title = Paragraph::new(format!(
                " Mode: {}    Status: ● RUNNING",
                mode_str
            ))
            .block(Block::default().title("Network Audio Bridge").borders(Borders::ALL));
            f.render_widget(title, chunks[0]);

            let devices = Paragraph::new(format!(
                " Input:  {}    Output: {}",
                input_device_name, output_device_name
            ))
            .block(Block::default().title("Devices").borders(Borders::ALL));
            f.render_widget(devices, chunks[1]);

            let gauge = Gauge::default()
                .block(Block::default().title("Buffer").borders(Borders::ALL))
                .gauge_style(Style::default().fg(Color::Cyan))
                .percent(buffer_fill as u16);
            f.render_widget(gauge, chunks[2]);

            let stats = Paragraph::new(format!(
                " Packets sent: {:>12}    Packet loss: {:>8}",
                packets_sent, packet_loss
            ))
            .block(Block::default().title("Stats").borders(Borders::ALL));
            f.render_widget(stats, chunks[3]);

            let help = Paragraph::new(" [q] Quit")
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(help, chunks[4]);
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    state.running.store(false, Ordering::Relaxed);
                    break;
                }
            }
        }
    }

    // TUI cleanup
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
    sample_rate: u32,
    state: Arc<SharedState>,
) {
    let config = cpal::StreamConfig {
        channels: CHANNELS as cpal::ChannelCount,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let socket = UdpSocket::bind("0.0.0.0:0").expect("Failed to bind sender socket");
    let capacity = sample_rate as usize * 2;
    let rb = HeapRb::<f32>::new(capacity);
    let (mut prod, mut cons) = rb.split();

    let err_fn = |err| eprintln!("Input stream error: {}", err);

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
        .expect("Failed to build input stream");

    stream.play().expect("Failed to play input stream");

    let mut seq_num: u32 = 0;
    let mut local_buf = vec![0.0f32; PACKET_SAMPLES * CHANNELS];
    let mut packet_bytes = vec![0u8; PACKET_BYTES];

    while state.running.load(Ordering::Relaxed) {
        state
            .buffer_fill_pct
            .store(cons.len() * 100 / capacity, Ordering::Relaxed);

        if cons.len() >= local_buf.len() {
            cons.pop_slice(&mut local_buf);

            LittleEndian::write_u32(&mut packet_bytes[0..4], seq_num);
            for (i, &sample) in local_buf.iter().enumerate() {
                LittleEndian::write_f32(&mut packet_bytes[4 + i * 4..4 + (i + 1) * 4], sample);
            }

            if let Err(e) = socket.send_to(&packet_bytes, &target_addr) {
                eprintln!("Failed to send UDP packet: {}", e);
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
    state: Arc<SharedState>,
) {
    let config = cpal::StreamConfig {
        channels: CHANNELS as cpal::ChannelCount,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let socket = UdpSocket::bind(&listen_addr).expect("Failed to bind receiver socket");
    socket
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();

    let rb = HeapRb::<f32>::new(sample_rate as usize * 2);
    let (mut prod, mut cons) = rb.split();

    let err_fn = |err| eprintln!("Output stream error: {}", err);

    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let read_len = cons.pop_slice(data);
                for sample in data.iter_mut().skip(read_len) {
                    *sample = 0.0;
                }
            },
            err_fn,
            None,
        )
        .expect("Failed to build output stream");

    stream.play().expect("Failed to play output stream");

    let mut packet_bytes = vec![0u8; PACKET_BYTES];
    let mut expected_seq: Option<u32> = None;

    while state.running.load(Ordering::Relaxed) {
        match socket.recv_from(&mut packet_bytes) {
            Ok((amt, _)) => {
                if amt == PACKET_BYTES {
                    let seq_num = LittleEndian::read_u32(&packet_bytes[0..4]);

                    if let Some(exp) = expected_seq {
                        if seq_num > exp {
                            let lost_packets = seq_num - exp;
                            let lost_samples =
                                (lost_packets as usize) * PACKET_SAMPLES * CHANNELS;
                            for _ in 0..lost_samples {
                                let _ = prod.push(0.0);
                            }
                            state
                                .packet_loss
                                .fetch_add(lost_packets as u64, Ordering::Relaxed);
                        }
                    }

                    expected_seq = Some(seq_num.wrapping_add(1));

                    for i in 0..(PACKET_SAMPLES * CHANNELS) {
                        let sample = LittleEndian::read_f32(
                            &packet_bytes[4 + i * 4..4 + (i + 1) * 4],
                        );
                        let _ = prod.push(sample);
                    }
                }
            }
            Err(_) => {} // timeout → running フラグを再チェック
        }
    }
}

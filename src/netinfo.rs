use std::net::{IpAddr, UdpSocket};

pub fn local_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    // UDP の connect() はパケットを送らずルーティング決定だけ行うので、
    // オフラインでも local_addr() からデフォルト経路の IP が取れる
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

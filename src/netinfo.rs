#![allow(dead_code)]

use std::net::Ipv4Addr;

// 優先度:
// 1. 192.168.x.x   (家庭・小規模LANで最も一般的)
// 2. 10.0.0.0/8 で 100.x を除く   (Tailscale の 100.64/10 を後ろに)
// 3. 172.16.0.0/12
// 4. 100.64.0.0/10 (CGNAT / Tailscale)
// 5. その他
fn priority(ip: &Ipv4Addr) -> u8 {
    let o = ip.octets();
    if o[0] == 192 && o[1] == 168 { 1 }
    else if o[0] == 10 { 2 }
    else if o[0] == 172 && (16..=31).contains(&o[1]) { 3 }
    else if o[0] == 100 && (64..=127).contains(&o[1]) { 4 }
    else { 5 }
}

pub fn candidate_ipv4s() -> Vec<Ipv4Addr> {
    let mut v: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .ok()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) => Some(v4.ip),
            _ => None,
        })
        .filter(|ip| !ip.is_link_local() && !ip.is_unspecified())
        .collect();
    v.sort_by_key(priority);
    v.dedup();
    v
}

pub fn local_ip() -> Option<std::net::IpAddr> {
    candidate_ipv4s().into_iter().next().map(std::net::IpAddr::V4)
}

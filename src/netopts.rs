use std::net::UdpSocket;

#[cfg(windows)]
pub fn disable_udp_connreset(socket: &UdpSocket) -> std::io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{WSAIoctl, SIO_UDP_CONNRESET, SOCKET};
    let raw = socket.as_raw_socket() as SOCKET;
    let enable: u32 = 0;
    let mut bytes_returned: u32 = 0;
    let rc = unsafe {
        WSAIoctl(
            raw,
            SIO_UDP_CONNRESET,
            &enable as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
            std::ptr::null_mut(),
            0,
            &mut bytes_returned,
            std::ptr::null_mut(),
            None,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(windows))]
pub fn disable_udp_connreset(_socket: &UdpSocket) -> std::io::Result<()> {
    Ok(())
}

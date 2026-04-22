use std::net::UdpSocket;

#[cfg(windows)]
#[link(name = "ws2_32")]
unsafe extern "system" {
    fn WSAIoctl(
        s: usize,
        dw_io_control_code: u32,
        lpv_in_buffer: *const core::ffi::c_void,
        cb_in_buffer: u32,
        lpv_out_buffer: *mut core::ffi::c_void,
        cb_out_buffer: u32,
        lpcb_bytes_returned: *mut u32,
        lp_overlapped: *mut core::ffi::c_void,
        lp_completion_routine: *mut core::ffi::c_void,
    ) -> i32;
}

// IOC_IN | IOC_VENDOR | 12 = 0x9800_000C
#[cfg(windows)]
const SIO_UDP_CONNRESET: u32 = 0x9800_000C;

#[cfg(windows)]
pub fn disable_udp_connreset(socket: &UdpSocket) -> std::io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    let raw = socket.as_raw_socket() as usize;
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
            std::ptr::null_mut(),
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

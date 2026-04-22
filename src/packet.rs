#![allow(dead_code)]

use byteorder::{ByteOrder, LittleEndian};

pub const MAGIC: [u8; 4] = [b'N', b'A', b'B', b'1'];
pub const PROTOCOL_VERSION: u8 = 1;
pub const HEADER_BYTES: usize = 16;
pub const PACKET_SAMPLES: usize = 128;

pub const fn packet_bytes(channels: usize) -> usize {
    HEADER_BYTES + PACKET_SAMPLES * channels * 4
}

pub struct Header {
    pub channels: u8,
    pub sample_rate: u32,
    pub seq: u32,
}

pub fn write_header(buf: &mut [u8], sr: u32, channels: u8, seq: u32) {
    buf[0..4].copy_from_slice(&MAGIC);
    buf[4] = PROTOCOL_VERSION;
    buf[5] = channels;
    LittleEndian::write_u32(&mut buf[6..10], sr);
    buf[10] = 0;
    buf[11] = 0;
    LittleEndian::write_u32(&mut buf[12..16], seq);
}

pub enum ParseError {
    BadMagic,
    UnsupportedVersion(u8),
    SampleRateMismatch { got: u32, expected: u32 },
    ChannelsMismatch { got: u8, expected: u8 },
}

pub fn parse_header(buf: &[u8], expected_sr: u32, expected_ch: u8) -> Result<Header, ParseError> {
    if buf.len() < HEADER_BYTES {
        return Err(ParseError::BadMagic);
    }
    if buf[0..4] != MAGIC {
        return Err(ParseError::BadMagic);
    }
    let version = buf[4];
    if version != PROTOCOL_VERSION {
        return Err(ParseError::UnsupportedVersion(version));
    }
    let channels = buf[5];
    if channels != expected_ch {
        return Err(ParseError::ChannelsMismatch { got: channels, expected: expected_ch });
    }
    let sample_rate = LittleEndian::read_u32(&buf[6..10]);
    if sample_rate != expected_sr {
        return Err(ParseError::SampleRateMismatch { got: sample_rate, expected: expected_sr });
    }
    let seq = LittleEndian::read_u32(&buf[12..16]);
    Ok(Header { channels, sample_rate, seq })
}

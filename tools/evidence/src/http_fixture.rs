//! Bounded loopback fixture request framing shared by evidence endpoints.

use std::{
    io::Read as _,
    net::TcpStream,
    time::{Duration, Instant},
};

/// Reads one content-length request within the fixture byte bound.
pub fn read_request(stream: &mut TcpStream) -> std::io::Result<String> {
    const MAX_REQUEST_BYTES: usize = 1024 * 1024;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "fixture request deadline")
            })?;
        stream.set_read_timeout(Some(remaining))?;
        stream.set_write_timeout(Some(remaining))?;
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "truncated fixture request",
            ));
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(std::io::Error::other("fixture request exceeds byte bound"));
        }
        if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..header_end + 4])
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|value| value.parse::<usize>().ok())
                })
                .unwrap_or(0);
            let end = (header_end + 4)
                .checked_add(content_length)
                .filter(|end| *end <= MAX_REQUEST_BYTES)
                .ok_or_else(|| std::io::Error::other("fixture body exceeds byte bound"))?;
            if bytes.len() >= end {
                bytes.truncate(end);
                break;
            }
        }
    }
    String::from_utf8(bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

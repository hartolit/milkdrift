//! Bounded loopback fixture request framing shared by evidence endpoints.

use std::{
    io::Read as _,
    net::TcpStream,
    time::{Duration, Instant},
};

/// Reads one content-length request within the fixture byte bound.
pub fn read_request(stream: &mut TcpStream) -> std::io::Result<String> {
    const MAX_REQUEST_BYTES: usize = 1024 * 1024;
    // Accepted sockets can inherit the listener's nonblocking mode on Windows.
    stream.set_nonblocking(false)?;
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
                    line.split_once(':')
                        .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write as _, net::TcpListener, sync::mpsc, thread};

    #[test]
    fn nonblocking_accepted_socket_waits_for_fragmented_request() -> std::io::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let (accepted, ready) = mpsc::sync_channel(1);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept()?;
            stream.set_nonblocking(true)?;
            accepted.send(()).map_err(std::io::Error::other)?;
            read_request(&mut stream)
        });
        let mut client = TcpStream::connect(address)?;
        ready
            .recv_timeout(Duration::from_secs(2))
            .map_err(std::io::Error::other)?;
        thread::sleep(Duration::from_millis(20));
        client.write_all(b"POST / HTTP/1.1\r\nContent-Length:3\r\n\r\na")?;
        thread::sleep(Duration::from_millis(20));
        client.write_all(b"bc")?;
        let request = server
            .join()
            .map_err(|_| std::io::Error::other("fixture reader panicked"))??;
        assert_eq!(request, "POST / HTTP/1.1\r\nContent-Length:3\r\n\r\nabc");
        Ok(())
    }
}

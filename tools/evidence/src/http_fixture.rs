//! Bounded loopback fixture request framing shared by evidence endpoints.

use std::{
    io::Read as _,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Owns a recurring loopback endpoint for the evidence binaries' controlled model responses.
///
/// The handler must use bounded I/O (normally [`read_request`]) and must not wait indefinitely
/// on external work. Call [`Self::finish`] before accepting scenario evidence; Drop only covers
/// cancellation/unwind. A failed finish does not prove the worker stopped.
pub struct LoopbackServer {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<std::io::Result<()>>>,
    failure: Option<(std::io::ErrorKind, String)>,
    finish_attempted: bool,
}

impl LoopbackServer {
    /// Binds a fresh loopback port and serves one connection at a time on an owned thread.
    ///
    /// # Errors
    /// Returns listener binding, address discovery, or nonblocking configuration failure.
    pub fn start(
        mut respond: impl FnMut(&mut TcpStream) -> std::io::Result<()> + Send + 'static,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => respond(&mut stream)?,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        });
        Ok(Self {
            address,
            stop,
            worker: Some(worker),
            failure: None,
            finish_attempted: false,
        })
    }

    /// The listener's immutable bound address.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    /// Stops accepting and joins the worker within the fixture read-plus-write allowance.
    ///
    /// # Errors
    /// Returns a handler/listener error, worker panic, or unconfirmed stop after 21 seconds.
    /// The first failure remains visible on later calls, even if the worker subsequently stops.
    pub fn finish(&mut self) -> std::io::Result<()> {
        self.finish_attempted = true;
        self.stop.store(true, Ordering::SeqCst);
        if let Err(error) = self.join()
            && self.failure.is_none()
        {
            self.failure = Some((
                error.kind(),
                milkdrift_contracts::truncate_utf8(&error.to_string(), 1024).to_owned(),
            ));
        }
        match &self.failure {
            Some((kind, message)) => Err(std::io::Error::new(*kind, message.clone())),
            None => Ok(()),
        }
    }

    fn join(&mut self) -> std::io::Result<()> {
        let started = Instant::now();
        while self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            if started.elapsed() >= Duration::from_secs(21) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "fixture worker stop unconfirmed",
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| std::io::Error::other("fixture worker panicked"))??;
        }
        Ok(())
    }
}

impl Drop for LoopbackServer {
    fn drop(&mut self) {
        if !self.finish_attempted
            && let Err(error) = self.finish()
        {
            #[expect(
                clippy::print_stderr,
                reason = "Cancellation/unwind cannot return fixture cleanup failure; explicit finish is required before accepting evidence."
            )]
            {
                eprintln!("evidence loopback fixture cleanup unconfirmed: {error}");
            }
        }
    }
}

/// Reads one content-length request within the fixture byte bound.
///
/// # Errors
/// Returns socket configuration/read failure, timeout, truncated or oversized framing, and
/// invalid UTF-8 in headers or the captured request.
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
        bytes.extend_from_slice(
            buffer
                .get(..read)
                .ok_or_else(|| std::io::Error::other("fixture read exceeds buffer"))?,
        );
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(std::io::Error::other("fixture request exceeds byte bound"));
        }
        if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = std::str::from_utf8(
                bytes
                    .get(..header_end + 4)
                    .ok_or_else(|| std::io::Error::other("fixture headers exceed request"))?,
            )
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
    fn recurring_server_joins_and_releases_handler_state() -> std::io::Result<()> {
        for entered in [false, true, false, true] {
            let hold = Arc::new(());
            let released = Arc::downgrade(&hold);
            let mut server = LoopbackServer::start(move |stream| {
                assert_eq!(Arc::strong_count(&hold), 1);
                read_request(stream)?;
                stream.write_all(b"done")
            })?;
            assert!(released.upgrade().is_some());
            if entered {
                let mut client = TcpStream::connect(server.address())?;
                client.set_read_timeout(Some(Duration::from_secs(2)))?;
                client.write_all(b"GET / HTTP/1.1\r\nContent-Length: 0\r\n\r\n")?;
                let mut reply = [0; 4];
                client.read_exact(&mut reply)?;
                assert_eq!(&reply, b"done");
            }
            server.finish()?;
            assert!(released.upgrade().is_none());
            server.finish()?;
        }
        Ok(())
    }

    #[test]
    fn recurring_server_preserves_handler_errors_and_panics_on_repeated_finish()
    -> std::io::Result<()> {
        for panic in [false, true] {
            let (entered, ready) = mpsc::sync_channel(1);
            let mut server = LoopbackServer::start(move |_| {
                entered.send(()).map_err(std::io::Error::other)?;
                if panic {
                    #[expect(
                        clippy::panic,
                        reason = "Inject an abnormal worker exit at the actual fixture owner to verify join evidence survives repeated finish."
                    )]
                    {
                        panic!("injected fixture panic");
                    }
                }
                Err(std::io::Error::other("injected handler failure"))
            })?;
            let _client = TcpStream::connect(server.address())?;
            ready
                .recv_timeout(Duration::from_secs(2))
                .map_err(std::io::Error::other)?;
            let error = server
                .finish()
                .err()
                .ok_or_else(|| std::io::Error::other("worker failure was discarded"))?;
            assert_eq!(error.kind(), std::io::ErrorKind::Other);
            assert!(error.to_string().contains(if panic {
                "panicked"
            } else {
                "injected handler failure"
            }));
            let repeated = server
                .finish()
                .err()
                .ok_or_else(|| std::io::Error::other("repeated finish erased failure"))?;
            assert_eq!(repeated.to_string(), error.to_string());
        }
        Ok(())
    }

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

use thiserror::Error;

/// Strict bounded SSE parser accepting only UTF-8 field lines and complete events.
pub(crate) struct SseParser {
    pending: Vec<u8>,
    data: Vec<u8>,
    max_line: usize,
    max_event: usize,
}

impl SseParser {
    pub(crate) fn new(max_line: u32, max_event: u32) -> Self {
        Self {
            pending: Vec::new(),
            data: Vec::new(),
            max_line: max_line as usize,
            max_event: max_event as usize,
        }
    }

    pub(crate) fn push(
        &mut self,
        bytes: &[u8],
        mut emit: impl FnMut(&str) -> Result<(), StreamError>,
    ) -> Result<(), StreamError> {
        self.pending.extend_from_slice(bytes);
        if self.pending.len() > self.max_line && !self.pending.contains(&b'\n') {
            return Err(StreamError::LineTooLarge);
        }
        while let Some(position) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line = self.pending.drain(..=position).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.len() > self.max_line {
                return Err(StreamError::LineTooLarge);
            }
            let line = std::str::from_utf8(&line).map_err(|_| StreamError::InvalidUtf8)?;
            if line.is_empty() {
                if !self.data.is_empty() {
                    if self.data.last() == Some(&b'\n') {
                        self.data.pop();
                    }
                    let data = std::str::from_utf8(&self.data)
                        .map_err(|_| StreamError::InvalidUtf8)?
                        .to_owned();
                    self.data.clear();
                    emit(&data)?;
                }
            } else if let Some(value) = line.strip_prefix("data:") {
                let value = value.strip_prefix(' ').unwrap_or(value);
                self.data.extend_from_slice(value.as_bytes());
                self.data.push(b'\n');
                if self.data.len() > self.max_event {
                    return Err(StreamError::EventTooLarge);
                }
            } else if line.starts_with(':')
                || line.starts_with("event:")
                || line.starts_with("id:")
                || line.starts_with("retry:")
            {
                // Known SSE fields are bounded but mapping is data-event driven.
            } else {
                return Err(StreamError::MalformedField);
            }
        }
        Ok(())
    }

    pub(crate) fn finish(&self) -> Result<(), StreamError> {
        if self.pending.is_empty() && self.data.is_empty() {
            Ok(())
        } else {
            Err(StreamError::Truncated)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub(crate) enum StreamError {
    #[error("SSE line exceeds configured bound")]
    LineTooLarge,
    #[error("SSE event exceeds configured bound")]
    EventTooLarge,
    #[error("SSE contains invalid UTF-8")]
    InvalidUtf8,
    #[error("SSE field is malformed or unsupported")]
    MalformedField,
    #[error("SSE ended with a truncated line or event")]
    Truncated,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_bounds_utf8_and_truncation_fail_closed() {
        let mut parser = SseParser::new(4, 8);
        assert_eq!(
            parser.push(b"data: too-long\n\n", |_| Ok(())),
            Err(StreamError::LineTooLarge)
        );
        let mut parser = SseParser::new(64, 64);
        assert_eq!(
            parser.push(&[b'd', b'a', b't', b'a', b':', b' ', 0xff, b'\n'], |_| Ok(
                ()
            )),
            Err(StreamError::InvalidUtf8)
        );
        let mut parser = SseParser::new(64, 64);
        assert!(parser.push(b"data: unfinished", |_| Ok(())).is_ok());
        assert_eq!(parser.finish(), Err(StreamError::Truncated));
    }
}

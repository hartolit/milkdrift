//! Incremental event-stream lines in wire order. CRLF, LF and CR follow WHATWG's
//! parsing rules; incomplete EOF is discarded by dropping the connection's decoder.
//! Milkdrift additionally requires strict UTF-8 and a valid JSON observation envelope.

use crate::ClientError;
use milkdrift_control_protocol::{MAX_DOCUMENT_BYTES, ObservationEnvelope, decode_json};

const MAX_FRAME_BYTES: usize = MAX_DOCUMENT_BYTES * 2;

#[derive(Default)]
pub(super) struct Decoder {
    line: Vec<u8>,
    data: String,
    frame_bytes: usize,
    skip_lf: bool,
    started: bool,
}

impl Decoder {
    pub(super) fn push(&mut self, byte: u8) -> Result<Option<ObservationEnvelope>, ClientError> {
        if std::mem::take(&mut self.skip_lf) && byte == b'\n' {
            if self.frame_bytes != 0 {
                self.count_byte()?;
            }
            return Ok(None);
        }
        self.count_byte()?;
        if matches!(byte, b'\r' | b'\n') {
            self.skip_lf = byte == b'\r';
            self.finish_line()
        } else {
            self.line.push(byte);
            Ok(None)
        }
    }

    fn count_byte(&mut self) -> Result<(), ClientError> {
        self.frame_bytes += 1;
        if self.frame_bytes > MAX_FRAME_BYTES {
            return Err(ClientError::Stream(
                "SSE frame exceeds client bound".to_owned(),
            ));
        }
        Ok(())
    }

    fn finish_line(&mut self) -> Result<Option<ObservationEnvelope>, ClientError> {
        let text = std::str::from_utf8(&self.line)
            .map_err(|_| ClientError::Stream("SSE line is not UTF-8".to_owned()))?;
        let text = if std::mem::replace(&mut self.started, true) {
            text
        } else {
            text.strip_prefix('\u{feff}').unwrap_or(text)
        };
        let observation = if text.is_empty() {
            self.frame_bytes = 0;
            if self.data.is_empty() {
                None
            } else {
                self.data.pop(); // Each data field contributes exactly one trailing LF.
                let observation: ObservationEnvelope = decode_json(self.data.as_bytes())?;
                observation.protocol.negotiate()?;
                self.data.clear();
                Some(observation)
            }
        } else {
            // Comments and unknown fields, including EventSource's id/retry fields,
            // confer no cursor or retry authority on this JSON protocol client.
            let (field, value) = text.split_once(':').unwrap_or((text, ""));
            if field == "data" {
                self.data.push_str(value.strip_prefix(' ').unwrap_or(value));
                self.data.push('\n');
            }
            None
        };
        self.line.clear();
        Ok(observation)
    }
}

#[cfg(test)]
mod tests;

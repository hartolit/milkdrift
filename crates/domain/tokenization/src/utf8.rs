//! Incremental UTF-8 validation across token boundaries.

use core::str;

use crate::{TextSink, TokenizationError};

/// Allocation-free UTF-8 decoder retaining at most one incomplete code point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IncrementalUtf8Decoder {
    pending: [u8; 4],
    pending_length: u8,
    expected_length: u8,
}

impl IncrementalUtf8Decoder {
    /// Creates an empty decoder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending: [0; 4],
            pending_length: 0,
            expected_length: 0,
        }
    }

    /// Returns the number of bytes waiting for completion.
    #[must_use]
    pub const fn pending_bytes(&self) -> u8 {
        self.pending_length
    }

    /// Clears incomplete state.
    pub const fn reset(&mut self) {
        self.pending = [0; 4];
        self.pending_length = 0;
        self.expected_length = 0;
    }

    /// Validates and emits complete UTF-8 fragments into a caller-owned sink.
    ///
    /// # Errors
    ///
    /// Returns [`TokenizationError::CapacityExhausted`] before consuming input if
    /// the sink cannot hold all pending and supplied bytes. Returns
    /// [`TokenizationError::InvalidUtf8`] if the input contains an invalid UTF-8
    /// sequence, or propagates an error reported by the sink.
    pub fn push_bytes<S: TextSink>(
        &mut self,
        bytes: &[u8],
        output: &mut S,
    ) -> Result<(), TokenizationError> {
        let required = usize::from(self.pending_length).saturating_add(bytes.len());
        if required > output.remaining_capacity() {
            return Err(domain_contracts::CapacityExhausted::new(
                domain_contracts::CapacityResource::DecodeBytes,
                required as u64,
                output.remaining_capacity() as u64,
            )
            .into());
        }

        self.flush_complete(output)?;
        for &byte in bytes {
            self.push_byte(byte, output)?;
        }
        Ok(())
    }

    /// Verifies that no incomplete code point remains.
    ///
    /// # Errors
    ///
    /// Returns [`TokenizationError::IncompleteUtf8`] when bytes from an unfinished
    /// code point remain buffered.
    pub const fn finish(&self) -> Result<(), TokenizationError> {
        if self.pending_length == 0 {
            Ok(())
        } else {
            Err(TokenizationError::IncompleteUtf8 {
                buffered_bytes: self.pending_length,
            })
        }
    }

    fn push_byte<S: TextSink>(
        &mut self,
        byte: u8,
        output: &mut S,
    ) -> Result<(), TokenizationError> {
        if self.pending_length == 0 {
            if byte.is_ascii() {
                self.pending[0] = byte;
                self.pending_length = 1;
                self.expected_length = 1;
                return self.flush_complete(output);
            }

            self.expected_length = match byte {
                0xC2..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF4 => 4,
                _ => return Err(TokenizationError::InvalidUtf8 { byte }),
            };
            self.pending[0] = byte;
            self.pending_length = 1;
            return Ok(());
        }

        if byte & 0b1100_0000 != 0b1000_0000 {
            self.reset();
            return Err(TokenizationError::InvalidUtf8 { byte });
        }

        let position = usize::from(self.pending_length);
        let Some(slot) = self.pending.get_mut(position) else {
            self.reset();
            return Err(TokenizationError::InvalidUtf8 { byte });
        };
        *slot = byte;
        self.pending_length = self.pending_length.saturating_add(1);

        if self.pending_length != self.expected_length {
            return Ok(());
        }

        self.flush_complete(output)
    }

    fn flush_complete<S: TextSink>(&mut self, output: &mut S) -> Result<(), TokenizationError> {
        if self.pending_length == 0 || self.pending_length != self.expected_length {
            return Ok(());
        }

        let length = usize::from(self.expected_length);
        let Some(fragment) = self.pending.get(..length) else {
            self.reset();
            return Err(TokenizationError::InvalidUtf8 { byte: 0 });
        };
        let Ok(text) = str::from_utf8(fragment) else {
            let byte = fragment.last().copied().unwrap_or(0);
            self.reset();
            return Err(TokenizationError::InvalidUtf8 { byte });
        };
        output.push_str(text)?;
        self.reset();
        Ok(())
    }
}

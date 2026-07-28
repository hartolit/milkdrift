//! Monomorphized output sinks backed by caller-owned flat slices.

use core::str;

use domain_contracts::{CapacityExhausted, CapacityResource, TokenId};

use crate::TokenizationError;

/// Statically dispatched destination for encoded token identifiers.
pub trait TokenSink {
    /// Returns the number of additional tokens the sink can accept.
    fn remaining_capacity(&self) -> usize;

    /// Appends one token without allocating.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink cannot accept the token. Fixed-capacity sinks
    /// return [`TokenizationError::CapacityExhausted`] when they are full.
    fn push_token(&mut self, token: TokenId) -> Result<(), TokenizationError>;

    /// Appends a token slice after validating the complete operation.
    ///
    /// # Errors
    ///
    /// Returns [`TokenizationError::CapacityExhausted`] without writing any tokens
    /// when the reported remaining capacity is too small. An implementation may
    /// also propagate an error from [`TokenSink::push_token`].
    fn push_tokens(&mut self, tokens: &[TokenId]) -> Result<(), TokenizationError> {
        if tokens.len() > self.remaining_capacity() {
            return Err(CapacityExhausted::new(
                CapacityResource::Tokens,
                tokens.len() as u64,
                self.remaining_capacity() as u64,
            )
            .into());
        }

        for &token in tokens {
            self.push_token(token)?;
        }
        Ok(())
    }
}

/// Fixed-capacity token output over a mutable slice.
pub struct TokenBuffer<'a> {
    storage: &'a mut [TokenId],
    length: usize,
}

impl<'a> TokenBuffer<'a> {
    /// Creates an empty token buffer over caller-owned storage.
    #[must_use]
    pub const fn new(storage: &'a mut [TokenId]) -> Self {
        Self { storage, length: 0 }
    }

    /// Returns the committed token prefix.
    #[must_use]
    pub fn as_slice(&self) -> &[TokenId] {
        self.storage.get(..self.length).unwrap_or(&[])
    }

    /// Returns the number of committed tokens.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.length
    }

    /// Returns whether no tokens are committed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Clears the logical contents without modifying or reallocating storage.
    pub const fn clear(&mut self) {
        self.length = 0;
    }
}

impl TokenSink for TokenBuffer<'_> {
    fn remaining_capacity(&self) -> usize {
        self.storage.len().saturating_sub(self.length)
    }

    fn push_token(&mut self, token: TokenId) -> Result<(), TokenizationError> {
        let available = self.remaining_capacity();
        let Some(slot) = self.storage.get_mut(self.length) else {
            return Err(
                CapacityExhausted::new(CapacityResource::Tokens, 1, available as u64).into(),
            );
        };
        *slot = token;
        self.length += 1;
        Ok(())
    }
}

/// Statically dispatched destination for tokenizer-produced bytes.
pub trait ByteSink {
    /// Returns the number of additional bytes the sink can accept.
    fn remaining_capacity(&self) -> usize;

    /// Appends one byte without allocating.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink cannot accept the byte. Fixed-capacity sinks
    /// return [`TokenizationError::CapacityExhausted`] when they are full.
    fn push_byte(&mut self, byte: u8) -> Result<(), TokenizationError>;

    /// Appends a byte slice after validating the complete operation.
    ///
    /// # Errors
    ///
    /// Returns [`TokenizationError::CapacityExhausted`] without writing any bytes
    /// when the reported remaining capacity is too small. An implementation may
    /// also propagate an error from [`ByteSink::push_byte`].
    fn push_bytes(&mut self, bytes: &[u8]) -> Result<(), TokenizationError> {
        if bytes.len() > self.remaining_capacity() {
            return Err(CapacityExhausted::new(
                CapacityResource::DecodeBytes,
                bytes.len() as u64,
                self.remaining_capacity() as u64,
            )
            .into());
        }

        for &byte in bytes {
            self.push_byte(byte)?;
        }
        Ok(())
    }
}

/// Fixed-capacity byte output over a mutable slice.
pub struct ByteBuffer<'a> {
    storage: &'a mut [u8],
    length: usize,
}

impl<'a> ByteBuffer<'a> {
    /// Creates an empty byte buffer over caller-owned storage.
    #[must_use]
    pub const fn new(storage: &'a mut [u8]) -> Self {
        Self { storage, length: 0 }
    }

    /// Returns the committed byte prefix.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.storage.get(..self.length).unwrap_or(&[])
    }

    /// Returns the number of committed bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.length
    }

    /// Returns whether no bytes are committed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Clears the logical contents without modifying or reallocating storage.
    pub const fn clear(&mut self) {
        self.length = 0;
    }
}

impl ByteSink for ByteBuffer<'_> {
    fn remaining_capacity(&self) -> usize {
        self.storage.len().saturating_sub(self.length)
    }

    fn push_byte(&mut self, byte: u8) -> Result<(), TokenizationError> {
        let available = self.remaining_capacity();
        let Some(slot) = self.storage.get_mut(self.length) else {
            return Err(
                CapacityExhausted::new(CapacityResource::DecodeBytes, 1, available as u64).into(),
            );
        };
        *slot = byte;
        self.length += 1;
        Ok(())
    }
}

/// Statically dispatched destination for validated UTF-8 fragments.
pub trait TextSink {
    /// Returns the number of additional UTF-8 bytes the sink can accept.
    fn remaining_capacity(&self) -> usize;

    /// Appends one valid UTF-8 fragment without allocating.
    ///
    /// # Errors
    ///
    /// Returns an error if the sink cannot accept the complete fragment.
    /// Fixed-capacity sinks return [`TokenizationError::CapacityExhausted`] when
    /// their remaining storage is too small.
    fn push_str(&mut self, text: &str) -> Result<(), TokenizationError>;
}

/// Fixed-capacity UTF-8 output over a mutable byte slice.
pub struct TextBuffer<'a> {
    storage: &'a mut [u8],
    length: usize,
}

impl<'a> TextBuffer<'a> {
    /// Creates an empty UTF-8 buffer over caller-owned storage.
    #[must_use]
    pub const fn new(storage: &'a mut [u8]) -> Self {
        Self { storage, length: 0 }
    }

    /// Returns the committed text.
    ///
    /// # Errors
    ///
    /// Returns [`TokenizationError::InvalidUtf8`] if the committed prefix cannot
    /// be interpreted as UTF-8.
    pub fn as_str(&self) -> Result<&str, TokenizationError> {
        let bytes = self
            .storage
            .get(..self.length)
            .ok_or(TokenizationError::InvalidUtf8 { byte: 0 })?;
        str::from_utf8(bytes).map_err(|_| TokenizationError::InvalidUtf8 { byte: 0 })
    }

    /// Returns the number of committed UTF-8 bytes.
    #[must_use]
    pub const fn len_bytes(&self) -> usize {
        self.length
    }

    /// Returns whether no text is committed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Clears the logical contents without modifying or reallocating storage.
    pub const fn clear(&mut self) {
        self.length = 0;
    }
}

impl TextSink for TextBuffer<'_> {
    fn remaining_capacity(&self) -> usize {
        self.storage.len().saturating_sub(self.length)
    }

    fn push_str(&mut self, text: &str) -> Result<(), TokenizationError> {
        let bytes = text.as_bytes();
        let available = self.remaining_capacity();
        if bytes.len() > available {
            return Err(CapacityExhausted::new(
                CapacityResource::DecodeBytes,
                bytes.len() as u64,
                available as u64,
            )
            .into());
        }

        let end = self.length.saturating_add(bytes.len());
        let Some(target) = self.storage.get_mut(self.length..end) else {
            return Err(CapacityExhausted::new(
                CapacityResource::DecodeBytes,
                bytes.len() as u64,
                available as u64,
            )
            .into());
        };
        target.copy_from_slice(bytes);
        self.length = end;
        Ok(())
    }
}

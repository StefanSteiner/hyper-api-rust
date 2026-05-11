// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! LittleEndian type serialization and deserialization for HyperBinary format.
//!
//! This module provides [`ToHyperBinary`] and [`FromHyperBinary`] implementations
//! for Rust primitive types using LittleEndian byte order.
//!
//! # Binary vs Text Format Detection
//!
//! Some [`FromHyperBinary`] implementations (notably `i32`, `i64`, `f32`, `f64`)
//! use a heuristic to distinguish binary data from text data: if the buffer
//! is exactly the expected size **and** contains at least one non-ASCII-digit
//! byte, it is treated as binary; otherwise it is parsed as text. This handles
//! both Hyper's binary COPY format and the text format used in simple query
//! results.
//!
//! Fixed-size types like `i16` and `u32` skip the heuristic and always treat
//! exact-size buffers as binary, which is more reliable.
//!
//! # Nullable Values
//!
//! `Option<T>` is the nullable wrapper. Serialization writes a 1-byte NULL
//! indicator (0 = not null, 1 = null) before the value. See
//! [`traits`](super::traits) for the encoding layout.

use bytes::{BufMut, BytesMut};
use std::error::Error;

use super::traits::{
    write_not_null_indicator, FromHyperBinary, ToHyperBinary, NULL_INDICATOR_SIZE,
};

// =============================================================================
// Boolean
// =============================================================================

impl ToHyperBinary for bool {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        write_not_null_indicator(buf);
        buf.put_u8(u8::from(*self));
        Ok(())
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        buf.put_u8(u8::from(*self));
        Ok(())
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        NULL_INDICATOR_SIZE + 1
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        1
    }
}

impl FromHyperBinary for bool {
    #[inline]
    fn from_hyper_binary(buf: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        if buf.len() != 1 {
            return Err("invalid buffer size for bool".into());
        }
        // Hyper sends boolean as ASCII 't' (true) or 'f' (false)
        // Also handle standard binary format (1/0)
        match buf[0] {
            b't' | b'T' | 1 => Ok(true),
            b'f' | b'F' | 0 => Ok(false),
            _ => Err(format!("invalid bool value: {}", buf[0]).into()),
        }
    }
}

// =============================================================================
// Integers - LittleEndian
// =============================================================================

impl ToHyperBinary for i8 {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        write_not_null_indicator(buf);
        buf.put_i8(*self);
        Ok(())
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        buf.put_i8(*self);
        Ok(())
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        NULL_INDICATOR_SIZE + 1
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        1
    }
}

impl FromHyperBinary for i8 {
    #[inline]
    fn from_hyper_binary(buf: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        // Handle both binary format (1 byte) and text format
        if buf.len() == 1 && buf[0] <= 127 {
            Ok(i8::try_from(buf[0]).expect("byte checked <= 127 above"))
        } else {
            // Try parsing as text
            let s = std::str::from_utf8(buf)?;
            s.trim()
                .parse()
                .map_err(|e: std::num::ParseIntError| e.into())
        }
    }
}

impl ToHyperBinary for i16 {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        write_not_null_indicator(buf);
        buf.put_i16_le(*self);
        Ok(())
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        buf.put_i16_le(*self);
        Ok(())
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        NULL_INDICATOR_SIZE + 2
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        2
    }
}

impl FromHyperBinary for i16 {
    #[inline]
    fn from_hyper_binary(buf: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        // For exact-size buffers (2 bytes), always treat as binary.
        // This is more reliable than heuristics because:
        // 1. Binary data that happens to be all ASCII digits would be misclassified
        // 2. The protocol format should be known from RowDescription, not guessed
        if buf.len() == 2 {
            Ok(i16::from_le_bytes([buf[0], buf[1]]))
        } else {
            // For other lengths, it must be text format
            let s = std::str::from_utf8(buf)?;
            s.trim()
                .parse()
                .map_err(|e: std::num::ParseIntError| e.into())
        }
    }
}

impl ToHyperBinary for i32 {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        write_not_null_indicator(buf);
        buf.put_i32_le(*self);
        Ok(())
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        buf.put_i32_le(*self);
        Ok(())
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        NULL_INDICATOR_SIZE + 4
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        4
    }
}

impl FromHyperBinary for i32 {
    #[inline]
    fn from_hyper_binary(buf: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        // Check if it looks like ASCII text (digits, optional leading minus)
        let is_text = buf
            .iter()
            .all(|&b| b.is_ascii_digit() || b == b'-' || b == b'+');
        if is_text || buf.len() != 4 {
            // Parse as text
            let s = std::str::from_utf8(buf)?;
            s.trim()
                .parse()
                .map_err(|e: std::num::ParseIntError| e.into())
        } else {
            // Binary format (4 bytes, contains non-ASCII-digit bytes)
            Ok(i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]))
        }
    }
}

impl ToHyperBinary for i64 {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        write_not_null_indicator(buf);
        buf.put_i64_le(*self);
        Ok(())
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        buf.put_i64_le(*self);
        Ok(())
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        NULL_INDICATOR_SIZE + 8
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        8
    }
}

impl FromHyperBinary for i64 {
    #[inline]
    fn from_hyper_binary(buf: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        // Check if it looks like ASCII text (digits, optional leading minus)
        let is_text = buf
            .iter()
            .all(|&b| b.is_ascii_digit() || b == b'-' || b == b'+');
        if is_text || buf.len() != 8 {
            // Parse as text
            let s = std::str::from_utf8(buf)?;
            s.trim()
                .parse()
                .map_err(|e: std::num::ParseIntError| e.into())
        } else {
            // Binary format (8 bytes, contains non-ASCII-digit bytes)
            Ok(i64::from_le_bytes([
                buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
            ]))
        }
    }
}

impl ToHyperBinary for u32 {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        write_not_null_indicator(buf);
        buf.put_u32_le(*self);
        Ok(())
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        buf.put_u32_le(*self);
        Ok(())
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        NULL_INDICATOR_SIZE + 4
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        4
    }
}

impl FromHyperBinary for u32 {
    #[inline]
    fn from_hyper_binary(buf: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        if buf.len() != 4 {
            return Err("invalid buffer size for u32".into());
        }
        Ok(u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]))
    }
}

impl ToHyperBinary for i128 {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        write_not_null_indicator(buf);
        buf.put_i128_le(*self);
        Ok(())
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        buf.put_i128_le(*self);
        Ok(())
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        NULL_INDICATOR_SIZE + 16
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        16
    }
}

impl FromHyperBinary for i128 {
    #[inline]
    fn from_hyper_binary(buf: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        if buf.len() != 16 {
            return Err("invalid buffer size for i128".into());
        }
        Ok(i128::from_le_bytes([
            buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8], buf[9],
            buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
        ]))
    }
}

impl ToHyperBinary for u128 {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        write_not_null_indicator(buf);
        buf.put_u128_le(*self);
        Ok(())
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        buf.put_u128_le(*self);
        Ok(())
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        NULL_INDICATOR_SIZE + 16
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        16
    }
}

impl FromHyperBinary for u128 {
    #[inline]
    fn from_hyper_binary(buf: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        if buf.len() != 16 {
            return Err("invalid buffer size for u128".into());
        }
        Ok(u128::from_le_bytes([
            buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8], buf[9],
            buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
        ]))
    }
}

// =============================================================================
// Floating Point - LittleEndian
// =============================================================================

impl ToHyperBinary for f32 {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        write_not_null_indicator(buf);
        buf.put_f32_le(*self);
        Ok(())
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        buf.put_f32_le(*self);
        Ok(())
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        NULL_INDICATOR_SIZE + 4
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        4
    }
}

impl FromHyperBinary for f32 {
    #[inline]
    fn from_hyper_binary(buf: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        // Check if it looks like ASCII text (digits, decimal point, sign, scientific notation)
        let is_text = buf.iter().all(|&b| {
            b.is_ascii_digit() || b == b'-' || b == b'+' || b == b'.' || b == b'e' || b == b'E'
        });
        if is_text || buf.len() != 4 {
            // Parse as text
            let s = std::str::from_utf8(buf)?;
            s.trim()
                .parse()
                .map_err(|e: std::num::ParseFloatError| e.into())
        } else {
            // Binary format (4 bytes, contains non-numeric-ASCII bytes)
            Ok(f32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]))
        }
    }
}

impl ToHyperBinary for f64 {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        write_not_null_indicator(buf);
        buf.put_f64_le(*self);
        Ok(())
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        buf.put_f64_le(*self);
        Ok(())
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        NULL_INDICATOR_SIZE + 8
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        8
    }
}

impl FromHyperBinary for f64 {
    #[inline]
    fn from_hyper_binary(buf: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        // Check if it looks like ASCII text (digits, decimal point, sign, scientific notation)
        let is_text = buf.iter().all(|&b| {
            b.is_ascii_digit() || b == b'-' || b == b'+' || b == b'.' || b == b'e' || b == b'E'
        });
        if is_text || buf.len() != 8 {
            // Parse as text
            let s = std::str::from_utf8(buf)?;
            s.trim()
                .parse()
                .map_err(|e: std::num::ParseFloatError| e.into())
        } else {
            // Binary format (8 bytes, contains non-numeric-ASCII bytes)
            Ok(f64::from_le_bytes([
                buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
            ]))
        }
    }
}

// =============================================================================
// Variable-length types: Text and Binary
// =============================================================================

impl ToHyperBinary for str {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        write_not_null_indicator(buf);
        let len = u32::try_from(self.len())
            .map_err(|_| "string length exceeds HyperBinary 4-byte length prefix (u32::MAX)")?;
        buf.put_u32_le(len);
        buf.put_slice(self.as_bytes());
        Ok(())
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let len = u32::try_from(self.len())
            .map_err(|_| "string length exceeds HyperBinary 4-byte length prefix (u32::MAX)")?;
        buf.put_u32_le(len);
        buf.put_slice(self.as_bytes());
        Ok(())
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        NULL_INDICATOR_SIZE + 4 + self.len()
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        4 + self.len()
    }
}

impl ToHyperBinary for String {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.as_str().to_hyper_binary(buf)
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.as_str().to_hyper_binary_not_null(buf)
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        self.as_str().hyper_binary_size()
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        self.as_str().hyper_binary_size_not_null()
    }
}

impl FromHyperBinary for String {
    #[inline]
    fn from_hyper_binary(buf: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        Ok(std::str::from_utf8(buf)?.to_string())
    }
}

impl ToHyperBinary for [u8] {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        write_not_null_indicator(buf);
        let len = u32::try_from(self.len())
            .map_err(|_| "byte slice length exceeds HyperBinary 4-byte length prefix (u32::MAX)")?;
        buf.put_u32_le(len);
        buf.put_slice(self);
        Ok(())
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let len = u32::try_from(self.len())
            .map_err(|_| "byte slice length exceeds HyperBinary 4-byte length prefix (u32::MAX)")?;
        buf.put_u32_le(len);
        buf.put_slice(self);
        Ok(())
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        NULL_INDICATOR_SIZE + 4 + self.len()
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        4 + self.len()
    }
}

impl ToHyperBinary for Vec<u8> {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.as_slice().to_hyper_binary(buf)
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.as_slice().to_hyper_binary_not_null(buf)
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        self.as_slice().hyper_binary_size()
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        self.as_slice().hyper_binary_size_not_null()
    }
}

impl FromHyperBinary for Vec<u8> {
    #[inline]
    fn from_hyper_binary(buf: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>> {
        Ok(buf.to_vec())
    }
}

// =============================================================================
// Option<T> for nullable values
// =============================================================================

impl<T: ToHyperBinary> ToHyperBinary for Option<T> {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        if let Some(value) = self {
            value.to_hyper_binary(buf)
        } else {
            super::traits::write_null(buf);
            Ok(())
        }
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        match self {
            Some(value) => value.to_hyper_binary_not_null(buf),
            None => Err("Cannot write None to a NOT NULL column".into()),
        }
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        match self {
            Some(value) => value.hyper_binary_size(),
            None => NULL_INDICATOR_SIZE,
        }
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        match self {
            Some(value) => value.hyper_binary_size_not_null(),
            None => panic!("Cannot get size of None for NOT NULL column"),
        }
    }
}

// =============================================================================
// Reference implementations
// =============================================================================

impl<T: ToHyperBinary + ?Sized> ToHyperBinary for &T {
    #[inline]
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>> {
        (*self).to_hyper_binary(buf)
    }

    #[inline]
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        (*self).to_hyper_binary_not_null(buf)
    }

    #[inline]
    fn hyper_binary_size(&self) -> usize {
        (*self).hyper_binary_size()
    }

    #[inline]
    fn hyper_binary_size_not_null(&self) -> usize {
        (*self).hyper_binary_size_not_null()
    }
}

// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `HyperBinary` COPY format support.
//!
//! This module provides support for Hyper's binary COPY format (`HyperBinary`),
//! which differs from standard `PostgreSQL` binary COPY in several ways
//! optimized for throughput.
//!
//! # Format Overview
//!
//! The `HyperBinary` format consists of:
//! - **Header**: `"HPRCPY"` + 13 null bytes (19 bytes total)
//! - **Data**: Values with null indicators (1 byte each for nullable columns)
//! - All values are **little-endian** encoded
//! - No tuple start markers (unlike `PostgreSQL`)
//!
//! For nullable columns: `[1 byte null indicator (0=not null, 1=null)] + [value]`
//! For non-nullable columns: just `[value]` (no indicator)
//!
//! # Differences from `PostgreSQL` Binary COPY
//!
//! | Aspect | `PostgreSQL` | `HyperBinary` |
//! |---|---|---|
//! | Header signature | `"PGCOPY\n\xff\r\n\0"` (11 bytes) + flags + ext | `"HPRCPY"` + 13 null bytes (19 bytes) |
//! | Byte order | **Big-endian** (network order) | **Little-endian** (x86 native) |
//! | Row framing | 2-byte field count per row | No per-row framing |
//! | Null encoding | `-1` length prefix (4 bytes) | 1-byte indicator on nullable columns only |
//! | Non-null values | 4-byte length prefix + data | Raw data (fixed-size) or length-prefixed (variable) |
//! | Trailer | 2-byte `-1` sentinel | None |
//!
//! The little-endian encoding and absence of per-row framing reduce both
//! encoding overhead and byte-swapping on x86/ARM-LE architectures, which
//! is where Hyper is typically deployed.

use bytes::{BufMut, BytesMut};
use std::fmt;

/// Narrows a `usize` length to the `HyperBinary` 4-byte little-endian length
/// prefix. Panics on overflow; individual COPY values >`u32::MAX` bytes are a
/// programming error per the `HyperBinary` format contract.
#[inline]
fn copy_len(n: usize) -> u32 {
    u32::try_from(n).expect("HyperBinary COPY value length exceeds u32::MAX")
}

// =============================================================================
// Error Types
// =============================================================================

/// Errors that can occur when reading `HyperBinary` COPY data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyReadError {
    /// Buffer is too short to read the expected type.
    BufferTooShort {
        /// The type being read (e.g., "i16", "i32", "varbinary").
        type_name: &'static str,
        /// The expected minimum buffer size.
        expected: usize,
        /// The actual buffer size.
        actual: usize,
    },
    /// The declared length in a variable-length field exceeds available data.
    LengthExceedsBuffer {
        /// The declared length from the length prefix.
        declared: usize,
        /// The available buffer space after the length prefix.
        available: usize,
    },
}

impl fmt::Display for CopyReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CopyReadError::BufferTooShort {
                type_name,
                expected,
                actual,
            } => write!(
                f,
                "Buffer too short for {type_name}: expected {expected} bytes, got {actual}"
            ),
            CopyReadError::LengthExceedsBuffer {
                declared,
                available,
            } => write!(
                f,
                "Declared length {declared} exceeds available buffer space {available}"
            ),
        }
    }
}

impl std::error::Error for CopyReadError {}

/// The `HyperBinary` COPY signature ("HPRCPY" + 13 null bytes).
pub const HYPER_BINARY_SIGNATURE: &[u8] = b"HPRCPY";

/// The full `HyperBinary` COPY header (19 bytes).
pub const HYPER_BINARY_HEADER: &[u8] = &[
    // Signature: "HPRCPY" (6 bytes)
    b'H', b'P', b'R', b'C', b'P', b'Y', // Padding: 13 null bytes to make total 19 bytes
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Size of the `HyperBinary` header (19 bytes).
pub const HYPER_BINARY_HEADER_SIZE: usize = 19;

/// The `HyperBinary` data format identifier for Hyper's protocol.
pub const HYPER_BINARY_FORMAT: u8 = 2;

/// Writes the `HyperBinary` COPY header to a buffer.
///
/// This should be written at the start of a COPY data stream.
#[inline]
pub fn write_header(buf: &mut BytesMut) {
    buf.put_slice(HYPER_BINARY_HEADER);
}

/// Writes a tuple start marker (no-op for `HyperBinary`).
///
/// `HyperBinary` format does not use tuple start markers.
/// This is kept for API compatibility but does nothing.
#[inline]
pub fn write_tuple_start(_buf: &mut BytesMut, _field_count: i16) {
    // HyperBinary does not use tuple start markers
}

/// Writes the COPY trailer (no-op for `HyperBinary`).
///
/// `HyperBinary` format does not require a trailer.
/// This is kept for API compatibility but does nothing.
#[inline]
pub fn write_trailer(_buf: &mut BytesMut) {
    // HyperBinary does not use a trailer
}

/// Writes a NULL value (1 byte with value 1).
#[inline]
pub fn write_null(buf: &mut BytesMut) {
    buf.put_u8(1); // 1 indicates NULL
}

/// Writes an i8 value (bool) for a nullable column.
/// Format: [not-null indicator: 1 byte (0)][value: 1 byte]
#[inline]
pub fn write_i8(buf: &mut BytesMut, value: i8) {
    buf.put_u8(0); // not-null indicator
    buf.put_i8(value);
}

/// Writes an i8 value for a non-nullable column.
/// Format: [value: 1 byte] (no null indicator)
#[inline]
pub fn write_i8_not_null(buf: &mut BytesMut, value: i8) {
    buf.put_i8(value);
}

/// Writes an i16 value (SMALLINT) for a nullable column.
/// Format: [not-null indicator: 1 byte (0)][value: 2 bytes LittleEndian]
#[inline]
pub fn write_i16(buf: &mut BytesMut, value: i16) {
    buf.put_u8(0); // not-null indicator
    buf.put_i16_le(value);
}

/// Writes an i16 value for a non-nullable column.
/// Format: [value: 2 bytes `LittleEndian`]
#[inline]
pub fn write_i16_not_null(buf: &mut BytesMut, value: i16) {
    buf.put_i16_le(value);
}

/// Writes an i32 value (INT) for a nullable column.
/// Format: [not-null indicator: 1 byte (0)][value: 4 bytes LittleEndian]
#[inline]
pub fn write_i32(buf: &mut BytesMut, value: i32) {
    buf.put_u8(0); // not-null indicator
    buf.put_i32_le(value);
}

/// Writes an i32 value for a non-nullable column.
/// Format: [value: 4 bytes `LittleEndian`]
#[inline]
pub fn write_i32_not_null(buf: &mut BytesMut, value: i32) {
    buf.put_i32_le(value);
}

/// Writes an i64 value (BIGINT) for a nullable column.
/// Format: [not-null indicator: 1 byte (0)][value: 8 bytes LittleEndian]
#[inline]
pub fn write_i64(buf: &mut BytesMut, value: i64) {
    buf.put_u8(0); // not-null indicator
    buf.put_i64_le(value);
}

/// Writes an i64 value for a non-nullable column.
/// Format: [value: 8 bytes `LittleEndian`]
#[inline]
pub fn write_i64_not_null(buf: &mut BytesMut, value: i64) {
    buf.put_i64_le(value);
}

/// Writes a 128-bit value (NUMERIC, INTERVAL) for a nullable column.
/// Format: [not-null indicator: 1 byte (0)][value: 16 bytes LittleEndian]
#[inline]
pub fn write_data128(buf: &mut BytesMut, value: &[u8; 16]) {
    buf.put_u8(0); // not-null indicator
    buf.put_slice(value);
}

/// Writes a 128-bit value for a non-nullable column.
/// Format: [value: 16 bytes `LittleEndian`]
#[inline]
pub fn write_data128_not_null(buf: &mut BytesMut, value: &[u8; 16]) {
    buf.put_slice(value);
}

/// Writes a variable-length binary value (TEXT, BYTEA) for a nullable column.
/// Format: [not-null indicator: 1 byte (0)][length: 4 bytes LittleEndian][data: N bytes]
#[inline]
pub fn write_varbinary(buf: &mut BytesMut, data: &[u8]) {
    buf.put_u8(0); // not-null indicator
    buf.put_u32_le(copy_len(data.len()));
    buf.put_slice(data);
}

/// Writes a variable-length binary value for a non-nullable column.
/// Format: [length: 4 bytes `LittleEndian`][data: N bytes]
#[inline]
pub fn write_varbinary_not_null(buf: &mut BytesMut, data: &[u8]) {
    buf.put_u32_le(copy_len(data.len()));
    buf.put_slice(data);
}

/// Writes an f32 value (REAL/FLOAT4) for a nullable column.
/// Format: [not-null indicator: 1 byte (0)][value: 4 bytes LittleEndian]
#[inline]
pub fn write_f32(buf: &mut BytesMut, value: f32) {
    buf.put_u8(0); // not-null indicator
    buf.put_f32_le(value);
}

/// Writes an f32 value for a non-nullable column.
/// Format: [value: 4 bytes `LittleEndian`]
#[inline]
pub fn write_f32_not_null(buf: &mut BytesMut, value: f32) {
    buf.put_f32_le(value);
}

/// Writes an f64 value (DOUBLE PRECISION/FLOAT8) for a nullable column.
/// Format: [not-null indicator: 1 byte (0)][value: 8 bytes LittleEndian]
#[inline]
pub fn write_f64(buf: &mut BytesMut, value: f64) {
    buf.put_u8(0); // not-null indicator
    buf.put_f64_le(value);
}

/// Writes an f64 value for a non-nullable column.
/// Format: [value: 8 bytes `LittleEndian`]
#[inline]
pub fn write_f64_not_null(buf: &mut BytesMut, value: f64) {
    buf.put_f64_le(value);
}

/// Reads an i8 value from `HyperBinary` format (`LittleEndian`).
#[inline]
#[must_use]
pub fn read_i8(buf: &[u8]) -> i8 {
    // Bit-pattern reinterpret: inverse of whatever wrote the byte.
    #[expect(
        clippy::cast_possible_wrap,
        reason = "intentional i8 bit-pattern reinterpret; inverse of i8 write"
    )]
    let value = buf[0] as i8;
    value
}

/// Reads an i16 value from `HyperBinary` format (`LittleEndian`).
///
/// # Errors
///
/// Returns [`CopyReadError::BufferTooShort`] if the buffer is too short (< 2 bytes).
///
/// # Example
///
/// ```
/// use hyperdb_api_core::protocol::copy::{read_i16, CopyReadError};
///
/// let buf = [0x34, 0x12];
/// assert_eq!(read_i16(&buf).unwrap(), 0x1234);
///
/// let short_buf = [0x34];
/// assert!(matches!(read_i16(&short_buf), Err(CopyReadError::BufferTooShort { .. })));
/// ```
#[inline]
pub fn read_i16(buf: &[u8]) -> Result<i16, CopyReadError> {
    if buf.len() < 2 {
        return Err(CopyReadError::BufferTooShort {
            type_name: "i16",
            expected: 2,
            actual: buf.len(),
        });
    }
    Ok(i16::from_le_bytes([buf[0], buf[1]]))
}

/// Reads an i32 value from `HyperBinary` format (`LittleEndian`).
///
/// # Errors
///
/// Returns [`CopyReadError::BufferTooShort`] if the buffer is too short (< 4 bytes).
///
/// # Example
///
/// ```
/// use hyperdb_api_core::protocol::copy::{read_i32, CopyReadError};
///
/// let buf = [0x04, 0x03, 0x02, 0x01];
/// assert_eq!(read_i32(&buf).unwrap(), 0x01020304);
///
/// let short_buf = [0x04, 0x03, 0x02];
/// assert!(matches!(read_i32(&short_buf), Err(CopyReadError::BufferTooShort { .. })));
/// ```
#[inline]
pub fn read_i32(buf: &[u8]) -> Result<i32, CopyReadError> {
    if buf.len() < 4 {
        return Err(CopyReadError::BufferTooShort {
            type_name: "i32",
            expected: 4,
            actual: buf.len(),
        });
    }
    Ok(i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]))
}

/// Reads an i64 value from `HyperBinary` format (`LittleEndian`).
///
/// # Errors
///
/// Returns [`CopyReadError::BufferTooShort`] if the buffer is too short (< 8 bytes).
///
/// # Example
///
/// ```
/// use hyperdb_api_core::protocol::copy::{read_i64, CopyReadError};
///
/// let buf = [0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01];
/// assert_eq!(read_i64(&buf).unwrap(), 0x0102030405060708);
///
/// let short_buf = [0x08, 0x07, 0x06, 0x05];
/// assert!(matches!(read_i64(&short_buf), Err(CopyReadError::BufferTooShort { .. })));
/// ```
#[inline]
pub fn read_i64(buf: &[u8]) -> Result<i64, CopyReadError> {
    if buf.len() < 8 {
        return Err(CopyReadError::BufferTooShort {
            type_name: "i64",
            expected: 8,
            actual: buf.len(),
        });
    }
    Ok(i64::from_le_bytes([
        buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
    ]))
}

/// Reads a 128-bit value from `HyperBinary` format.
///
/// # Errors
///
/// Returns [`CopyReadError::BufferTooShort`] if the buffer is too short (< 16 bytes).
#[inline]
pub fn read_data128(buf: &[u8]) -> Result<[u8; 16], CopyReadError> {
    if buf.len() < 16 {
        return Err(CopyReadError::BufferTooShort {
            type_name: "data128",
            expected: 16,
            actual: buf.len(),
        });
    }
    let mut result = [0u8; 16];
    result.copy_from_slice(&buf[0..16]);
    Ok(result)
}

/// Reads a variable-length binary value from `HyperBinary` format.
///
/// Returns the data slice (without the length prefix).
///
/// # Errors
///
/// - [`CopyReadError::BufferTooShort`] if the buffer is too short to read the length field
/// - [`CopyReadError::LengthExceedsBuffer`] if the declared length exceeds available data
///
/// # Example
///
/// ```
/// use hyperdb_api_core::protocol::copy::{read_varbinary, CopyReadError};
///
/// // "hello" with 4-byte length prefix
/// let buf = [0x05, 0x00, 0x00, 0x00, b'h', b'e', b'l', b'l', b'o'];
/// assert_eq!(read_varbinary(&buf).unwrap(), b"hello");
///
/// // Too short for length field
/// let short_buf = [0x05, 0x00, 0x00];
/// assert!(matches!(read_varbinary(&short_buf), Err(CopyReadError::BufferTooShort { .. })));
///
/// // Declared length exceeds buffer
/// let bad_len = [0x10, 0x00, 0x00, 0x00, b'h', b'i']; // declares 16 bytes, has 2
/// assert!(matches!(read_varbinary(&bad_len), Err(CopyReadError::LengthExceedsBuffer { .. })));
/// ```
#[inline]
pub fn read_varbinary(buf: &[u8]) -> Result<&[u8], CopyReadError> {
    if buf.len() < 4 {
        return Err(CopyReadError::BufferTooShort {
            type_name: "varbinary length field",
            expected: 4,
            actual: buf.len(),
        });
    }
    let len_u32 = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    // On 32-bit platforms, u32 may exceed usize::MAX. Reject explicitly rather
    // than silently truncating.
    let Ok(len) = usize::try_from(len_u32) else {
        return Err(CopyReadError::LengthExceedsBuffer {
            declared: len_u32 as usize,
            available: buf.len().saturating_sub(4),
        });
    };
    let available = buf.len() - 4;
    if available < len {
        return Err(CopyReadError::LengthExceedsBuffer {
            declared: len,
            available,
        });
    }
    let Some(end) = 4_usize.checked_add(len) else {
        return Err(CopyReadError::LengthExceedsBuffer {
            declared: len,
            available,
        });
    };
    Ok(&buf[4..end])
}

/// A builder for `HyperBinary` COPY data.
///
/// Provides a convenient interface for constructing `HyperBinary` COPY format data.
/// Automatically writes the header before the first data value.
///
/// # Example
///
/// ```
/// use hyperdb_api_core::protocol::copy::CopyDataBuilder;
///
/// let mut builder = CopyDataBuilder::new(1024);
/// builder.write_i32(42, false);
/// builder.write_str("hello", false);
/// let data = builder.take();
/// ```
#[derive(Debug)]
pub struct CopyDataBuilder {
    /// Buffer containing the COPY data.
    buffer: BytesMut,
    /// Whether the `HyperBinary` header has been written.
    header_written: bool,
}

impl CopyDataBuilder {
    /// Creates a new builder with the specified initial capacity.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Initial buffer capacity in bytes
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        CopyDataBuilder {
            buffer: BytesMut::with_capacity(capacity),
            header_written: false,
        }
    }

    /// Creates a new builder with default capacity (1 MB).
    #[must_use]
    pub fn with_default_capacity() -> Self {
        Self::new(1024 * 1024)
    }

    /// Ensures the `HyperBinary` COPY header is written.
    ///
    /// Called automatically before writing any data. Writes the header
    /// only once, even if called multiple times.
    pub fn ensure_header(&mut self) {
        if !self.header_written {
            write_header(&mut self.buffer);
            self.header_written = true;
        }
    }

    /// Writes a NULL value.
    pub fn write_null(&mut self) {
        self.ensure_header();
        write_null(&mut self.buffer);
    }

    /// Writes a boolean value.
    pub fn write_bool(&mut self, value: bool, nullable: bool) {
        self.ensure_header();
        let int_value = i8::from(value);
        if nullable {
            write_i8(&mut self.buffer, int_value);
        } else {
            write_i8_not_null(&mut self.buffer, int_value);
        }
    }

    /// Writes an i16 value.
    pub fn write_i16(&mut self, value: i16, nullable: bool) {
        self.ensure_header();
        if nullable {
            write_i16(&mut self.buffer, value);
        } else {
            write_i16_not_null(&mut self.buffer, value);
        }
    }

    /// Writes an i32 value.
    pub fn write_i32(&mut self, value: i32, nullable: bool) {
        self.ensure_header();
        if nullable {
            write_i32(&mut self.buffer, value);
        } else {
            write_i32_not_null(&mut self.buffer, value);
        }
    }

    /// Writes an i64 value.
    pub fn write_i64(&mut self, value: i64, nullable: bool) {
        self.ensure_header();
        if nullable {
            write_i64(&mut self.buffer, value);
        } else {
            write_i64_not_null(&mut self.buffer, value);
        }
    }

    /// Writes a 128-bit value.
    pub fn write_data128(&mut self, value: &[u8; 16], nullable: bool) {
        self.ensure_header();
        if nullable {
            write_data128(&mut self.buffer, value);
        } else {
            write_data128_not_null(&mut self.buffer, value);
        }
    }

    /// Writes a variable-length binary value (text, bytea, etc.).
    pub fn write_varbinary(&mut self, value: &[u8], nullable: bool) {
        self.ensure_header();
        if nullable {
            write_varbinary(&mut self.buffer, value);
        } else {
            write_varbinary_not_null(&mut self.buffer, value);
        }
    }

    /// Writes a string value.
    pub fn write_str(&mut self, value: &str, nullable: bool) {
        self.write_varbinary(value.as_bytes(), nullable);
    }

    /// Returns the current buffer size in bytes.
    ///
    /// Includes the header if it has been written.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Returns true if the buffer is empty (no data written yet).
    ///
    /// Note: This returns true even if the header has been written,
    /// since the header is written automatically before data.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Returns the buffer contents and resets the builder.
    ///
    /// The builder can be reused after calling this method.
    /// The header flag is reset, so the header will be written again
    /// on the next write operation.
    pub fn take(&mut self) -> BytesMut {
        self.header_written = false;
        std::mem::take(&mut self.buffer)
    }

    /// Returns a reference to the buffer contents.
    ///
    /// Useful for inspecting the data without consuming it.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buffer
    }

    /// Clears the buffer and resets the header flag.
    ///
    /// The builder can be reused after clearing. The capacity is preserved.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.header_written = false;
    }
}

impl Default for CopyDataBuilder {
    fn default() -> Self {
        Self::with_default_capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header() {
        let mut buf = BytesMut::new();
        write_header(&mut buf);
        assert_eq!(buf.as_ref(), HYPER_BINARY_HEADER);
        assert_eq!(buf.len(), HYPER_BINARY_HEADER_SIZE);
        // Verify header starts with "HPRCPY"
        assert_eq!(&buf[..6], b"HPRCPY");
    }

    #[test]
    fn test_i32_little_endian() {
        let mut buf = BytesMut::new();
        write_i32_not_null(&mut buf, 0x01020304);
        // LittleEndian: least significant byte first
        assert_eq!(buf.as_ref(), &[0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn test_nullable_value() {
        let mut buf = BytesMut::new();
        write_i32(&mut buf, 42);
        // [not-null indicator (0)][value LE: 42, 0, 0, 0]
        assert_eq!(buf.as_ref(), &[0, 42, 0, 0, 0]);
    }

    #[test]
    fn test_null() {
        let mut buf = BytesMut::new();
        write_null(&mut buf);
        // NULL indicator is 1
        assert_eq!(buf.as_ref(), &[1]);
    }

    #[test]
    fn test_varbinary() {
        let mut buf = BytesMut::new();
        write_varbinary_not_null(&mut buf, b"hello");
        // [length LE: 5, 0, 0, 0][data: "hello"]
        assert_eq!(buf.as_ref(), &[5, 0, 0, 0, b'h', b'e', b'l', b'l', b'o']);
    }

    #[test]
    fn test_varbinary_nullable() {
        let mut buf = BytesMut::new();
        write_varbinary(&mut buf, b"hi");
        // [not-null (0)][length LE: 2, 0, 0, 0][data: "hi"]
        assert_eq!(buf.as_ref(), &[0, 2, 0, 0, 0, b'h', b'i']);
    }

    #[test]
    fn test_read_i32() {
        let buf = [0x04, 0x03, 0x02, 0x01];
        assert_eq!(read_i32(&buf).unwrap(), 0x01020304);
    }

    #[test]
    fn test_read_i32_too_short() {
        let buf = [0x04, 0x03, 0x02]; // Only 3 bytes
        let err = read_i32(&buf).unwrap_err();
        assert!(matches!(
            err,
            CopyReadError::BufferTooShort {
                type_name: "i32",
                expected: 4,
                actual: 3
            }
        ));
    }

    #[test]
    fn test_read_i16() {
        let buf = [0x34, 0x12];
        assert_eq!(read_i16(&buf).unwrap(), 0x1234);
    }

    #[test]
    fn test_read_i16_too_short() {
        let buf = [0x34]; // Only 1 byte
        let err = read_i16(&buf).unwrap_err();
        assert!(matches!(
            err,
            CopyReadError::BufferTooShort {
                type_name: "i16",
                expected: 2,
                actual: 1
            }
        ));
    }

    #[test]
    fn test_read_i64() {
        let buf = [0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01];
        assert_eq!(read_i64(&buf).unwrap(), 0x0102030405060708);
    }

    #[test]
    fn test_read_i64_too_short() {
        let buf = [0x08, 0x07, 0x06, 0x05]; // Only 4 bytes
        let err = read_i64(&buf).unwrap_err();
        assert!(matches!(
            err,
            CopyReadError::BufferTooShort {
                type_name: "i64",
                expected: 8,
                actual: 4
            }
        ));
    }

    #[test]
    fn test_read_varbinary_valid() {
        let buf = [0x05, 0x00, 0x00, 0x00, b'h', b'e', b'l', b'l', b'o'];
        let result = read_varbinary(&buf).unwrap();
        assert_eq!(result, b"hello");
    }

    #[test]
    fn test_read_varbinary_length_field_too_short() {
        let buf = [0x05, 0x00, 0x00]; // Only 3 bytes for length
        let err = read_varbinary(&buf).unwrap_err();
        assert!(matches!(
            err,
            CopyReadError::BufferTooShort {
                type_name: "varbinary length field",
                expected: 4,
                actual: 3
            }
        ));
    }

    #[test]
    fn test_read_varbinary_data_too_short() {
        let buf = [0x05, 0x00, 0x00, 0x00, b'h', b'e']; // Declares 5 bytes but only 2 present
        let err = read_varbinary(&buf).unwrap_err();
        assert!(matches!(
            err,
            CopyReadError::LengthExceedsBuffer {
                declared: 5,
                available: 2
            }
        ));
    }

    #[test]
    fn read_varbinary_zero_length() {
        // A zero-length value is valid: 4 bytes of length prefix, no data.
        let buf = [0x00, 0x00, 0x00, 0x00];
        let bytes = read_varbinary(&buf).unwrap();
        assert!(bytes.is_empty());
    }

    #[test]
    fn read_varbinary_exact_fit() {
        // Length declares exactly the bytes available.
        let buf = [0x03, 0x00, 0x00, 0x00, b'a', b'b', b'c'];
        let bytes = read_varbinary(&buf).unwrap();
        assert_eq!(bytes, b"abc");
    }

    #[test]
    fn read_varbinary_rejects_huge_declared_length_on_short_buf() {
        // A hostile server could declare u32::MAX but only send a few bytes.
        // The check must reject this rather than slicing past buffer end.
        let buf = [0xFF, 0xFF, 0xFF, 0xFF, b'h', b'i'];
        let err = read_varbinary(&buf).unwrap_err();
        assert!(matches!(err, CopyReadError::LengthExceedsBuffer { .. }));
    }

    #[test]
    fn test_copy_read_error_display() {
        let err = CopyReadError::BufferTooShort {
            type_name: "i32",
            expected: 4,
            actual: 2,
        };
        assert_eq!(
            err.to_string(),
            "Buffer too short for i32: expected 4 bytes, got 2"
        );

        let err = CopyReadError::LengthExceedsBuffer {
            declared: 100,
            available: 10,
        };
        assert_eq!(
            err.to_string(),
            "Declared length 100 exceeds available buffer space 10"
        );
    }

    #[test]
    fn test_copy_data_builder() {
        let mut builder = CopyDataBuilder::new(1024);
        builder.write_i32(42, false);
        builder.write_str("hello", false);

        let data = builder.as_bytes();
        // Header + i32(42) + varbinary("hello")
        assert!(data.starts_with(HYPER_BINARY_HEADER));
    }

    #[test]
    fn test_f64_not_null() {
        let mut buf = BytesMut::new();
        write_f64_not_null(&mut buf, std::f64::consts::PI);
        assert_eq!(buf.len(), 8);
        // Verify it's little-endian
        let read_value = f64::from_le_bytes([
            buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
        ]);
        assert!((read_value - std::f64::consts::PI).abs() < 1e-10);
    }
}

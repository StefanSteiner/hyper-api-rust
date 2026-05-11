// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Traits for HyperBinary serialization and deserialization.
//!
//! HyperBinary is Hyper's optimized binary wire format. All multi-byte
//! values are **little-endian** (native on x86/ARM-LE). The two core
//! traits are:
//!
//! - [`ToHyperBinary`] — Serialize a Rust value into a buffer
//! - [`FromHyperBinary`] — Deserialize a Rust value from a byte slice
//!
//! # Encoding Layout
//!
//! **Fixed-size types** (e.g., `i32`, `f64`, `Date`):
//! ```text
//! Nullable:     [0x00 (not null)] [value LE bytes]
//! Nullable NULL: [0x01 (null)]
//! NOT NULL:     [value LE bytes]  (no indicator)
//! ```
//!
//! **Variable-length types** (e.g., `String`, `Vec<u8>`):
//! ```text
//! Nullable:     [0x00] [4-byte LE length] [data bytes]
//! Nullable NULL: [0x01]
//! NOT NULL:     [4-byte LE length] [data bytes]
//! ```
//!
//! Example: `"hello"` in a nullable column →
//! `[0x00, 0x05, 0x00, 0x00, 0x00, 0x68, 0x65, 0x6C, 0x6C, 0x6F]`

use bytes::BytesMut;
use std::error::Error;

/// Indicates whether a value is NULL or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsNull {
    /// The value is NULL.
    Yes,
    /// The value is not NULL.
    No,
}

/// A trait for types that can be serialized to HyperBinary format.
///
/// # Nullable vs Non-Nullable
///
/// HyperBinary has two encoding modes:
/// - Nullable: Prefixed with a 1-byte NULL indicator (0 = value follows, 1 = NULL)
/// - Non-nullable: No prefix, just the raw value
///
/// Use `to_hyper_binary` for nullable columns and `to_hyper_binary_not_null` for
/// NOT NULL columns.
pub trait ToHyperBinary {
    /// Serializes the value to HyperBinary format for a nullable column.
    ///
    /// This writes a 0 byte (not null indicator) followed by the value.
    ///
    /// # Errors
    ///
    /// Returns an error if the value cannot be encoded — for example,
    /// variable-length values whose size exceeds the 4-byte length field
    /// (`u32::MAX`), or type-specific validation failures (numeric range,
    /// geography bounds, etc.). Concrete implementations surface their own
    /// error type boxed behind [`Error`].
    fn to_hyper_binary(&self, buf: &mut BytesMut) -> Result<(), Box<dyn Error + Send + Sync>>;

    /// Serializes the value to HyperBinary format for a NOT NULL column.
    ///
    /// This writes just the value without any NULL indicator.
    ///
    /// # Errors
    ///
    /// Same as [`ToHyperBinary::to_hyper_binary`]: returns an error when
    /// the value cannot be encoded (oversized payload, out-of-range
    /// coordinates, invalid numeric scale, etc.).
    fn to_hyper_binary_not_null(
        &self,
        buf: &mut BytesMut,
    ) -> Result<(), Box<dyn Error + Send + Sync>>;

    /// Returns the size in bytes this value will occupy when serialized (nullable).
    fn hyper_binary_size(&self) -> usize;

    /// Returns the size in bytes this value will occupy when serialized (not nullable).
    fn hyper_binary_size_not_null(&self) -> usize;
}

/// A trait for types that can be deserialized from HyperBinary format.
pub trait FromHyperBinary: Sized {
    /// Deserializes a value from HyperBinary format.
    ///
    /// # Arguments
    ///
    /// * `buf` - The byte slice containing the serialized value (without NULL indicator)
    ///
    /// # Returns
    ///
    /// The deserialized value, or an error if deserialization fails.
    ///
    /// # Errors
    ///
    /// Returns an error if `buf` is shorter than the type's fixed width,
    /// if the declared length field (for variable-length types) exceeds
    /// the remaining buffer, or if the bytes fail type-specific validation
    /// (for example, UTF-8 decoding for `String`, scale/precision checks
    /// for numeric types, or endianness/geometry checks for `Geography`).
    fn from_hyper_binary(buf: &[u8]) -> Result<Self, Box<dyn Error + Send + Sync>>;
}

/// Writes a NULL value to the buffer (just the NULL indicator byte).
#[inline]
pub(crate) fn write_null(buf: &mut BytesMut) {
    buf.extend_from_slice(&[1u8]); // 1 = NULL
}

/// Writes the not-null indicator to the buffer.
#[inline]
pub(crate) fn write_not_null_indicator(buf: &mut BytesMut) {
    buf.extend_from_slice(&[0u8]); // 0 = not NULL
}

/// Size of the NULL indicator byte.
pub(crate) const NULL_INDICATOR_SIZE: usize = 1;

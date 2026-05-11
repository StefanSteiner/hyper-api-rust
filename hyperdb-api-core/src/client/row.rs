// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Row handling for query results.
//!
//! This module provides three row types optimized for different access patterns:
//!
//! - [`Row`] - Standard row with pre-computed offsets for random access
//! - [`StreamRow`] - Zero-allocation row for high-throughput sequential scans
//! - [`BatchRow`] - Pre-computed offsets for multi-column batch access
//!
//! # Example
//!
//! ```no_run
//! # use hyperdb_api_core::client::{Client, Config};
//! # fn example(client: &Client) -> hyperdb_api_core::client::Result<()> {
//! for row in client.query("SELECT id, name FROM users")? {
//!     let id = row.get_i32(0);      // Option<i32>
//!     let name = row.get_string(1); // Option<String>
//!     println!("{:?}: {:?}", id, name);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Attribution
//!
//! The [`Row`] / [`StreamRow`] / [`BatchRow`] type decomposition and the
//! pre-computed-offset random-access pattern were adapted from
//! [`tokio-postgres`](https://github.com/sfackler/rust-postgres) (Copyright
//! (c) 2016 Steven Fackler, MIT or Apache-2.0). Hyper-specific performance
//! changes were added on top. See the `NOTICE` file at the repo root for
//! the full upstream copyright and reproduced license text.

#![allow(
    clippy::cast_precision_loss,
    reason = "column accessor: explicit user-requested widening"
)]

use std::sync::Arc;

use crate::protocol::message::backend::DataRowBody;
use crate::types::FromHyperBinary;

use super::error::{Error, ErrorKind, Result};
use super::statement::Column;

// =============================================================================
// BYTEA Hex Escape Decoding
// =============================================================================

/// Decodes `PostgreSQL` hex-escaped BYTEA format to raw bytes.
fn decode_bytea_hex(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() >= 2 && data[0] == b'\\' && data[1] == b'x' {
        let hex_data = &data[2..];
        if hex_data.len() % 2 != 0 {
            return None;
        }
        let mut result = Vec::with_capacity(hex_data.len() / 2);
        for chunk in hex_data.chunks(2) {
            let high = hex_digit_to_value(chunk[0])?;
            let low = hex_digit_to_value(chunk[1])?;
            result.push((high << 4) | low);
        }
        Some(result)
    } else {
        Some(data.to_vec())
    }
}

#[inline]
fn hex_digit_to_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// =============================================================================
// Row - Standard row with pre-computed offsets
// =============================================================================

/// A row of data from a query result.
///
/// Pre-computes column byte ranges for O(1) random access. Best for accessing
/// multiple columns or accessing columns in random order.
///
/// # Example
///
/// ```no_run
/// # use hyperdb_api_core::client::{Client, Config};
/// # fn example(client: &Client) -> hyperdb_api_core::client::Result<()> {
/// for row in client.query("SELECT id, name, active FROM users")? {
///     let id = row.get_i32(0);
///     let name = row.get_string(1);
///     let active = row.get_bool(2);
///     println!("{:?}: {:?} (active={:?})", id, name, active);
/// }
/// # Ok(())
/// # }
/// ```
pub struct Row {
    columns: Arc<Vec<Column>>,
    data: DataRowBody,
    ranges: Vec<Option<std::ops::Range<usize>>>,
}

impl Row {
    pub(crate) fn new(columns: Arc<Vec<Column>>, data: DataRowBody) -> Result<Self> {
        let ranges: Vec<_> = data
            .ranges()
            .map(|r| r.map_err(|e| Error::protocol(format!("invalid data row: {e}"))))
            .collect::<Result<Vec<_>>>()?;
        Ok(Row {
            columns,
            data,
            ranges,
        })
    }

    /// Returns the number of columns.
    #[inline]
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Returns true if the row has no columns.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// Returns the column metadata.
    #[inline]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Returns raw bytes for a column, or None if NULL.
    #[inline]
    pub fn get_bytes(&self, idx: usize) -> Option<&[u8]> {
        match self.ranges.get(idx)? {
            Some(range) => Some(&self.data.buffer()[range.start..range.end]),
            None => None,
        }
    }

    /// Returns true if the column is NULL.
    #[inline]
    pub fn is_null(&self, idx: usize) -> bool {
        self.ranges
            .get(idx)
            .map_or(true, std::option::Option::is_none)
    }

    #[inline]
    fn column_format(&self, idx: usize) -> super::statement::ColumnFormat {
        self.columns
            .get(idx)
            .map(super::statement::Column::format)
            .unwrap_or_default()
    }

    /// Gets an i16 value. Returns None if NULL or invalid format.
    #[inline]
    pub fn get_i16(&self, idx: usize) -> Option<i16> {
        let bytes = self.get_bytes(idx)?;
        if self.column_format(idx).is_binary() && bytes.len() == 2 {
            Some(i16::from_le_bytes([bytes[0], bytes[1]]))
        } else if !self.column_format(idx).is_binary() {
            std::str::from_utf8(bytes).ok()?.trim().parse().ok()
        } else {
            None
        }
    }

    /// Gets an i32 value. Returns None if NULL or invalid format.
    #[inline]
    pub fn get_i32(&self, idx: usize) -> Option<i32> {
        let bytes = self.get_bytes(idx)?;
        if self.column_format(idx).is_binary() && bytes.len() == 4 {
            Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        } else if !self.column_format(idx).is_binary() {
            std::str::from_utf8(bytes).ok()?.trim().parse().ok()
        } else {
            None
        }
    }

    /// Gets an i64 value. Returns None if NULL or invalid format.
    #[inline]
    pub fn get_i64(&self, idx: usize) -> Option<i64> {
        let bytes = self.get_bytes(idx)?;
        if self.column_format(idx).is_binary() && bytes.len() == 8 {
            Some(i64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]))
        } else if !self.column_format(idx).is_binary() {
            std::str::from_utf8(bytes).ok()?.trim().parse().ok()
        } else {
            None
        }
    }

    /// Gets an f32 value. Returns None if NULL or invalid format.
    #[inline]
    pub fn get_f32(&self, idx: usize) -> Option<f32> {
        let bytes = self.get_bytes(idx)?;
        if self.column_format(idx).is_binary() && bytes.len() == 4 {
            Some(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        } else if !self.column_format(idx).is_binary() {
            std::str::from_utf8(bytes).ok()?.trim().parse().ok()
        } else {
            None
        }
    }

    /// Gets an f64 value. Returns None if NULL or invalid format.
    #[inline]
    pub fn get_f64(&self, idx: usize) -> Option<f64> {
        let bytes = self.get_bytes(idx)?;
        if self.column_format(idx).is_binary() && bytes.len() == 8 {
            Some(f64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]))
        } else if !self.column_format(idx).is_binary() {
            std::str::from_utf8(bytes).ok()?.trim().parse().ok()
        } else {
            None
        }
    }

    /// Gets a bool value. Returns None if NULL or invalid format.
    #[inline]
    pub fn get_bool(&self, idx: usize) -> Option<bool> {
        let bytes = self.get_bytes(idx)?;
        if self.column_format(idx).is_binary() && bytes.len() == 1 {
            match bytes[0] {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            }
        } else {
            match bytes {
                [b't' | b'T'] => Some(true),
                [b'f' | b'F'] => Some(false),
                b"true" => Some(true),
                b"false" => Some(false),
                _ => None,
            }
        }
    }

    /// Gets a String value. Returns None if NULL or invalid UTF-8.
    #[inline]
    pub fn get_string(&self, idx: usize) -> Option<String> {
        let bytes = self.get_bytes(idx)?;
        String::from_utf8(bytes.to_vec()).ok()
    }

    /// Gets raw bytes as owned Vec, decoding BYTEA hex format if needed.
    #[inline]
    pub fn get_bytes_owned(&self, idx: usize) -> Option<Vec<u8>> {
        let bytes = self.get_bytes(idx)?;
        if self.column_format(idx).is_binary() {
            Some(bytes.to_vec())
        } else {
            decode_bytea_hex(bytes)
        }
    }

    /// Gets a typed value using the `FromHyperBinary` trait.
    ///
    /// Returns None if the column is NULL or conversion fails.
    #[inline]
    pub fn get<T: FromHyperBinary>(&self, idx: usize) -> Option<T> {
        let bytes = self.get_bytes(idx)?;
        T::from_hyper_binary(bytes).ok()
    }

    /// Gets a typed value by column name.
    ///
    /// Returns None if the column is NULL, not found, or conversion fails.
    pub fn get_by_name<T: FromHyperBinary>(&self, name: &str) -> Option<T> {
        let idx = self.column_index(name).ok()?;
        self.get(idx)
    }

    /// Returns the index of a column by name.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Query`] with the column name in the message
    /// if no column matches.
    pub fn column_index(&self, name: &str) -> Result<usize> {
        self.columns
            .iter()
            .position(|c| c.name() == name)
            .ok_or_else(|| Error::new(ErrorKind::Query, format!("column not found: {name}")))
    }
}

impl std::fmt::Debug for Row {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Row")
            .field("columns", &self.columns)
            .field("column_count", &self.column_count())
            .finish_non_exhaustive()
    }
}

// =============================================================================
// StreamRow - Zero-allocation row for high-throughput streaming
// =============================================================================

/// A streaming row optimized for sequential single-pass access.
///
/// Unlike [`Row`], this computes column offsets on-demand without pre-allocation.
/// Best for high-throughput scans where each row is processed once.
///
/// # Example
///
/// ```no_run
/// # use hyperdb_api_core::client::{Client, Config};
/// # fn example(client: &Client) -> hyperdb_api_core::client::Result<()> {
/// // High-throughput streaming access
/// let mut stream = client.query_streaming("SELECT id, value FROM sensors", 1000)?;
/// while let Some(chunk) = stream.next_chunk()? {
///     for row in chunk {
///         if let Some(id) = row.get_i32(0) {
///             process(id, row.get_f64(1));
///         }
///     }
/// }
/// # fn process(_: i32, _: Option<f64>) {}
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct StreamRow {
    data: DataRowBody,
}

impl StreamRow {
    #[inline]
    pub(crate) fn new(data: DataRowBody) -> Self {
        StreamRow { data }
    }

    /// Returns the number of columns.
    #[inline]
    pub fn column_count(&self) -> usize {
        self.data.column_count() as usize
    }

    /// Gets raw bytes for a column, or None if NULL.
    #[inline]
    pub fn get_bytes(&self, idx: usize) -> Option<&[u8]> {
        self.data.get_column_bytes(idx)
    }

    /// Returns true if the column is NULL.
    #[inline]
    pub fn is_null(&self, idx: usize) -> bool {
        self.data.is_column_null(idx)
    }

    /// Gets an i16 value. Returns None if NULL or wrong size.
    #[inline]
    pub fn get_i16(&self, idx: usize) -> Option<i16> {
        let bytes = self.get_bytes(idx)?;
        (bytes.len() == 2).then(|| i16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// Gets an i32 value. Returns None if NULL or wrong size.
    #[inline]
    pub fn get_i32(&self, idx: usize) -> Option<i32> {
        let bytes = self.get_bytes(idx)?;
        (bytes.len() == 4).then(|| i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Gets an i64 value. Returns None if NULL or wrong size.
    #[inline]
    pub fn get_i64(&self, idx: usize) -> Option<i64> {
        let bytes = self.get_bytes(idx)?;
        (bytes.len() == 8).then(|| {
            i64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ])
        })
    }

    /// Gets an f32 value. Returns None if NULL or wrong size.
    #[inline]
    pub fn get_f32(&self, idx: usize) -> Option<f32> {
        let bytes = self.get_bytes(idx)?;
        (bytes.len() == 4).then(|| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Gets an f64 value. Returns None if NULL or wrong size.
    #[inline]
    pub fn get_f64(&self, idx: usize) -> Option<f64> {
        let bytes = self.get_bytes(idx)?;
        (bytes.len() == 8).then(|| {
            f64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ])
        })
    }

    /// Gets a bool value. Returns None if NULL or invalid.
    #[inline]
    pub fn get_bool(&self, idx: usize) -> Option<bool> {
        let bytes = self.get_bytes(idx)?;
        (bytes.len() == 1).then(|| match bytes[0] {
            b't' | b'T' | 1 => Some(true),
            b'f' | b'F' | 0 => Some(false),
            _ => None,
        })?
    }

    /// Gets a String value. Returns None if NULL or invalid UTF-8.
    #[inline]
    pub fn get_string(&self, idx: usize) -> Option<String> {
        let bytes = self.get_bytes(idx)?;
        String::from_utf8(bytes.to_vec()).ok()
    }

    /// Generic typed getter with automatic type coercion.
    ///
    /// Supports widening conversions (i16 → i32 → i64, f32 → f64).
    #[inline]
    pub fn get<T: FromBinaryValue>(&self, idx: usize) -> Option<T> {
        T::from_stream_row(self, idx)
    }

    /// Converts to a `BatchRow` with pre-computed offsets for multi-column access.
    #[inline]
    pub fn to_batch(self) -> BatchRow {
        BatchRow::from_stream(self)
    }

    /// Returns the underlying `DataRowBody` (for advanced use).
    #[inline]
    pub fn into_data(self) -> DataRowBody {
        self.data
    }
}

// =============================================================================
// BatchRow - Pre-computed offsets for multi-column batch access
// =============================================================================

/// A row with pre-computed offsets for fast multi-column batch access.
///
/// Pre-computes all column offsets once (O(n) total), making subsequent
/// column access O(1). Best for accessing many columns from each row.
///
/// # Example
///
/// ```no_run
/// # use hyperdb_api_core::client::{Client, Config, BatchRow};
/// # fn example(client: &Client) -> hyperdb_api_core::client::Result<()> {
/// let mut stream = client.query_streaming("SELECT * FROM events", 1000)?;
/// while let Some(chunk) = stream.next_chunk()? {
///     for row in chunk {
///         let batch = BatchRow::from_stream(row); // Pre-compute all offsets
///         let id = batch.get_i32(0);
///         let sensor = batch.get_i32(1);
///         let value = batch.get_f64(2);
///         let ts = batch.get_i64(3);
///     }
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct BatchRow {
    data: DataRowBody,
    offsets: Vec<Option<(usize, usize)>>,
}

impl BatchRow {
    /// Creates a `BatchRow` from a `DataRowBody`.
    #[inline]
    pub fn new(data: DataRowBody) -> Self {
        let offsets = data.compute_all_offsets();
        BatchRow { data, offsets }
    }

    /// Creates a `BatchRow` from a `StreamRow`.
    #[inline]
    pub fn from_stream(row: StreamRow) -> Self {
        Self::new(row.data)
    }

    /// Returns the number of columns.
    #[inline]
    pub fn column_count(&self) -> usize {
        self.offsets.len()
    }

    /// Gets raw bytes for a column (O(1) access).
    #[inline]
    pub fn get_bytes(&self, idx: usize) -> Option<&[u8]> {
        let (start, end) = self.offsets.get(idx)?.as_ref().copied()?;
        self.data.buffer().get(start..end)
    }

    /// Returns true if the column is NULL (O(1) access).
    #[inline]
    pub fn is_null(&self, idx: usize) -> bool {
        self.offsets
            .get(idx)
            .map_or(true, std::option::Option::is_none)
    }

    /// Gets an i16 value.
    #[inline]
    pub fn get_i16(&self, idx: usize) -> Option<i16> {
        let bytes = self.get_bytes(idx)?;
        (bytes.len() == 2).then(|| i16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// Gets an i32 value.
    #[inline]
    pub fn get_i32(&self, idx: usize) -> Option<i32> {
        let bytes = self.get_bytes(idx)?;
        (bytes.len() == 4).then(|| i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Gets an i64 value.
    #[inline]
    pub fn get_i64(&self, idx: usize) -> Option<i64> {
        let bytes = self.get_bytes(idx)?;
        (bytes.len() == 8).then(|| {
            i64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ])
        })
    }

    /// Gets an f32 value.
    #[inline]
    pub fn get_f32(&self, idx: usize) -> Option<f32> {
        let bytes = self.get_bytes(idx)?;
        (bytes.len() == 4).then(|| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Gets an f64 value.
    #[inline]
    pub fn get_f64(&self, idx: usize) -> Option<f64> {
        let bytes = self.get_bytes(idx)?;
        (bytes.len() == 8).then(|| {
            f64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ])
        })
    }

    /// Gets a bool value.
    #[inline]
    pub fn get_bool(&self, idx: usize) -> Option<bool> {
        let bytes = self.get_bytes(idx)?;
        (bytes.len() == 1).then(|| match bytes[0] {
            b't' | b'T' | 1 => Some(true),
            b'f' | b'F' | 0 => Some(false),
            _ => None,
        })?
    }

    /// Gets a String value.
    #[inline]
    pub fn get_string(&self, idx: usize) -> Option<String> {
        let bytes = self.get_bytes(idx)?;
        String::from_utf8(bytes.to_vec()).ok()
    }

    /// Generic typed getter with automatic type coercion.
    ///
    /// Supports widening conversions (i16 → i32 → i64, f32 → f64).
    #[inline]
    pub fn get<T: FromBinaryValue>(&self, idx: usize) -> Option<T> {
        T::from_batch_row(self, idx)
    }
}

// =============================================================================
// FromBinaryValue - Type coercion trait for row types
// =============================================================================

/// Trait for extracting typed values from binary row data with automatic coercion.
pub trait FromBinaryValue: Sized {
    /// Extracts a value from a `StreamRow` at the given column index.
    fn from_stream_row(row: &StreamRow, idx: usize) -> Option<Self>;

    /// Extracts a value from a `BatchRow` at the given column index.
    fn from_batch_row(row: &BatchRow, idx: usize) -> Option<Self>;
}

impl FromBinaryValue for bool {
    #[inline]
    fn from_stream_row(row: &StreamRow, idx: usize) -> Option<Self> {
        row.get_bool(idx)
    }
    #[inline]
    fn from_batch_row(row: &BatchRow, idx: usize) -> Option<Self> {
        row.get_bool(idx)
    }
}

impl FromBinaryValue for i16 {
    #[inline]
    fn from_stream_row(row: &StreamRow, idx: usize) -> Option<Self> {
        row.get_i16(idx)
    }
    #[inline]
    fn from_batch_row(row: &BatchRow, idx: usize) -> Option<Self> {
        row.get_i16(idx)
    }
}

impl FromBinaryValue for i32 {
    #[inline]
    fn from_stream_row(row: &StreamRow, idx: usize) -> Option<Self> {
        row.get_i32(idx).or_else(|| row.get_i16(idx).map(i32::from))
    }
    #[inline]
    fn from_batch_row(row: &BatchRow, idx: usize) -> Option<Self> {
        row.get_i32(idx).or_else(|| row.get_i16(idx).map(i32::from))
    }
}

impl FromBinaryValue for i64 {
    #[inline]
    fn from_stream_row(row: &StreamRow, idx: usize) -> Option<Self> {
        row.get_i64(idx)
            .or_else(|| row.get_i32(idx).map(i64::from))
            .or_else(|| row.get_i16(idx).map(i64::from))
    }
    #[inline]
    fn from_batch_row(row: &BatchRow, idx: usize) -> Option<Self> {
        row.get_i64(idx)
            .or_else(|| row.get_i32(idx).map(i64::from))
            .or_else(|| row.get_i16(idx).map(i64::from))
    }
}

impl FromBinaryValue for f32 {
    #[inline]
    fn from_stream_row(row: &StreamRow, idx: usize) -> Option<Self> {
        row.get_f32(idx)
    }
    #[inline]
    fn from_batch_row(row: &BatchRow, idx: usize) -> Option<Self> {
        row.get_f32(idx)
    }
}

impl FromBinaryValue for f64 {
    #[inline]
    fn from_stream_row(row: &StreamRow, idx: usize) -> Option<Self> {
        row.get_f64(idx)
            .or_else(|| row.get_f32(idx).map(f64::from))
            .or_else(|| row.get_i64(idx).map(|v| v as f64))
            .or_else(|| row.get_i32(idx).map(f64::from))
    }
    #[inline]
    fn from_batch_row(row: &BatchRow, idx: usize) -> Option<Self> {
        row.get_f64(idx)
            .or_else(|| row.get_f32(idx).map(f64::from))
            .or_else(|| row.get_i64(idx).map(|v| v as f64))
            .or_else(|| row.get_i32(idx).map(f64::from))
    }
}

impl FromBinaryValue for String {
    #[inline]
    fn from_stream_row(row: &StreamRow, idx: usize) -> Option<Self> {
        row.get_string(idx)
    }
    #[inline]
    fn from_batch_row(row: &BatchRow, idx: usize) -> Option<Self> {
        row.get_string(idx)
    }
}

impl FromBinaryValue for Vec<u8> {
    #[inline]
    fn from_stream_row(row: &StreamRow, idx: usize) -> Option<Self> {
        row.get_bytes(idx).map(<[u8]>::to_vec)
    }
    #[inline]
    fn from_batch_row(row: &BatchRow, idx: usize) -> Option<Self> {
        row.get_bytes(idx).map(<[u8]>::to_vec)
    }
}

impl FromBinaryValue for crate::types::Date {
    #[inline]
    fn from_stream_row(row: &StreamRow, idx: usize) -> Option<Self> {
        let bytes = row.get_bytes(idx)?;
        crate::types::FromHyperBinary::from_hyper_binary(bytes).ok()
    }
    #[inline]
    fn from_batch_row(row: &BatchRow, idx: usize) -> Option<Self> {
        let bytes = row.get_bytes(idx)?;
        crate::types::FromHyperBinary::from_hyper_binary(bytes).ok()
    }
}

impl FromBinaryValue for crate::types::Time {
    #[inline]
    fn from_stream_row(row: &StreamRow, idx: usize) -> Option<Self> {
        let bytes = row.get_bytes(idx)?;
        crate::types::FromHyperBinary::from_hyper_binary(bytes).ok()
    }
    #[inline]
    fn from_batch_row(row: &BatchRow, idx: usize) -> Option<Self> {
        let bytes = row.get_bytes(idx)?;
        crate::types::FromHyperBinary::from_hyper_binary(bytes).ok()
    }
}

impl FromBinaryValue for crate::types::Timestamp {
    #[inline]
    fn from_stream_row(row: &StreamRow, idx: usize) -> Option<Self> {
        let bytes = row.get_bytes(idx)?;
        crate::types::FromHyperBinary::from_hyper_binary(bytes).ok()
    }
    #[inline]
    fn from_batch_row(row: &BatchRow, idx: usize) -> Option<Self> {
        let bytes = row.get_bytes(idx)?;
        crate::types::FromHyperBinary::from_hyper_binary(bytes).ok()
    }
}

impl FromBinaryValue for crate::types::Interval {
    #[inline]
    fn from_stream_row(row: &StreamRow, idx: usize) -> Option<Self> {
        let bytes = row.get_bytes(idx)?;
        crate::types::FromHyperBinary::from_hyper_binary(bytes).ok()
    }
    #[inline]
    fn from_batch_row(row: &BatchRow, idx: usize) -> Option<Self> {
        let bytes = row.get_bytes(idx)?;
        crate::types::FromHyperBinary::from_hyper_binary(bytes).ok()
    }
}

impl FromBinaryValue for crate::types::OffsetTimestamp {
    #[inline]
    fn from_stream_row(row: &StreamRow, idx: usize) -> Option<Self> {
        let bytes = row.get_bytes(idx)?;
        crate::types::FromHyperBinary::from_hyper_binary(bytes).ok()
    }
    #[inline]
    fn from_batch_row(row: &BatchRow, idx: usize) -> Option<Self> {
        let bytes = row.get_bytes(idx)?;
        crate::types::FromHyperBinary::from_hyper_binary(bytes).ok()
    }
}

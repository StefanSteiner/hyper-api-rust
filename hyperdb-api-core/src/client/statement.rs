// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Prepared statement handling.

use crate::types::Oid;

/// Format code for column data.
///
/// This indicates how data values are encoded in the wire protocol.
/// The format affects how values are serialized/deserialized and can
/// significantly impact performance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnFormat {
    /// Text format (human-readable ASCII).
    ///
    /// Values are sent as UTF-8 strings. Slower but human-readable.
    /// Use for debugging or when compatibility with text-based tools is needed.
    #[default]
    Text,
    /// Standard `PostgreSQL` binary format.
    ///
    /// Uses `PostgreSQL`'s standard binary encoding (`BigEndian` for most types).
    /// Compatible with standard `PostgreSQL` clients.
    Binary,
    /// Hyper-specific binary format (little-endian, optimized).
    ///
    /// Uses Hyper's optimized binary format where all multi-byte values are
    /// **little-endian** (x86/ARM-LE native byte order), avoiding byte-swapping
    /// on modern hardware. This contrasts with standard `PostgreSQL` binary format
    /// which uses **big-endian** (network byte order).
    ///
    /// Additional differences from `PostgreSQL` binary:
    /// - No per-row field count prefix (rows are implicitly framed)
    /// - NULL is a 1-byte indicator on nullable columns only (vs 4-byte `-1` length)
    /// - Fixed-size types have no length prefix (vs 4-byte length in PG binary)
    ///
    /// This is the fastest format and is used by default for `query_fast()`,
    /// `query_streaming()`, and the COPY bulk insertion path.
    HyperBinary,
}

impl ColumnFormat {
    /// Creates a `ColumnFormat` from the wire protocol format code.
    ///
    /// Format codes: 0 = Text, 1 = Binary, 2 = `HyperBinary`
    #[must_use]
    pub fn from_code(code: i16) -> Self {
        match code {
            0 => ColumnFormat::Text,
            1 => ColumnFormat::Binary,
            2 => ColumnFormat::HyperBinary,
            _ => ColumnFormat::Text, // Default to text for unknown codes
        }
    }

    /// Returns the wire protocol format code.
    #[must_use]
    pub fn to_code(self) -> i16 {
        match self {
            ColumnFormat::Text => 0,
            ColumnFormat::Binary => 1,
            ColumnFormat::HyperBinary => 2,
        }
    }

    /// Returns true if this is a binary format (Binary or `HyperBinary`).
    #[must_use]
    pub fn is_binary(self) -> bool {
        matches!(self, ColumnFormat::Binary | ColumnFormat::HyperBinary)
    }
}

/// Metadata about a column in a result set.
///
/// `Column` carries the three pieces of information the wire protocol's
/// `RowDescription` message provides for each result field: the column
/// name, its type OID, and the type modifier that encodes width-specific
/// parameters like `NUMERIC(precision, scale)` or `VARCHAR(n)`.
///
/// The type modifier is essential for decoding types whose wire format
/// depends on declared precision/scale (e.g. `NUMERIC`, where precision
/// ≤ 18 uses an 8-byte `i64` wire form and precision > 18 uses a
/// 16-byte `i128` wire form — and in both cases the scale needed to
/// interpret the unscaled integer value lives only in the type
/// modifier). Upper layers construct a `SqlType` from OID + modifier
/// via [`crate::types::SqlType::from_oid_and_modifier`] — using
/// [`crate::types::SqlType::from_oid`] alone silently drops precision
/// and scale, causing decoders to default to scale = 0 and corrupt
/// fractional values.
#[derive(Debug, Clone)]
pub struct Column {
    pub(crate) name: String,
    pub(crate) type_oid: Oid,
    /// PostgreSQL-style type modifier. For NUMERIC columns the encoding
    /// is `((precision << 16) | scale) + 4`; for VARCHAR it's
    /// `length + 4`; for most other types the server sends `-1`
    /// (no modifier). Parse with
    /// [`SqlType::from_oid_and_modifier`](crate::types::SqlType::from_oid_and_modifier).
    pub(crate) type_modifier: i32,
    pub(crate) format: ColumnFormat,
}

impl Column {
    /// Creates a new Column.
    #[inline]
    pub(crate) fn new(
        name: String,
        type_oid: Oid,
        type_modifier: i32,
        format: ColumnFormat,
    ) -> Self {
        Column {
            name,
            type_oid,
            type_modifier,
            format,
        }
    }

    /// Returns the column name.
    #[inline]
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the column type OID.
    #[inline]
    #[must_use]
    pub fn type_oid(&self) -> Oid {
        self.type_oid
    }

    /// Returns the column's type modifier (PostgreSQL-style `atttypmod`).
    ///
    /// For `NUMERIC` columns this encodes precision and scale and is
    /// required for correct decode (see
    /// [`crate::types::SqlType::from_oid_and_modifier`]). For most other
    /// types the server sends `-1` to indicate "no modifier".
    #[inline]
    #[must_use]
    pub fn type_modifier(&self) -> i32 {
        self.type_modifier
    }

    /// Returns the data format for this column.
    #[inline]
    #[must_use]
    pub fn format(&self) -> ColumnFormat {
        self.format
    }
}

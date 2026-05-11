// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Hyper type OIDs and type information.
//!
//! OIDs (Object Identifiers) are integer tags that identify SQL types on the
//! wire. Hyper reuses PostgreSQL's OID numbering for standard types and adds
//! its own OIDs for extensions like `GEOGRAPHY` (OID 5003).
//!
//! This module provides:
//! - [`Oid`] -- A newtype wrapper around `u32` for type safety.
//! - [`oids`] -- Constants for every well-known Hyper OID.
//! - [`Type`] -- Pairs an OID with an optional type modifier (e.g., precision/scale
//!   for `NUMERIC`, max length for `VARCHAR`).
//!
//! The protocol layer (`hyper-protocol`) sends OIDs in `RowDescription` messages;
//! the client layer decodes them into [`SqlType`](crate::types::SqlType) via
//! [`SqlType::from_oid_and_modifier`](crate::types::SqlType::from_oid_and_modifier).
//!
//! # Attribution
//!
//! The [`Oid`] newtype + constants-module pattern was adapted from
//! [`postgres-types`](https://github.com/sfackler/rust-postgres) (Copyright
//! (c) 2016 Steven Fackler, MIT or Apache-2.0). The OID numeric values are
//! PostgreSQL spec; the constant names diverged ([`oids::BIG_INT`] /
//! [`oids::SMALL_INT`] / [`oids::INT`] vs. upstream `INT8` / `INT2` / `INT4`)
//! for semantic clarity at the call site. Hyper-specific OIDs (e.g.
//! [`oids::GEOGRAPHY`] = 5003) were added on top. See the `NOTICE` file at
//! the repo root for the full upstream copyright and reproduced license text.

/// A Hyper Object Identifier (OID).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Oid(pub u32);

impl Oid {
    /// Creates a new OID from a raw value.
    pub const fn new(value: u32) -> Self {
        Oid(value)
    }

    /// Returns the raw OID value.
    pub const fn value(self) -> u32 {
        self.0
    }
}

impl From<u32> for Oid {
    fn from(value: u32) -> Self {
        Oid(value)
    }
}

impl From<Oid> for u32 {
    fn from(oid: Oid) -> Self {
        oid.0
    }
}

/// Well-known Hyper type OIDs.
///
/// These constants match the values used by the Hyper server and C API.
/// Standard PostgreSQL OIDs (e.g., `INT` = 23, `TEXT` = 25) are identical
/// to upstream PostgreSQL. Hyper-specific OIDs (e.g., `GEOGRAPHY` = 5003)
/// are in a non-conflicting range.
pub mod oids {
    use super::Oid;

    /// Boolean type
    pub const BOOL: Oid = Oid(16);
    /// 64-bit integer (BIGINT)
    pub const BIG_INT: Oid = Oid(20);
    /// 16-bit integer (SMALLINT)
    pub const SMALL_INT: Oid = Oid(21);
    /// 32-bit integer (INT)
    pub const INT: Oid = Oid(23);
    /// Arbitrary precision numeric
    pub const NUMERIC: Oid = Oid(1700);
    /// 32-bit floating point (REAL/FLOAT4)
    pub const FLOAT: Oid = Oid(700);
    /// 64-bit floating point (DOUBLE PRECISION/FLOAT8)
    pub const DOUBLE: Oid = Oid(701);
    /// Object identifier
    pub const OID: Oid = Oid(26);
    /// Binary data (BYTEA)
    pub const BYTE_A: Oid = Oid(17);
    /// Variable-length text
    pub const TEXT: Oid = Oid(25);
    /// Variable-length character with limit
    pub const VARCHAR: Oid = Oid(1043);
    /// Fixed-length character
    pub const CHAR: Oid = Oid(1042);
    /// Single character (internal)
    #[allow(
        dead_code,
        reason = "exposed for completeness; Postgres catalog value not currently mapped by the crate"
    )]
    pub const CHAR1: Oid = Oid(18);
    /// JSON data
    pub const JSON: Oid = Oid(114);
    /// Date (days since epoch)
    pub const DATE: Oid = Oid(1082);
    /// Time interval
    pub const INTERVAL: Oid = Oid(1186);
    /// Time of day (microseconds since midnight)
    pub const TIME: Oid = Oid(1083);
    /// Timestamp without timezone
    pub const TIMESTAMP: Oid = Oid(1114);
    /// Timestamp with timezone
    pub const TIMESTAMP_TZ: Oid = Oid(1184);
    /// Tableau geography type (matches HYPER_OID_GEOGRAPHY in C API)
    pub const GEOGRAPHY: Oid = Oid(5003);
}

/// Hyper SQL type information (OID + optional modifier).
///
/// The modifier encodes type-specific parameters using PostgreSQL conventions:
/// - **NUMERIC**: `((precision << 16) | scale) + 4`
/// - **VARCHAR**: `max_length + 4`
/// - **CHAR**: `length + 4`
///
/// The `+ 4` offset is a PostgreSQL convention that reserves values < 4 as
/// "unspecified". See [`SqlType::from_oid_and_modifier`](crate::types::SqlType::from_oid_and_modifier)
/// for the decoding logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Type {
    /// The type OID
    pub oid: Oid,
    /// Type modifier (e.g., VARCHAR length, NUMERIC precision/scale)
    pub modifier: Option<i32>,
}

impl Type {
    /// Creates a new Type with the given OID and no modifier.
    pub const fn new(oid: Oid) -> Self {
        Type {
            oid,
            modifier: None,
        }
    }

    /// Creates a new Type with the given OID and modifier.
    pub const fn with_modifier(oid: Oid, modifier: i32) -> Self {
        Type {
            oid,
            modifier: Some(modifier),
        }
    }

    /// Returns true if this is a variable-length type.
    pub fn is_variable_length(&self) -> bool {
        matches!(
            self.oid,
            oids::TEXT | oids::VARCHAR | oids::CHAR | oids::BYTE_A | oids::JSON | oids::GEOGRAPHY
        )
    }

    /// Returns the fixed size of this type in bytes, if it has a fixed size.
    pub fn fixed_size(&self) -> Option<usize> {
        match self.oid {
            oids::BOOL => Some(1),
            oids::SMALL_INT => Some(2),
            oids::INT | oids::FLOAT | oids::DATE | oids::OID => Some(4),
            oids::BIG_INT | oids::DOUBLE | oids::TIME | oids::TIMESTAMP | oids::TIMESTAMP_TZ => {
                Some(8)
            }
            oids::NUMERIC | oids::INTERVAL => Some(16),
            _ => None, // Variable-length types
        }
    }
}

// Convenience constructors for common types
impl Type {
    /// BOOL type
    pub const fn bool() -> Self {
        Type::new(oids::BOOL)
    }
    /// SMALLINT type
    pub const fn small_int() -> Self {
        Type::new(oids::SMALL_INT)
    }
    /// INT type
    pub const fn int() -> Self {
        Type::new(oids::INT)
    }
    /// BIGINT type
    pub const fn big_int() -> Self {
        Type::new(oids::BIG_INT)
    }
    /// REAL/FLOAT4 type
    pub const fn float() -> Self {
        Type::new(oids::FLOAT)
    }
    /// DOUBLE PRECISION type
    pub const fn double() -> Self {
        Type::new(oids::DOUBLE)
    }
    /// TEXT type
    pub const fn text() -> Self {
        Type::new(oids::TEXT)
    }
    /// BYTEA type
    pub const fn byte_a() -> Self {
        Type::new(oids::BYTE_A)
    }
    /// DATE type
    pub const fn date() -> Self {
        Type::new(oids::DATE)
    }
    /// TIME type
    pub const fn time() -> Self {
        Type::new(oids::TIME)
    }
    /// TIMESTAMP type
    pub const fn timestamp() -> Self {
        Type::new(oids::TIMESTAMP)
    }
    /// TIMESTAMPTZ type
    pub const fn timestamp_tz() -> Self {
        Type::new(oids::TIMESTAMP_TZ)
    }
    /// INTERVAL type
    pub const fn interval() -> Self {
        Type::new(oids::INTERVAL)
    }
    /// NUMERIC type
    pub const fn numeric() -> Self {
        Type::new(oids::NUMERIC)
    }
    /// GEOGRAPHY type
    pub const fn geography() -> Self {
        Type::new(oids::GEOGRAPHY)
    }
    /// OID type
    pub const fn oid() -> Self {
        Type::new(oids::OID)
    }
    /// JSON type
    pub const fn json() -> Self {
        Type::new(oids::JSON)
    }

    /// VARCHAR with max length
    pub const fn varchar(max_length: i32) -> Self {
        Type::with_modifier(oids::VARCHAR, max_length + 4) // Postgres convention
    }

    /// CHAR with fixed length
    pub const fn char(length: i32) -> Self {
        Type::with_modifier(oids::CHAR, length + 4) // Postgres convention
    }

    /// NUMERIC with precision and scale.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - `precision` is not in range `1..=38` ([`Numeric::MAX_PRECISION`](super::Numeric::MAX_PRECISION))
    /// - `scale` is negative or greater than `precision`
    ///
    /// # Example
    ///
    /// ```
    /// use hyperdb_api_core::types::Type;
    ///
    /// let decimal = Type::numeric_with_precision(10, 2); // NUMERIC(10, 2)
    /// ```
    pub const fn numeric_with_precision(precision: i32, scale: i32) -> Self {
        // Validate inputs to prevent overflow in modifier calculation.
        // Uses Numeric::MAX_PRECISION (38) as the canonical limit.
        assert!(
            precision >= 1 && precision <= super::Numeric::MAX_PRECISION as i32,
            "NUMERIC precision must be between 1 and 38"
        );
        assert!(scale >= 0, "NUMERIC scale cannot be negative");
        assert!(scale <= precision, "NUMERIC scale cannot exceed precision");

        // Modifier format: (precision << 16) | scale, plus 4
        // Safe because precision <= 38 fits in lower 16 bits after shift
        Type::with_modifier(oids::NUMERIC, ((precision << 16) | scale) + 4)
    }
}

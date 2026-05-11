// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Parameter encoding for parameterized queries.
//!
//! This module provides the [`ToSqlParam`] trait for type-safe parameter encoding
//! in parameterized SQL queries, preventing SQL injection attacks.
//!
//! # SQL Injection Prevention
//!
//! Using parameterized queries is the safest way to include user input in SQL:
//!
//! ```no_run
//! # use hyperdb_api::{Connection, Result};
//! # fn example(conn: &Connection, user_input: &str) -> Result<()> {
//! // DANGEROUS - vulnerable to SQL injection:
//! let query = format!("SELECT * FROM users WHERE name = '{}'", user_input);
//!
//! // SAFE - parameterized query:
//! let result = conn.query_params("SELECT * FROM users WHERE name = $1", &[&user_input])?;
//! # Ok(())
//! # }
//! ```
//!
//! # Supported Types
//!
//! The following types implement [`ToSqlParam`]:
//!
//! - Integers: `i16`, `i32`, `i64`
//! - Floats: `f32`, `f64`
//! - `bool`
//! - `&str`, `String`
//! - `Option<T>` where `T: ToSqlParam` (for nullable parameters)
//! - Date/time types: `Date`, `Time`, `Timestamp`, `OffsetTimestamp`
//!
//! # Example
//!
//! ```no_run
//! use hyperdb_api::{Connection, CreateMode, ToSqlParam, Result};
//!
//! fn find_user(conn: &Connection, user_id: i32, name: &str) -> Result<()> {
//!     // Multiple parameters with different types
//!     let result = conn.query_params(
//!         "SELECT * FROM users WHERE id = $1 AND name = $2",
//!         &[&user_id, &name],
//!     )?;
//!     Ok(())
//! }
//! ```

use hyperdb_api_core::types::{oids, Date, OffsetTimestamp, Oid, Time, Timestamp};

/// Trait for types that can be used as parameters in parameterized SQL queries.
///
/// This trait enables type-safe parameter encoding for use with
/// [`Connection::query_params`](crate::Connection::query_params) and
/// [`Connection::command_params`](crate::Connection::command_params).
///
/// # Implementing for Custom Types
///
/// You can implement this trait for custom types:
///
/// ```no_run
/// # use hyperdb_api::ToSqlParam;
/// # struct MyType;
/// # impl MyType { fn to_bytes(&self) -> Vec<u8> { vec![] } }
/// # impl ToString for MyType { fn to_string(&self) -> String { String::new() } }
/// impl ToSqlParam for MyType {
///     fn encode_param(&self) -> Option<Vec<u8>> {
///         Some(self.to_bytes())
///     }
///
///     fn to_sql_literal(&self) -> String {
///         format!("'{}'", self.to_string().replace('\'', "''"))
///     }
/// }
/// ```
pub trait ToSqlParam: Send + Sync {
    /// Encodes this value as binary bytes for use in parameterized queries.
    ///
    /// Returns `None` to represent a SQL NULL value.
    /// Returns `Some(bytes)` with the binary-encoded value otherwise.
    fn encode_param(&self) -> Option<Vec<u8>>;

    /// Returns the SQL type OID this parameter should bind as.
    ///
    /// The default returns `Oid(0)` (unspecified) which asks the server
    /// to infer the type from surrounding SQL context. That works for
    /// clauses like `WHERE column = $1` where the column type is known,
    /// but not for `INSERT INTO t VALUES ($1, $2)` — those require the
    /// caller (or the trait impl) to return a concrete OID.
    ///
    /// All built-in `ToSqlParam` impls override this with a concrete
    /// value from [`hyperdb_api_core::types::oids`].
    fn sql_oid(&self) -> Oid {
        Oid::new(0)
    }

    /// Returns the SQL literal representation of this value.
    ///
    /// Retained for building DDL statement strings that cannot use
    /// parameterized queries (e.g. `escape_sql_path` in catalog code).
    /// The parameterized-query path in
    /// [`Connection::query_params`](crate::Connection::query_params)
    /// no longer uses this method — parameters travel as binary bytes
    /// via `encode_param`.
    fn to_sql_literal(&self) -> String;
}

// =============================================================================
// Integer implementations
// =============================================================================

impl ToSqlParam for i16 {
    fn encode_param(&self) -> Option<Vec<u8>> {
        // PostgreSQL wire-protocol Bind uses big-endian for numeric
        // binary parameters. (Results come back as little-endian
        // HyperBinary because we request format code 2 for results;
        // params use format code 1 = standard PG binary = BE.)
        Some(self.to_be_bytes().to_vec())
    }

    fn sql_oid(&self) -> Oid {
        oids::SMALL_INT
    }

    fn to_sql_literal(&self) -> String {
        self.to_string()
    }
}

impl ToSqlParam for i32 {
    fn encode_param(&self) -> Option<Vec<u8>> {
        Some(self.to_be_bytes().to_vec())
    }

    fn sql_oid(&self) -> Oid {
        oids::INT
    }

    fn to_sql_literal(&self) -> String {
        self.to_string()
    }
}

impl ToSqlParam for i64 {
    fn encode_param(&self) -> Option<Vec<u8>> {
        Some(self.to_be_bytes().to_vec())
    }

    fn sql_oid(&self) -> Oid {
        oids::BIG_INT
    }

    fn to_sql_literal(&self) -> String {
        self.to_string()
    }
}

// =============================================================================
// Float implementations
// =============================================================================

impl ToSqlParam for f32 {
    fn encode_param(&self) -> Option<Vec<u8>> {
        Some(self.to_be_bytes().to_vec())
    }

    fn sql_oid(&self) -> Oid {
        oids::FLOAT
    }

    fn to_sql_literal(&self) -> String {
        // Handle special float values
        if self.is_nan() {
            "'NaN'".to_string()
        } else if self.is_infinite() {
            if *self > 0.0 {
                "'Infinity'".to_string()
            } else {
                "'-Infinity'".to_string()
            }
        } else {
            self.to_string()
        }
    }
}

impl ToSqlParam for f64 {
    fn encode_param(&self) -> Option<Vec<u8>> {
        Some(self.to_be_bytes().to_vec())
    }

    fn sql_oid(&self) -> Oid {
        oids::DOUBLE
    }

    fn to_sql_literal(&self) -> String {
        // Handle special float values
        if self.is_nan() {
            "'NaN'".to_string()
        } else if self.is_infinite() {
            if *self > 0.0 {
                "'Infinity'".to_string()
            } else {
                "'-Infinity'".to_string()
            }
        } else {
            self.to_string()
        }
    }
}

// =============================================================================
// Boolean implementation
// =============================================================================

impl ToSqlParam for bool {
    fn encode_param(&self) -> Option<Vec<u8>> {
        Some(vec![u8::from(*self)])
    }

    fn sql_oid(&self) -> Oid {
        oids::BOOL
    }

    fn to_sql_literal(&self) -> String {
        if *self { "TRUE" } else { "FALSE" }.to_string()
    }
}

// =============================================================================
// String implementations
// =============================================================================

impl ToSqlParam for str {
    fn encode_param(&self) -> Option<Vec<u8>> {
        Some(self.as_bytes().to_vec())
    }

    fn sql_oid(&self) -> Oid {
        oids::TEXT
    }

    fn to_sql_literal(&self) -> String {
        // Escape single quotes by doubling them
        format!("'{}'", self.replace('\'', "''"))
    }
}

impl ToSqlParam for String {
    fn encode_param(&self) -> Option<Vec<u8>> {
        Some(self.as_bytes().to_vec())
    }

    fn sql_oid(&self) -> Oid {
        oids::TEXT
    }

    fn to_sql_literal(&self) -> String {
        format!("'{}'", self.replace('\'', "''"))
    }
}

impl ToSqlParam for &str {
    fn encode_param(&self) -> Option<Vec<u8>> {
        Some(self.as_bytes().to_vec())
    }

    fn sql_oid(&self) -> Oid {
        oids::TEXT
    }

    fn to_sql_literal(&self) -> String {
        format!("'{}'", self.replace('\'', "''"))
    }
}

// =============================================================================
// Reference implementations
// =============================================================================

impl<T: ToSqlParam> ToSqlParam for &T {
    fn encode_param(&self) -> Option<Vec<u8>> {
        (*self).encode_param()
    }

    fn sql_oid(&self) -> Oid {
        (*self).sql_oid()
    }

    fn to_sql_literal(&self) -> String {
        (*self).to_sql_literal()
    }
}

// =============================================================================
// Option implementation (for nullable parameters)
// =============================================================================

impl<T: ToSqlParam> ToSqlParam for Option<T> {
    fn encode_param(&self) -> Option<Vec<u8>> {
        match self {
            Some(value) => value.encode_param(),
            None => None, // SQL NULL
        }
    }

    fn sql_oid(&self) -> Oid {
        match self {
            Some(value) => value.sql_oid(),
            // For NULL we leave the OID unspecified — server infers
            // from context, which is the correct behavior for `WHERE
            // col = $1` with a NULL binding.
            None => Oid::new(0),
        }
    }

    fn to_sql_literal(&self) -> String {
        match self {
            Some(value) => value.to_sql_literal(),
            None => "NULL".to_string(),
        }
    }
}

// =============================================================================
// Date/Time implementations
// =============================================================================

impl ToSqlParam for Date {
    fn encode_param(&self) -> Option<Vec<u8>> {
        // Date is stored as i32 Julian day offset from 2000-01-01.
        // Big-endian per the PG Bind protocol (format code 1).
        Some(self.to_julian_day().to_be_bytes().to_vec())
    }

    fn sql_oid(&self) -> Oid {
        oids::DATE
    }

    fn to_sql_literal(&self) -> String {
        format!("DATE '{self}'")
    }
}

impl ToSqlParam for Time {
    fn encode_param(&self) -> Option<Vec<u8>> {
        // Time is stored as i64 microseconds since midnight.
        Some(self.to_microseconds().to_be_bytes().to_vec())
    }

    fn sql_oid(&self) -> Oid {
        oids::TIME
    }

    fn to_sql_literal(&self) -> String {
        format!("TIME '{self}'")
    }
}

impl ToSqlParam for Timestamp {
    fn encode_param(&self) -> Option<Vec<u8>> {
        // Timestamp is stored as i64 microseconds since 2000-01-01.
        Some(self.to_microseconds().to_be_bytes().to_vec())
    }

    fn sql_oid(&self) -> Oid {
        oids::TIMESTAMP
    }

    fn to_sql_literal(&self) -> String {
        format!("TIMESTAMP '{self}'")
    }
}

impl ToSqlParam for OffsetTimestamp {
    fn encode_param(&self) -> Option<Vec<u8>> {
        // OffsetTimestamp is stored as i64 microseconds UTC since 2000-01-01.
        Some(self.to_microseconds_utc().to_be_bytes().to_vec())
    }

    fn sql_oid(&self) -> Oid {
        oids::TIMESTAMP_TZ
    }

    fn to_sql_literal(&self) -> String {
        format!("TIMESTAMPTZ '{self}'")
    }
}

// =============================================================================
// Bytes implementation
// =============================================================================

impl ToSqlParam for [u8] {
    fn encode_param(&self) -> Option<Vec<u8>> {
        Some(self.to_vec())
    }

    fn sql_oid(&self) -> Oid {
        oids::BYTE_A
    }

    #[expect(
        clippy::format_collect,
        reason = "readable hex/string formatting loop; refactoring to fold! obscures intent"
    )]
    fn to_sql_literal(&self) -> String {
        // Encode as hex bytea literal
        let hex_str: String = self.iter().map(|b| format!("{b:02x}")).collect();
        format!("E'\\\\x{hex_str}'")
    }
}

impl ToSqlParam for Vec<u8> {
    fn encode_param(&self) -> Option<Vec<u8>> {
        Some(self.clone())
    }

    fn sql_oid(&self) -> Oid {
        oids::BYTE_A
    }

    #[expect(
        clippy::format_collect,
        reason = "readable hex/string formatting loop; refactoring to fold! obscures intent"
    )]
    fn to_sql_literal(&self) -> String {
        let hex_str: String = self.iter().map(|b| format!("{b:02x}")).collect();
        format!("E'\\\\x{hex_str}'")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_i32_encoding() {
        // Big-endian per PG Bind format code 1.
        assert_eq!(42i32.encode_param(), Some(vec![0, 0, 0, 42]));
        assert_eq!((-1i32).encode_param(), Some(vec![255, 255, 255, 255]));
    }

    #[test]
    fn test_i64_encoding() {
        assert_eq!(42i64.encode_param(), Some(vec![0, 0, 0, 0, 0, 0, 0, 42]));
    }

    #[test]
    fn test_string_encoding() {
        assert_eq!("hello".encode_param(), Some(b"hello".to_vec()));
        assert_eq!(
            String::from("world").encode_param(),
            Some(b"world".to_vec())
        );
    }

    #[test]
    fn test_bool_encoding() {
        assert_eq!(true.encode_param(), Some(vec![1]));
        assert_eq!(false.encode_param(), Some(vec![0]));
    }

    #[test]
    fn test_option_encoding() {
        // Big-endian per PG Bind format code 1.
        assert_eq!(Some(42i32).encode_param(), Some(vec![0, 0, 0, 42]));
        assert_eq!(None::<i32>.encode_param(), None);
    }

    #[test]
    fn test_reference_encoding() {
        let value = 42i32;
        assert_eq!(value.encode_param(), Some(vec![0, 0, 0, 42]));
        assert_eq!((&&value).encode_param(), Some(vec![0, 0, 0, 42]));
    }
}

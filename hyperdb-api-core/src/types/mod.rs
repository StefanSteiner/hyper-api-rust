// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(
    missing_docs,
    reason = "internal crate; not published to crates.io. Docs are contributor-facing, covered in DEVELOPMENT.md."
)]

//! Type conversions for Hyper database using LittleEndian (HyperBinary) format.
//!
//! This crate provides serialization and deserialization of Rust types to and from
//! Hyper's native binary format. Unlike standard PostgreSQL which uses BigEndian
//! (network byte order), Hyper uses LittleEndian encoding for performance on x86.
//!
//! # HyperBinary Format
//!
//! The HyperBinary format has these characteristics:
//! - All multi-byte integers are LittleEndian
//! - Nullable values have a 1-byte NULL indicator prefix (0 = not null, 1 = null)
//! - Variable-length types (text, bytea) have a 4-byte LittleEndian length prefix
//! - Fixed-size types have known sizes (e.g., i32 = 4 bytes, i64 = 8 bytes)
//!
//! # Example
//!
//! ```rust
//! use hyperdb_api_core::types::ToHyperBinary;
//! use bytes::BytesMut;
//!
//! let mut buf = BytesMut::new();
//! // Write i32 without null indicator (for NOT NULL columns)
//! 42i32.to_hyper_binary_not_null(&mut buf).unwrap();
//! assert_eq!(buf.as_ref(), &[42, 0, 0, 0]); // LittleEndian
//!
//! // With null indicator for nullable columns
//! let mut buf2 = BytesMut::new();
//! 42i32.to_hyper_binary(&mut buf2).unwrap();
//! assert_eq!(buf2.as_ref(), &[0, 42, 0, 0, 0]); // [not-null][value LE]
//! ```

#![warn(missing_docs, rust_2018_idioms, clippy::all)]

// Type implementations (ToHyperBinary/FromHyperBinary for primitive types)
mod oid;
#[cfg(kani)]
mod proofs;
mod special;
mod sql_type;
mod traits;
#[allow(
    clippy::module_inception,
    reason = "preserved submodule name after collapsing the old hyper-types crate into hyperdb-api-core/src/types/"
)]
mod types;

pub use oid::{oids, Oid, Type};
pub use special::{Date, Geography, Interval, Numeric, OffsetTimestamp, Time, Timestamp};
pub use sql_type::{ColumnDefinition, Nullability, SqlType};
pub use traits::{FromHyperBinary, IsNull, ToHyperBinary};

// Re-export GeoError
pub use special::GeoError;

// Chrono integration
mod chrono_integration;
pub use chrono_integration::ChronoConversionError;

/// Re-export bytes for convenience
pub use bytes;

// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(
    missing_docs,
    reason = "internal crate; not published to crates.io. Docs are contributor-facing, covered in DEVELOPMENT.md."
)]

//! Wire protocol implementation for Hyper database.
//!
//! This module implements the `PostgreSQL` wire protocol with Hyper-specific
//! extensions for `LittleEndian` binary format (`HyperBinary`). It sits between
//! the sibling `types` module (type system) and `client` module (connection management).
//!
//! # Two-Layer Byte Order Design
//!
//! The Hyper wire protocol has two layers with *different* byte orders:
//!
//! | Layer | Byte Order | Modules | Rationale |
//! |---|---|---|---|
//! | Message framing | **`BigEndian`** | [`message`] | `PostgreSQL` specification compatibility |
//! | Data encoding | **`LittleEndian`** | [`copy`], [`types`] | Hyper's native format, optimized for x86/x64 |
//!
//! All message headers, lengths, and column counts use `BigEndian` (network byte
//! order), exactly like standard `PostgreSQL`. Actual data values inside those
//! messages use `LittleEndian`, which is Hyper's native format. This lets Hyper
//! stay compatible with `PostgreSQL` client libraries while avoiding byte-swapping
//! on the dominant deployment architectures.
//!
//! # Modules
//!
//! - [`message`] -- Frontend (client-to-server) and backend (server-to-client)
//!   message framing, parsing, and construction. All lengths are `BigEndian`.
//! - [`copy`] -- `HyperBinary` COPY format: header, row encoding, and read helpers.
//!   All data values are `LittleEndian`.
//! - [`types`] -- Conversion between Rust types and `HyperBinary` format
//!   (`LittleEndian` encoding/decoding).
//! - [`escape`] -- SQL identifier and literal escaping via zero-cost newtype
//!   wrappers with [`std::fmt::Display`] implementations.

#![warn(missing_docs, rust_2018_idioms, clippy::all)]

pub mod copy;
pub mod escape;
pub mod message;
#[cfg(kani)]
mod proofs;
pub mod types;

pub use crate::types::Oid;
pub use types::ParseError;

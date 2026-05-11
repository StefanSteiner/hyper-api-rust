// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `PostgreSQL` wire protocol message framing and parsing.
//!
//! This module implements the `PostgreSQL` v3 wire protocol's message format.
//! All structural fields (message tags, lengths, column counts, OIDs, format
//! codes) use **`BigEndian`** (network byte order), exactly per the `PostgreSQL`
//! specification. The *data payloads* inside `DataRow` and COPY messages use
//! `LittleEndian` (`HyperBinary`) -- see [`crate::protocol::copy`] and [`crate::types`].
//!
//! # Message Format
//!
//! ```text
//! ┌─────────┬─────────────────┬─────────────────────────────┐
//! │ Tag (1) │ Length (4, BE)  │ Payload (variable)          │
//! └─────────┴─────────────────┴─────────────────────────────┘
//! ```
//!
//! - **Tag**: 1-byte identifier (e.g. `b'Q'` for Query, `b'D'` for `DataRow`).
//!   The startup message is the sole exception -- it has no tag.
//! - **Length**: 4-byte `BigEndian` `u32`, includes itself but *not* the tag.
//! - **Payload**: Tag-specific; see [`frontend`] and [`backend`] for details.
//!
//! # Sub-modules
//!
//! - [`frontend`] -- Messages sent from the client to the server.
//! - [`backend`] -- Messages sent from the server to the client.

pub mod backend;
pub mod frontend;

pub use backend::Message;

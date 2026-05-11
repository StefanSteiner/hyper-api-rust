// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Internal implementation details for [`hyperdb-api`](https://crates.io/crates/hyperdb-api).
//!
//! **This crate is not a public API.** Use `hyperdb-api` directly. The items
//! exposed here may change between any two releases, including patch releases,
//! without semver deprecation.
//!
//! # Organization
//!
//! - [`types`] — SQL type system, binary encoding, OIDs (ex-`hyper-types`)
//! - [`protocol`] — `PostgreSQL` wire protocol messages, `HyperBinary` COPY format (ex-`hyper-protocol`)
//! - [`client`] — Sync/async TCP and gRPC clients, auth, TLS (ex-`hyper-client`)

#![allow(
    missing_docs,
    reason = "internal crate; contributor-facing docs live in the per-layer DEVELOPMENT-*.md files in docs/"
)]

pub mod client;
pub mod protocol;
pub mod types;

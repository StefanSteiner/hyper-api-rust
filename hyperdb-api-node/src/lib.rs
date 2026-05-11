// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![deny(clippy::all)]
#![allow(
    missing_docs,
    reason = "Node.js binding crate; public surface is exported to JS via napi-rs and documented in the TypeScript declarations, not rustdoc."
)]

mod arrow_inserter;
mod catalog;
mod columnar;
mod connection;
mod inserter;
mod prepared;
mod process;
mod query_stats;
mod query_stream;
mod result;
mod types;

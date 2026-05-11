// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Version reporting for the `HyperDB` MCP server.
//!
//! Exposes two version strings used by `status` and the
//! `hyper://workspace` resource so LLMs and humans alike can correlate
//! a running server with a specific point in source control:
//!
//! * [`mcp_version_string`] — the `hyperdb-mcp` crate's own version.
//! * [`hyper_api_version_string`] — the `hyperdb-api` pure-Rust Hyper
//!   client library version (pulled from [`hyperdb_api::VERSION`]).
//!
//! Both are postfixed with the short git commit hash of the workspace
//! they were built from, formatted as `.r<8-char-hash>`. When the
//! working tree had uncommitted changes at build time the suffix is
//! extended to `.r<hash>-dirty-<YYYYMMDDTHHMMSSZ>` — a literal
//! `-dirty` marker plus an ISO 8601 basic UTC build timestamp, so
//! iterative dirty rebuilds of the same commit can still be told
//! apart by their version string alone. The hash and timestamp are
//! captured by `build.rs` at compile time. For source trees without a
//! working `git` binary the suffix falls back to `.runknown`.

/// Semantic version of this crate, from `Cargo.toml`.
pub const MCP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Short git hash of the workspace HEAD at build time. When the tree
/// was dirty at build time the value is extended to
/// `<hash>-dirty-<YYYYMMDDTHHMMSSZ>` (ISO 8601 basic UTC). Falls back
/// to `"unknown"` when git is unavailable.
pub const GIT_HASH: &str = env!("HYPERDB_GIT_HASH");

/// Version string reported for the `HyperDB` MCP server.
///
/// Example clean build: `"0.1.0.ra1b2c3d4"`.
/// Example dirty build: `"0.1.0.ra1b2c3d4-dirty-20260423T184900Z"`.
#[must_use]
pub fn mcp_version_string() -> String {
    format!("{MCP_VERSION}.r{GIT_HASH}")
}

/// Version string reported for the underlying pure-Rust Hyper API
/// (the `hyperdb-api` crate). Shares the same git hash suffix as the MCP
/// crate because both live in the same workspace.
///
/// Example: `"0.1.0.ra1b2c3d4"`.
#[must_use]
pub fn hyper_api_version_string() -> String {
    format!("{}.r{GIT_HASH}", hyperdb_api::VERSION)
}

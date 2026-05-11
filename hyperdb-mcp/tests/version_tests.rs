// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for the version helpers exposed by [`hyperdb_mcp::version`].
//!
//! We can't assert a specific hash (it changes every commit), but we can
//! check the general shape: `<semver>.r<hash>` where `<hash>` is one of
//!
//! * `<8-hex-chars>` (clean build),
//! * `<8-hex-chars>-dirty-<YYYYMMDDTHHMMSSZ>` (dirty build, with an
//!   ISO 8601 basic UTC timestamp so iterative rebuilds of the same
//!   commit can be told apart),
//! * `unknown` (no git binary at build time).

use hyperdb_mcp::version::{hyper_api_version_string, mcp_version_string, GIT_HASH, MCP_VERSION};

/// `YYYYMMDDTHHMMSSZ` — 16 chars, all digits except a `T` at position
/// 8 and a `Z` at position 15. Deliberately strict so a malformed
/// timestamp (e.g. missing `Z`, wrong separator) is caught here
/// rather than percolating into logs and status payloads.
fn is_valid_build_timestamp(ts: &str) -> bool {
    let bytes = ts.as_bytes();
    if bytes.len() != 16 {
        return false;
    }
    bytes.iter().enumerate().all(|(i, &b)| match i {
        8 => b == b'T',
        15 => b == b'Z',
        _ => b.is_ascii_digit(),
    })
}

fn is_valid_hash(hash: &str) -> bool {
    if hash == "unknown" {
        return true;
    }
    // Dirty builds: `<hash>-dirty-<YYYYMMDDTHHMMSSZ>`.
    if let Some(rest) = hash.strip_prefix("unknown-dirty-") {
        return is_valid_build_timestamp(rest);
    }
    if let Some((base, rest)) = hash.split_once("-dirty-") {
        return base.len() == 8
            && base.chars().all(|c| c.is_ascii_hexdigit())
            && is_valid_build_timestamp(rest);
    }
    // Clean builds: just the 8-char hex hash.
    hash.len() == 8 && hash.chars().all(|c| c.is_ascii_hexdigit())
}

#[test]
fn mcp_version_has_semver_and_r_suffix() {
    let v = mcp_version_string();
    let (semver, hash) = v
        .split_once(".r")
        .unwrap_or_else(|| panic!("expected `<semver>.r<hash>`; got: {v}"));
    assert_eq!(semver, MCP_VERSION);
    assert!(
        is_valid_hash(hash),
        "hash portion of {v:?} is not 8 hex chars or 'unknown' (got {hash:?})",
    );
}

#[test]
fn hyper_api_version_has_hyperdb_api_semver_and_r_suffix() {
    let v = hyper_api_version_string();
    let (semver, hash) = v
        .split_once(".r")
        .unwrap_or_else(|| panic!("expected `<semver>.r<hash>`; got: {v}"));
    assert_eq!(
        semver,
        hyperdb_api::VERSION,
        "hyper_api_version must report hyperdb_api::VERSION, not hyperdb-mcp's"
    );
    assert!(is_valid_hash(hash));
}

/// Both helpers must embed the same git hash — the whole workspace
/// builds from a single commit, and having two different hashes in the
/// status output would be a red flag.
#[test]
fn both_versions_share_the_same_git_hash() {
    let mcp = mcp_version_string();
    let hyper = hyper_api_version_string();
    let mcp_hash = mcp.split_once(".r").unwrap().1;
    let hyper_hash = hyper.split_once(".r").unwrap().1;
    assert_eq!(mcp_hash, hyper_hash);
    assert_eq!(mcp_hash, GIT_HASH);
}

/// The build-timestamp suffix is only present on dirty builds. Check
/// that clean builds don't carry one (the commit hash alone already
/// identifies them uniquely), and that dirty builds always do.
#[test]
fn dirty_builds_have_timestamp_clean_builds_do_not() {
    if GIT_HASH.contains("-dirty") {
        let (_base, ts) = GIT_HASH.split_once("-dirty-").unwrap_or_else(|| {
            panic!("dirty hash must carry `-dirty-<timestamp>`; got {GIT_HASH}")
        });
        assert!(
            is_valid_build_timestamp(ts),
            "dirty-build timestamp {ts:?} is not ISO 8601 basic UTC (YYYYMMDDTHHMMSSZ)",
        );
    } else {
        assert!(
            !GIT_HASH.contains('T') && !GIT_HASH.contains('Z'),
            "clean build hash should not contain a timestamp; got {GIT_HASH:?}",
        );
    }
}

#[test]
fn print_live_version_strings() {
    // Not really a test — just prints the compiled-in version so we
    // can eyeball the dirty-build timestamp shape during development
    // (`cargo test ... -- --nocapture print_live_version_strings`).
    println!("  GIT_HASH           = {GIT_HASH}");
    println!("  mcp_version_string = {}", mcp_version_string());
    println!("  hyper_api_version  = {}", hyper_api_version_string());
}

#[test]
fn timestamp_shape_validator_accepts_and_rejects() {
    assert!(is_valid_build_timestamp("20260423T184900Z"));
    assert!(is_valid_build_timestamp("20000101T000000Z"));
    // Missing `Z`.
    assert!(!is_valid_build_timestamp("20260423T184900"));
    // Wrong separator.
    assert!(!is_valid_build_timestamp("20260423-184900Z"));
    // Too short.
    assert!(!is_valid_build_timestamp("20260423T1849Z"));
    // Non-digit where a digit is required.
    assert!(!is_valid_build_timestamp("2026042XT184900Z"));
}

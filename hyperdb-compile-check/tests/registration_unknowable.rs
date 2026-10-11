// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `registry::registration_unknowable` and its effect on `validate_query_as`.
//!
//! The registry is process-global, so this lives in its own integration-test
//! binary (one process) and runs the empty -> populated transition inside a
//! single `#[test]` so it cannot race with other registrations.
//!
//! The rust-analyzer arm (`running_under_rust_analyzer`) is not covered: it
//! keys off `std::env::current_exe()` having a `rust-analyzer*` file stem,
//! cached in a `OnceLock`, and the test binary's name cannot be faked.

use hyperdb_compile_check::{ValidationError, registry, validate_query_as};

#[test]
fn empty_registry_skips_then_populated_registry_reports_unregistered() {
    // Empty registry: "not registered" is not evidence of a missing derive.
    assert!(registry::is_empty());
    assert!(registry::registration_unknowable());
    validate_query_as("NeverRegistered", "SELECT 1 AS id")
        .expect("an empty registry must skip validation, not report an error");

    // Populated registry (under cargo, not rust-analyzer): full validation.
    registry::register(
        "RegUnknowableUser",
        "reg_unknowable_users",
        "CREATE TABLE reg_unknowable_users (id BIGINT)",
        vec!["id".into()],
    );
    assert!(!registry::is_empty());
    assert!(!registry::registration_unknowable());
    let err = validate_query_as("StillNeverRegistered", "SELECT 1 AS id")
        .expect_err("a populated registry must report an unknown struct");
    assert!(
        matches!(err, ValidationError::StructNotRegistered { .. }),
        "expected StructNotRegistered, got: {err:?}"
    );
}

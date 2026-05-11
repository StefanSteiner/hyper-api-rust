// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Regression tests for the Claude-session bug: `sample` returning a
//! spurious `TABLE_NOT_FOUND` due to a racy `has_table` probe, and the
//! connection-lost detection heuristics that drive auto-reconnect.

mod common;

use common::TestEngine;
use hyperdb_mcp::error::{is_connection_lost, ErrorCode};

/// After creating a table and inserting rows, `sample_table` must return the
/// rows — not `TABLE_NOT_FOUND`. The old implementation used `has_table` with
/// `.unwrap_or(false)`, which silently returned false on any catalog read
/// hiccup.
#[test]
fn sample_works_immediately_after_insert() {
    let te = TestEngine::new_ephemeral();
    te.engine
        .execute_command("CREATE TABLE recent (id INT, label TEXT)")
        .unwrap();
    te.engine
        .execute_command("INSERT INTO recent VALUES (1, 'a'), (2, 'b'), (3, 'c')")
        .unwrap();

    let sample = te.engine.sample_table("recent", 10).unwrap();
    assert_eq!(sample["table"], "recent");
    assert_eq!(sample["row_count"], 3);
    let rows = sample["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
}

/// `sample_table` on a missing table must return `TABLE_NOT_FOUND` (not the
/// underlying Hyper "does not exist (42P01)" message).
#[test]
fn sample_missing_table_translates_to_table_not_found() {
    let te = TestEngine::new_ephemeral();
    let err = te
        .engine
        .sample_table("this_table_does_not_exist", 5)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::TableNotFound);
    assert!(err.message.contains("this_table_does_not_exist"));
}

/// Classifier: recognizes the OS-level "broken pipe" error from a dead
/// hyperd, along with related transport-level failure strings.
#[test]
fn connection_lost_classifier_recognizes_transport_errors() {
    assert!(is_connection_lost("Broken pipe (os error 32)"));
    assert!(is_connection_lost("Connection reset by peer"));
    assert!(is_connection_lost("Connection refused"));
    assert!(is_connection_lost("connection closed"));
    assert!(is_connection_lost("unexpected EOF"));
    assert!(is_connection_lost(
        "server unexpectedly closed the connection"
    ));
    assert!(is_connection_lost("Socket is not connected"));
}

/// Classifier: does NOT flag ordinary SQL errors as transport errors. These
/// must keep their normal error code routing.
#[test]
fn connection_lost_classifier_ignores_sql_errors() {
    assert!(!is_connection_lost("syntax error at or near \"SELEKT\""));
    assert!(!is_connection_lost("table \"foo\" does not exist"));
    assert!(!is_connection_lost("column \"bar\" does not exist"));
    assert!(!is_connection_lost("ERROR: table already exists (42P07)"));
    assert!(!is_connection_lost(
        "ERROR: non-NULL value required (23502)"
    ));
    assert!(!is_connection_lost(""));
}

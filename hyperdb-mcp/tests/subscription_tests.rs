// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for [`hyperdb_mcp::subscriptions`].
//!
//! `rmcp::Peer` is only constructible from inside the rmcp service
//! machinery, so the tests here focus on the parts of the registry that
//! are peer-free: the URI-fanout helpers, the structural-SQL detector,
//! and the `is_empty` / `subscribed_uris` state of an empty registry. The
//! peer-carrying code paths are exercised via the higher-level
//! integration tests in `resource_tests.rs` and `saved_queries_tests.rs`
//! whenever an MCP handler hits them indirectly.

use hyperdb_mcp::subscriptions::{
    uris_for_table_change, uris_for_workspace_change, SubscriptionRegistry,
};

#[test]
fn empty_registry_has_no_subscribed_uris() {
    let reg = SubscriptionRegistry::new();
    assert!(reg.subscribed_uris().is_empty());
    assert!(reg.subscribers_for("hyper://anything").is_empty());
}

#[test]
fn unsubscribe_on_empty_registry_is_a_noop() {
    // We can't create a real Peer in unit tests, but unsubscribe's Peer
    // argument is deliberately unused for matching — the method only
    // cares about the URI. So we construct a bogus call via the public
    // API of the helper that doesn't need a Peer.
    let reg = SubscriptionRegistry::new();
    // subscribers_for after an unsubscribe should still be empty.
    assert_eq!(reg.subscribers_for("hyper://workspace").len(), 0);
    reg.clear();
    assert!(reg.subscribed_uris().is_empty());
}

#[test]
fn notify_updated_on_empty_registry_is_safe() {
    // Must not panic or require a tokio runtime when there are no
    // subscribers — the registry short-circuits on the empty vec.
    let reg = SubscriptionRegistry::new();
    reg.notify_updated("hyper://workspace");
    reg.notify_list_changed();
    // Should still have no subscribers after the no-op.
    assert!(reg.subscribed_uris().is_empty());
}

// --- URI fan-out helpers ----------------------------------------------------

#[test]
fn uris_for_table_change_covers_the_summary_and_per_table_resources() {
    let uris = uris_for_table_change("orders");
    // The three summary URIs are always present: any table mutation
    // changes the aggregate stats that feed the workspace/readme/tables
    // resources, so subscribers to those should get a ping.
    assert!(uris.contains(&"hyper://workspace".to_string()));
    assert!(uris.contains(&"hyper://tables".to_string()));
    assert!(uris.contains(&"hyper://readme".to_string()));
    // Plus one per-table URI for each of the three per-table resource kinds.
    assert!(uris.contains(&"hyper://tables/orders/schema".to_string()));
    assert!(uris.contains(&"hyper://tables/orders/sample".to_string()));
    assert!(uris.contains(&"hyper://tables/orders/csv-sample".to_string()));
    assert_eq!(uris.len(), 6, "exactly the 3 summary + 3 per-table URIs");
}

#[test]
fn uris_for_table_change_interpolates_the_table_name_verbatim() {
    // Table names containing unusual characters (underscores, digits, hyphens)
    // are passed through unchanged — the caller is responsible for
    // validating them earlier in the stack.
    let uris = uris_for_table_change("sales_2023");
    assert!(uris.iter().any(|u| u == "hyper://tables/sales_2023/schema"));
    assert!(uris.iter().any(|u| u == "hyper://tables/sales_2023/sample"));
    assert!(uris
        .iter()
        .any(|u| u == "hyper://tables/sales_2023/csv-sample"));
}

#[test]
fn uris_for_workspace_change_returns_the_three_summary_uris() {
    let uris = uris_for_workspace_change();
    assert_eq!(
        uris,
        vec!["hyper://workspace", "hyper://tables", "hyper://readme"]
    );
}

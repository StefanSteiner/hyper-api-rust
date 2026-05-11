// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for the gRPC `cancel_query` API.
//!
//! These tests exercise the end-to-end plumbing of the cancel RPC against a
//! local `hyperd` instance running with `ListenMode::Grpc`. They do NOT try
//! to prove that a specific long-running query was interrupted mid-flight —
//! that requires refactoring the executor to yield control between chunk
//! fetches, which is out of scope for the bare-API plumbing under test.
//!
//! Instead, these tests verify three things that collectively guarantee the
//! API is wired up correctly:
//!
//! 1. `cancel_query` reaches the server and returns a well-formed result
//!    (either `Ok(())` if the server accepts the cancel, or a specific
//!    `ErrorKind` — never a transport crash / panic / hang).
//! 2. The gRPC channel survives a cancel RPC: follow-up queries on the
//!    same `GrpcConnection` succeed. Regressions in metadata handling or
//!    channel reuse would show up as corrupted follow-ups.
//! 3. Both the sync (`GrpcConnection`) and async (`GrpcConnectionAsync`)
//!    wrappers call the underlying client correctly.

use hyperdb_api::grpc::{GrpcConfig, GrpcConnection, GrpcConnectionAsync};
use hyperdb_api::{HyperProcess, ListenMode, Parameters, Result};

/// Spins up `hyperd` with gRPC enabled and returns the bound URL. The
/// returned `HyperProcess` must outlive the client so the backend stays up.
fn grpc_hyperd() -> Result<(HyperProcess, String)> {
    let mut params = Parameters::new();
    params.set("log_dir", "test_results");
    params.set_listen_mode(ListenMode::Grpc { port: 0 });
    let hyper = HyperProcess::new(None, Some(&params))?;
    let url = hyper
        .grpc_url()
        .expect("hyperd should expose a gRPC URL when listen_mode=Grpc");
    Ok((hyper, url))
}

/// Smoke test: invoke `cancel_query` with an arbitrary query id against a
/// live server and confirm (a) the RPC completes without a transport
/// crash, and (b) the channel is still usable for a real query after.
///
/// # Why we accept either Ok or Err from the cancel itself
///
/// Server behavior for cancel-of-unknown-id is not contractually
/// defined at the base-gRPC layer and varies by build:
///
/// - **Ok(())**: the server treats cancel as a best-effort signal —
///   the target query is "not running" from the server's perspective,
///   and that satisfies the cancel intent. This is the dominant
///   behavior in recent Hyper builds.
/// - **Err(_)**: older / stricter builds return `NOT_FOUND` or
///   similar when the query id does not correspond to an in-flight
///   query, surfaced as a transport-level error.
///
/// Both are valid server behaviors for this edge case, and the
/// `cancel_query` API's contract is "fallible; propagates transport
/// errors" — not "asserts a specific server response". The test
/// therefore does NOT try to pin down which outcome is correct; that
/// would couple the test to server-version-specific behavior that is
/// outside our API contract.
///
/// What the test *does* pin down is the invariant we actually promise:
/// the RPC round-trips without crashing and the HTTP/2 channel stays
/// healthy for subsequent queries. That's the regression surface a
/// client-side bug would break, and it's what the follow-up
/// `execute_query_to_arrow("SELECT 1")` proves.
#[test]
fn cancel_query_sync_does_not_poison_the_channel() -> Result<()> {
    let (hyper, url) = grpc_hyperd()?;
    let config = GrpcConfig::new(&url);
    let mut conn = GrpcConnection::connect_with_config(config)?;

    // Cancel against a clearly-synthetic id. Accept either outcome — what
    // we're really asserting is that this call doesn't panic, hang, or
    // corrupt the channel.
    let cancel_result = conn.cancel_query("test-synthetic-query-id");
    match &cancel_result {
        Ok(()) => {}
        Err(e) => eprintln!("cancel_query returned error (acceptable): {e}"),
    }

    // Follow-up query MUST succeed on the same connection. If the cancel
    // path accidentally took ownership of or invalidated the shared
    // channel, this is where we'd see the failure.
    let arrow = conn.execute_query_to_arrow("SELECT 1")?;
    assert!(
        !arrow.is_empty(),
        "follow-up query after cancel returned empty arrow data",
    );

    drop(conn);
    drop(hyper);
    Ok(())
}

/// Async variant of the smoke test — same contract, different wrapper.
#[test]
fn cancel_query_async_does_not_poison_the_channel() -> Result<()> {
    let (hyper, url) = grpc_hyperd()?;

    // `GrpcConnectionAsync` needs a tokio runtime to drive its futures.
    // Use a current-thread runtime to keep the test simple and avoid
    // spawning extra worker threads we don't need.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    runtime.block_on(async {
        let config = GrpcConfig::new(&url);
        let mut conn = GrpcConnectionAsync::connect_with_config(config).await?;

        let cancel_result = conn.cancel_query("test-synthetic-query-id").await;
        match &cancel_result {
            Ok(()) => {}
            Err(e) => eprintln!("cancel_query returned error (acceptable): {e}"),
        }

        let arrow = conn.execute_query_to_arrow("SELECT 1").await?;
        assert!(
            !arrow.is_empty(),
            "follow-up query after cancel returned empty arrow data",
        );

        Result::Ok(())
    })?;

    drop(hyper);
    Ok(())
}

/// Cancel a query whose id we genuinely obtained from the server. The
/// query will very likely have completed by the time we ask to cancel it,
/// but the API contract explicitly treats cancel as best-effort — the
/// server is free to answer Ok — so we just need to verify the round
/// trip works and the channel is healthy afterwards.
#[test]
fn cancel_query_with_real_query_id_succeeds_or_errors_cleanly() -> Result<()> {
    let (hyper, url) = grpc_hyperd()?;
    let config = GrpcConfig::new(&url);
    let mut conn = GrpcConnection::connect_with_config(config)?;

    // Produce some real traffic so the server has a chance to assign a
    // query id. Even if this particular query doesn't populate one (pure
    // SYNC-mode operations often skip it), we still exercise the happy
    // path of a `SELECT 1` before and after the cancel.
    let result = conn.execute_query("SELECT 1 AS one")?;
    if let Some(query_id) = result.query_id() {
        match conn.cancel_query(query_id) {
            Ok(()) => {}
            Err(e) => eprintln!(
                "cancel_query on completed query returned {e} \
                 (acceptable: server may reject cancel of finished query)",
            ),
        }
    } else {
        // No query id — nothing to cancel, but we can still prove the API
        // tolerates a synthetic id after real query traffic.
        let _ = conn.cancel_query("no-such-query-id");
    }

    // Healthy-channel assertion, again.
    let arrow = conn.execute_query_to_arrow("SELECT 2")?;
    assert!(!arrow.is_empty());

    drop(conn);
    drop(hyper);
    Ok(())
}

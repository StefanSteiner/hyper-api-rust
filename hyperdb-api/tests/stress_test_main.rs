// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Hyper DB Monte Carlo Stress Test — test entry points.
//!
//! All tests are `#[ignore]` so they don't run during normal `cargo test`.
//! Run explicitly with:
//!
//! ```sh
//! cargo test -p hyperdb-api --test stress_test -- --ignored --nocapture
//! ```

// The stress-test harness is a throwaway simulation: randomized sizes, counts,
// and timings that intentionally sample from wide ranges and narrow for display
// or RNG bounds. Blanket-allowing the narrowing-cast lints here (with an
// explicit reason) avoids sprinkling `#[expect]` on every dice roll without
// weakening the rule in production code, where the deny-level config still
// applies.
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "stress-test harness: intentional wide→narrow conversions for RNG bounds, metric buckets, and log formatting"
)]

#[path = "stress_test/mod.rs"]
mod stress_test;

use stress_test::config::{SimDataFormat, SimTransport, SimulationConfig};
use stress_test::simulation;

use std::path::PathBuf;

/// TCP + `HyperBinary` stress test (default mode).
#[test]
#[ignore = "long-running stress test; opt-in via `cargo test -- --ignored`"]
fn stress_test_tcp_hyperbinary() {
    let mut config = SimulationConfig::from_env();
    config.transport = SimTransport::Tcp;
    config.data_format = SimDataFormat::HyperBinary;
    let summary = simulation::run(config);
    assert!(
        !summary.hyper_crashed,
        "Hyper process crashed during stress test"
    );
}

/// TCP + Arrow stream stress test.
#[test]
#[ignore = "long-running stress test; opt-in via `cargo test -- --ignored`"]
fn stress_test_tcp_arrow() {
    let mut config = SimulationConfig::from_env();
    config.transport = SimTransport::Tcp;
    config.data_format = SimDataFormat::ArrowStream;
    let summary = simulation::run(config);
    assert!(
        !summary.hyper_crashed,
        "Hyper process crashed during stress test"
    );
}

/// gRPC stress test (inserts via TCP, queries via gRPC).
#[test]
#[ignore = "long-running stress test; opt-in via `cargo test -- --ignored`"]
fn stress_test_grpc() {
    let mut config = SimulationConfig::from_env();
    config.transport = SimTransport::Grpc;
    config.data_format = SimDataFormat::HyperBinary;
    let summary = simulation::run(config);
    assert!(
        !summary.hyper_crashed,
        "Hyper process crashed during stress test"
    );
}

/// Replay a previous stress test run from a replay.json file.
///
/// Set `STRESS_REPLAY_FILE` to the path of the replay file.
#[test]
#[ignore = "long-running stress test; opt-in via `cargo test -- --ignored`"]
fn stress_test_replay() {
    let replay_path = std::env::var("STRESS_REPLAY_FILE")
        .expect("STRESS_REPLAY_FILE env var must be set for replay mode");
    let summary = simulation::run_replay(&PathBuf::from(replay_path));
    assert!(
        !summary.hyper_crashed,
        "Hyper process crashed during stress test replay"
    );
}

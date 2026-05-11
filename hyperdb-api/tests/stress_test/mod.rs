// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Hyper DB Monte Carlo Stress Test.
//!
//! A comprehensive, long-running stress test that simulates concurrent
//! multi-user workloads against a single `HyperProcess` with configurable
//! resource limits, transport modes, and stochastic operation selection.
//!
//! # Running
//!
//! ```sh
//! # Default (5 min, 3 DBs, 9 users):
//! cargo test -p hyperdb-api --test stress_test -- --ignored --nocapture
//!
//! # Custom:
//! STRESS_DURATION=3600 STRESS_DATABASES=5 STRESS_SEED=42 \
//!   cargo test -p hyperdb-api --test stress_test -- --ignored --nocapture
//!
//! # Replay a previous run:
//! STRESS_REPLAY_FILE=/tmp/hyper_stress_123/replay.json \
//!   cargo test -p hyperdb-api --test stress_test -- --ignored --nocapture stress_test_replay
//! ```

pub(crate) mod config;
pub(crate) mod replay;
pub(crate) mod resource_monitor;
pub(crate) mod schema;
pub(crate) mod simulation;
pub(crate) mod stats;
pub(crate) mod user_profiles;
pub(crate) mod workload;

// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `HyperProcess::drop` must only remove a socket directory it created itself.

#![cfg(unix)]

use hyperdb_api::{HyperProcess, Parameters, TransportMode};

/// A caller-supplied `domain_socket_directory` whose basename happens to start
/// with `hyper-` (e.g. `~/hyper-data`) is user data, not scratch space. Drop
/// used to `remove_dir_all` it purely on the name prefix.
#[test]
fn user_supplied_socket_dir_survives_drop() {
    // Keep the path short: Unix socket paths are limited to ~104 bytes.
    let dir = std::path::PathBuf::from(format!("/tmp/hyper-keep-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create user dir");
    let sentinel = dir.join("precious.txt");
    std::fs::write(&sentinel, b"do not delete").expect("write sentinel");

    let mut params = Parameters::new();
    params.set_transport_mode(TransportMode::Ipc);
    params.set_domain_socket_directory(&dir);
    {
        let hyper = HyperProcess::new(None, Some(&params)).expect("start hyperd over IPC");
        assert_eq!(hyper.socket_directory(), Some(dir.as_path()));
    }

    let survived = sentinel.exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(survived, "Drop deleted a user-supplied socket directory");
}

/// The default temp socket directory is ours and must still be cleaned up.
#[test]
fn default_socket_dir_is_removed_on_drop() {
    let mut params = Parameters::new();
    params.set_transport_mode(TransportMode::Ipc);
    let dir = {
        let hyper = HyperProcess::new(None, Some(&params)).expect("start hyperd over IPC");
        let dir = hyper
            .socket_directory()
            .expect("ipc socket dir")
            .to_path_buf();
        assert!(dir.exists());
        dir
    };
    assert!(!dir.exists(), "default socket dir should be cleaned up");
}

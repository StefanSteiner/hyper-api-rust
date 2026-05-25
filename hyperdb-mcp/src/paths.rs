// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Cross-platform resolution for the persistent-database default path.
//!
//! The persistent database lives in the platform-standard data directory:
//!
//! - **macOS:** `~/Library/Application Support/hyperdb/workspace.hyper`
//! - **Linux:** `$XDG_DATA_HOME/hyperdb/workspace.hyper`
//!   (defaults to `~/.local/share/hyperdb/workspace.hyper`)
//! - **Windows:** `%APPDATA%\hyperdb\workspace.hyper`
//!
//! Note this is intentionally distinct from `~/.hyperdb/`, which is the
//! daemon's state directory (`daemon.json`, `logs/`). Daemon coordination
//! and user data have different lifecycles, so they live in different
//! places.
//!
//! Resolution precedence:
//! 1. Explicit CLI value (`--persistent-db <PATH>` or the deprecated
//!    `--workspace <PATH>`).
//! 2. `HYPERDB_PERSISTENT_DB` environment variable.
//! 3. Platform default via [`dirs::data_dir`].

use std::path::PathBuf;

/// Application directory name used inside the platform data dir.
const APP_DIR_NAME: &str = "hyperdb";

/// Filename of the persistent workspace inside the app dir.
const PERSISTENT_DB_FILENAME: &str = "workspace.hyper";

/// Environment variable that overrides the platform-default path.
pub const ENV_PERSISTENT_DB: &str = "HYPERDB_PERSISTENT_DB";

/// Returns the platform-default path for the persistent database. Returns
/// `None` if the home / data directory cannot be determined (rare; usually
/// indicates a misconfigured environment).
#[must_use]
pub fn default_persistent_db_path() -> Option<PathBuf> {
    Some(
        dirs::data_dir()?
            .join(APP_DIR_NAME)
            .join(PERSISTENT_DB_FILENAME),
    )
}

/// Resolve where the persistent database should live, applying the
/// CLI > env-var > platform-default precedence. Returns `None` only when
/// no source supplied a path (the platform default failed *and* nothing
/// was set explicitly), which the caller should treat as an error.
///
/// `cli_value` is the value of `--persistent-db` (or `--workspace` after
/// deprecation translation). When `Some`, takes precedence over both
/// the env var and the platform default.
#[must_use]
pub fn resolve_persistent_db_path(cli_value: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = cli_value {
        return Some(PathBuf::from(p));
    }
    if let Some(p) = std::env::var_os(ENV_PERSISTENT_DB) {
        return Some(PathBuf::from(p));
    }
    default_persistent_db_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Process-wide lock for env-var tests. `std::env::set_var` is
    /// `unsafe` in newer toolchains because it's not thread-safe; we
    /// serialize all env-touching tests to keep them sound.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_env_lock<R>(f: impl FnOnce() -> R) -> R {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f()
    }

    /// Sets an env var. Marked `unsafe` because [`std::env::set_var`] is
    /// `unsafe` in newer toolchains; callers hold `ENV_LOCK`.
    unsafe fn set_env(key: &str, value: &str) {
        // SAFETY: serialized by ENV_LOCK; matches std::env contract.
        unsafe { std::env::set_var(key, value) }
    }

    /// Removes an env var. Marked `unsafe` for the same reason as
    /// [`set_env`]; callers hold `ENV_LOCK`.
    unsafe fn remove_env(key: &str) {
        // SAFETY: serialized by ENV_LOCK; matches std::env contract.
        unsafe { std::env::remove_var(key) }
    }

    #[test]
    fn default_persistent_db_path_returns_some_on_supported_platforms() {
        // On macOS, Linux, and Windows the platform helpers always
        // resolve to a usable path. CI runs on these three; if this
        // fails on a new platform we want a loud signal.
        let p = default_persistent_db_path().expect("platform data_dir resolves");
        assert!(p.ends_with("hyperdb/workspace.hyper") || p.ends_with("hyperdb\\workspace.hyper"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn default_persistent_db_path_uses_app_support_on_macos() {
        let p = default_persistent_db_path().unwrap();
        let s = p.to_string_lossy();
        assert!(
            s.contains("Library/Application Support/hyperdb/"),
            "expected macOS Application Support path, got {s}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn default_persistent_db_path_uses_xdg_share_on_linux() {
        let p = default_persistent_db_path().unwrap();
        let s = p.to_string_lossy();
        assert!(
            s.contains(".local/share/hyperdb/") || s.contains("share/hyperdb/"),
            "expected XDG share path, got {s}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn default_persistent_db_path_uses_appdata_on_windows() {
        let p = default_persistent_db_path().unwrap();
        let s = p.to_string_lossy();
        assert!(
            s.contains("hyperdb"),
            "expected APPDATA path containing hyperdb, got {s}"
        );
    }

    #[test]
    fn resolve_persistent_db_path_cli_takes_precedence() {
        with_env_lock(|| {
            // SAFETY: serialized by ENV_LOCK.
            unsafe { set_env(ENV_PERSISTENT_DB, "/from/env.hyper") };
            let p = resolve_persistent_db_path(Some("/from/cli.hyper"))
                .expect("CLI path always resolves");
            assert_eq!(p, PathBuf::from("/from/cli.hyper"));
            // SAFETY: serialized by ENV_LOCK.
            unsafe { remove_env(ENV_PERSISTENT_DB) };
        });
    }

    #[test]
    fn resolve_persistent_db_path_env_used_when_no_cli() {
        with_env_lock(|| {
            // SAFETY: serialized by ENV_LOCK.
            unsafe { set_env(ENV_PERSISTENT_DB, "/from/env.hyper") };
            let p = resolve_persistent_db_path(None).expect("env path resolves");
            assert_eq!(p, PathBuf::from("/from/env.hyper"));
            // SAFETY: serialized by ENV_LOCK.
            unsafe { remove_env(ENV_PERSISTENT_DB) };
        });
    }

    #[test]
    fn resolve_persistent_db_path_falls_back_to_default() {
        with_env_lock(|| {
            // SAFETY: serialized by ENV_LOCK.
            unsafe { remove_env(ENV_PERSISTENT_DB) };
            let p = resolve_persistent_db_path(None).expect("default resolves");
            // Just check it's under hyperdb/ — exact location varies by
            // platform and is covered by the default-path tests above.
            assert!(p.to_string_lossy().contains("hyperdb"));
        });
    }
}

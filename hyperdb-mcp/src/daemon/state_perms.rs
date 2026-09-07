// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Owner-only permissions for the daemon state directory and the files in it.
//!
//! The state directory (`~/.hyperdb`, or `HYPERDB_STATE_DIR`) holds the
//! daemon's connection details: `daemon.json` names the `hyperd` endpoint, and
//! `logs/` records it too. Those are the owning user's business, so this module
//! is the single place that decides how they are created.
//!
//! Created with [`std::fs::create_dir_all`] and [`std::fs::write`], both would
//! take their mode from the process umask instead — commonly `0755` and `0644`.
//! The helpers here pin the directory to `0700` and files to `0600`.
//!
//! Restricting the *directory* is what makes `logs/` safe: `hyperd` is a
//! separate process writing its own log files under its own umask, so their
//! individual modes are not ours to set. A `0700` directory settles it for
//! every file inside, whoever wrote it.
//!
//! # Platform behaviour
//!
//! Unix modes have no Windows equivalent, so the mode calls compile out there.
//! Windows leans on ACL inheritance instead: the default state directory sits
//! under `%USERPROFILE%`, whose ACL grants the owning user, `SYSTEM`, and
//! administrators — and *not* other interactive users — and a new subdirectory
//! inherits it. Pointing `HYPERDB_STATE_DIR` outside the profile forfeits that
//! inheritance, which is why the env var is documented as the user's own call.

use std::io;
use std::path::Path;

/// Directory mode for state directories: owner-only, including traversal.
#[cfg(unix)]
const STATE_DIR_MODE: u32 = 0o700;

/// File mode for state files: readable and writable only by the owner.
#[cfg(unix)]
const STATE_FILE_MODE: u32 = 0o600;

/// Create `dir` and any missing parents, restricted to the owning user.
///
/// A directory this call creates is restricted from the start. One that already
/// exists — from a release that created it under the process umask — is
/// tightened in place.
///
/// # Errors
/// Returns an error if the directory cannot be created. Failing to *tighten* an
/// existing directory is logged as a warning and tolerated, because `chmod` can
/// be refused for reasons unrelated to this directory's contents — a state
/// directory on a filesystem with no Unix modes, say — and refusing to start
/// there would be a worse outcome than the permissions it was trying to fix.
pub fn ensure_owner_only_dir(dir: &Path) -> io::Result<()> {
    create_owner_only_dir(dir)?;
    restrict_existing_dir(dir);
    Ok(())
}

#[cfg(unix)]
fn create_owner_only_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(STATE_DIR_MODE)
        .create(dir)
}

#[cfg(not(unix))]
fn create_owner_only_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Tighten an existing directory to owner-only access, best effort.
///
/// Tolerating failure here is deliberate. `chmod` can fail for reasons that
/// have nothing to do with this directory's contents: `HYPERDB_STATE_DIR` may
/// name a path on a filesystem with no Unix modes at all (an SMB or NFS mount,
/// exFAT, some container bind-mounts), where the call is refused however the
/// directory is actually protected. Refusing to start would trade a
/// confidentiality gap the user already had for a total loss of the MCP on
/// those setups — a strictly worse outcome for a local developer tool.
///
/// The file we are about to *publish* an endpoint into is the opposite case, so
/// [`write_owner_only_atomic`] fails loudly instead.
fn restrict_existing_dir(dir: &Path) {
    #[cfg(unix)]
    {
        if let Err(error) = restrict_existing_unix(dir, STATE_DIR_MODE) {
            tracing::warn!(
                path = %dir.display(),
                %error,
                "could not restrict the daemon state directory to the current user; \
                 its existing permissions are left in place"
            );
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
}

/// Reset an existing path's permission bits to `mode`.
///
/// Skips the `chmod` when the mode already matches, so the steady state costs
/// one `stat`. `permissions().mode()` carries the file-type bits too, hence the
/// mask before comparing.
#[cfg(unix)]
fn restrict_existing_unix(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let metadata = std::fs::metadata(path)?;
    if metadata.permissions().mode() & 0o777 == mode {
        return Ok(());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// Write `contents` to `path` atomically, restricted to the owning user.
///
/// `tmp_path` receives the content and is then renamed onto `path`, preserving
/// the existing guarantee that a concurrent reader sees either the old record
/// or the new one, never a partial write. The mode is set on `tmp_path` *before
/// any content is written*, so the endpoint is never on disk in a
/// world-readable file — not even briefly.
///
/// Because `rename` replaces the target's inode rather than its contents, the
/// published file carries this mode even if `path` already existed with looser
/// permissions. That is what corrects a `daemon.json` left behind by an earlier
/// release.
///
/// # Errors
/// Returns an error if the temp file cannot be created with the intended mode,
/// cannot be written, or cannot be renamed onto `path`. Unlike a directory
/// that merely already exists, this file is one this process is about to
/// publish an endpoint into, so a record whose permissions could not be set is
/// not published at all.
pub(crate) fn write_owner_only_atomic(
    path: &Path,
    tmp_path: &Path,
    contents: &[u8],
) -> io::Result<()> {
    use std::io::Write as _;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(STATE_FILE_MODE);
    }

    let mut file = options.open(tmp_path)?;

    // `OpenOptions::mode` applies only to a file this call creates. An
    // interrupted earlier write can leave `tmp_path` behind, and reopening
    // that file keeps whatever mode it already had, so restrict the open
    // handle too. Operating on the descriptor rather than the path also means
    // no window in which the name could be swapped for another file.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(STATE_FILE_MODE))?;
    }

    file.write_all(contents)?;
    // Close before renaming: Windows refuses to rename a file that is still
    // open for writing.
    drop(file);

    std::fs::rename(tmp_path, path)
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use tempfile::TempDir;

    use super::*;

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path)
            .expect("fixture path should exist")
            .permissions()
            .mode()
            & 0o777
    }

    #[test]
    fn state_directory_is_created_owner_only() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("state");

        ensure_owner_only_dir(&dir).unwrap();

        assert_eq!(
            mode_of(&dir),
            0o700,
            "a freshly created state directory must not be reachable by other users"
        );
    }

    #[test]
    fn nested_state_directory_is_created_owner_only_at_every_level() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("state");
        let nested = root.join("logs");

        ensure_owner_only_dir(&nested).unwrap();

        assert_eq!(
            mode_of(&nested),
            0o700,
            "the leaf state directory must be owner-only"
        );
        assert_eq!(
            mode_of(&root),
            0o700,
            "an intermediate state directory must be owner-only too, or the leaf is still \
             reachable through it"
        );
    }

    #[test]
    fn a_pre_existing_world_readable_state_directory_is_tightened() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("state");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            mode_of(&dir),
            0o755,
            "fixture must start world-readable or it does not exercise the correction"
        );

        ensure_owner_only_dir(&dir).unwrap();

        assert_eq!(
            mode_of(&dir),
            0o700,
            "a state directory left loose by an earlier run must be corrected, not accepted"
        );
    }

    /// The shape an upgrade actually meets: both the state directory and the
    /// `logs/` directory inside it already exist, world-readable, from a
    /// release that created them under the process umask.
    #[test]
    fn a_pre_existing_world_readable_directory_tree_is_tightened_at_every_level() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("state");
        let nested = root.join("logs");
        std::fs::create_dir_all(&nested).unwrap();
        for dir in [&root, &nested] {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(
                mode_of(dir),
                0o755,
                "fixture must start world-readable or it does not exercise the correction"
            );
        }

        // Mirrors the daemon startup order: the state directory, then the log
        // directory inside it.
        ensure_owner_only_dir(&root).unwrap();
        ensure_owner_only_dir(&nested).unwrap();

        assert_eq!(
            mode_of(&root),
            0o700,
            "an existing state directory must be tightened on the next daemon start"
        );
        assert_eq!(
            mode_of(&nested),
            0o700,
            "an existing log directory must be tightened too — it holds hyperd's own logs, \
             which name the endpoint"
        );
    }

    #[test]
    fn state_file_is_written_owner_only() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("daemon.json");
        let tmp_path = tmp.path().join("daemon.json.tmp");

        write_owner_only_atomic(&path, &tmp_path, b"{}").unwrap();

        assert_eq!(
            mode_of(&path),
            0o600,
            "a state file naming the endpoint must be readable only by its owner"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        assert!(
            !tmp_path.exists(),
            "the temp file should have been renamed onto the target"
        );
    }

    #[test]
    fn replacing_a_world_readable_state_file_tightens_it() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("daemon.json");
        let tmp_path = tmp.path().join("daemon.json.tmp");
        std::fs::write(&path, b"stale").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            mode_of(&path),
            0o644,
            "fixture must start world-readable or it does not exercise the correction"
        );

        write_owner_only_atomic(&path, &tmp_path, b"{}").unwrap();

        assert_eq!(
            mode_of(&path),
            0o600,
            "rewriting a state file left loose by an earlier release must tighten it"
        );
    }

    #[test]
    fn a_world_readable_leftover_temp_file_does_not_leak_the_new_record() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("daemon.json");
        let tmp_path = tmp.path().join("daemon.json.tmp");
        // An interrupted earlier write leaves the temp file behind. Reopening
        // it does not re-apply `OpenOptions::mode`, so without an explicit
        // restriction the new record would land in a world-readable file.
        std::fs::write(&tmp_path, b"interrupted").unwrap();
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_owner_only_atomic(&path, &tmp_path, b"{}").unwrap();

        assert_eq!(
            mode_of(&path),
            0o600,
            "a leftover temp file must not carry its loose mode into the published record"
        );
    }
}

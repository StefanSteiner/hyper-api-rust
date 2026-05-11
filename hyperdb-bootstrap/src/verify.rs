// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! HEAD each supported platform's download URL to confirm the pinned
//! release is still reachable on Tableau's CDN. Used by the `verify`
//! CLI subcommand and by CI workflows that guard against silent yanks
//! or URL-scheme changes.

use std::process::Command;

use crate::platform::Platform;
use crate::release::PinnedRelease;
use crate::url::build_download_url;
use crate::Error;

const PLATFORMS: &[Platform] = &[
    Platform::MacosArm64,
    Platform::MacosX86_64,
    Platform::LinuxX86_64,
    Platform::WindowsX86_64,
];

/// Result of a single platform reachability probe performed by
/// [`verify_release`].
#[derive(Debug, Clone)]
pub struct VerifyOutcome {
    /// Platform the probe targeted.
    pub platform: Platform,
    /// URL that was probed.
    pub url: String,
    /// HTTP status returned by the CDN, or `None` if the probe itself failed.
    pub status: Option<u16>,
    /// Error message when `status` is `None` (spawn failure, parse failure,
    /// stderr from a failed `curl` invocation).
    pub error: Option<String>,
}

impl VerifyOutcome {
    /// Returns `true` when the probe observed a 2xx/3xx HTTP status, which
    /// is what `curl --head --location` returns when the CDN serves the
    /// release.
    #[must_use]
    pub fn ok(&self) -> bool {
        matches!(self.status, Some(s) if (200..400).contains(&s))
    }
}

/// HEAD every supported platform URL for `release`. Returns one outcome
/// per platform; callers decide how to surface failures.
///
/// Uses `curl --head` rather than reqwest so that Akamai's bot-protection
/// layer (which blocks reqwest's TLS fingerprint from GitHub-hosted runner
/// IPs) does not cause false 403 failures.
///
/// # Errors
///
/// Currently always returns `Ok(_)` — individual platform failures are
/// surfaced through [`VerifyOutcome::error`]. The `Result` wrapper is
/// kept for forward compatibility if a top-level failure mode is added.
pub fn verify_release(release: &PinnedRelease) -> Result<Vec<VerifyOutcome>, Error> {
    let mut out = Vec::with_capacity(PLATFORMS.len());
    for &platform in PLATFORMS {
        let url = build_download_url(release, platform);
        let outcome = curl_head(&url);
        out.push(VerifyOutcome {
            platform,
            url,
            status: outcome.0,
            error: outcome.1,
        });
    }
    Ok(out)
}

/// Run `curl --head --silent --show-error --location <url>` and parse the
/// HTTP status from the first status line. Returns `(Some(status), None)`
/// on success and `(None, Some(error))` on spawn/parse failure.
fn curl_head(url: &str) -> (Option<u16>, Option<String>) {
    let result = Command::new("curl")
        .args(["--head", "--silent", "--show-error", "--location"])
        .arg(url)
        .output();

    match result {
        Err(e) => (None, Some(format!("failed to spawn curl: {e}"))),
        Ok(output) => {
            if !output.status.success() && output.stdout.is_empty() {
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                return (None, Some(stderr));
            }
            // Parse the last "HTTP/x.x NNN" status line (curl --location may
            // produce multiple status lines when following redirects).
            let stdout = String::from_utf8_lossy(&output.stdout);
            let status = stdout
                .lines()
                .filter_map(|line| {
                    let line = line.trim();
                    // Matches "HTTP/1.1 200 OK", "HTTP/2 403", etc.
                    if line.starts_with("HTTP/") {
                        line.split_whitespace().nth(1)?.parse::<u16>().ok()
                    } else {
                        None
                    }
                })
                .next_back();
            match status {
                Some(s) => (Some(s), None),
                None => (
                    None,
                    Some(format!(
                        "could not parse HTTP status from curl output: {}",
                        stdout.chars().take(200).collect::<String>()
                    )),
                ),
            }
        }
    }
}

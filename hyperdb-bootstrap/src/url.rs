// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Builds canonical download URLs for Tableau's public Hyper C++ API
//! release bundles.

use crate::platform::Platform;
use crate::release::PinnedRelease;

const BASE_URL: &str = "https://downloads.tableau.com/tssoftware";

/// Builds the `downloads.tableau.com` URL for the given release / platform
/// combination.
///
/// The URL template matches
/// `https://downloads.tableau.com/tssoftware/tableauhyperapi-cxx-<platform>-release-main.<version>.<build_id>.zip`.
#[must_use]
pub fn build_download_url(release: &PinnedRelease, platform: Platform) -> String {
    format!(
        "{base}/tableauhyperapi-cxx-{plat}-release-main.{version}.{build_id}.zip",
        base = BASE_URL,
        plat = platform.slug(),
        version = release.version,
        build_id = release.build_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn url_matches_expected_template() {
        let r = PinnedRelease {
            version: "0.0.24457".to_string(),
            build_id: "rc36858b6".to_string(),
            sha256: HashMap::new(),
        };
        let url = build_download_url(&r, Platform::MacosArm64);
        assert_eq!(
            url,
            "https://downloads.tableau.com/tssoftware/tableauhyperapi-cxx-macos-arm64-release-main.0.0.24457.rc36858b6.zip"
        );
    }
}

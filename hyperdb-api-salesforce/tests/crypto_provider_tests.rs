// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Guards the `rustls-no-provider` feature choice on the `reqwest` dependency.
//!
//! That feature links no crypto provider, and `reqwest` resolves one through
//! `CryptoProvider::get_default()`, which has no crate-feature fallback. If
//! nothing installs a provider first, `reqwest` *panics* while building its
//! `Client` — so this needs no network and no credentials to fail. It only
//! has to reach the point where a client is constructed.
//!
//! Without the `ensure_crypto_provider` call in `DataCloudTokenProvider::new`,
//! this test aborts the thread instead of returning.

use hyperdb_api_salesforce::{AuthMode, DataCloudTokenProvider, SalesforceAuthConfig};
use zeroize::Zeroizing;

#[test]
fn provider_construction_installs_a_crypto_provider() {
    let config = SalesforceAuthConfig::new("https://login.salesforce.com", "test-client-id")
        .expect("a well-formed https login URL is valid")
        .auth_mode(AuthMode::Password {
            username: "nobody@example.com".to_owned(),
            password: Zeroizing::new("not-a-real-password".to_owned()),
        })
        .client_secret("not-a-real-secret");

    // Builds the HTTP client. Reaching `Ok` at all is the assertion; no
    // request is issued, so the dummy credentials are never used.
    let provider = DataCloudTokenProvider::new(config);
    assert!(
        provider.is_ok(),
        "provider construction failed: {:?}",
        provider.err()
    );
}

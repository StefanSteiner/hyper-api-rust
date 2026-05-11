// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! TLS configuration and connection handling.
//!
//! This module provides TLS support for secure connections to Hyper servers
//! using a pure Rust TLS implementation (rustls).
//!
//! # Certificate Formats
//!
//! | Item | Format |
//! |------|--------|
//! | CA Cert | PEM |
//! | Client Cert | PEM |
//! | Client Key | PEM (PKCS#8 or PKCS#1) |
//!
//! - **PEM format**: Base64-encoded with `-----BEGIN/END CERTIFICATE-----` headers
//! - **PKCS#8**: Key format with `-----BEGIN PRIVATE KEY-----` header
//! - **PKCS#1**: RSA-specific format with `-----BEGIN RSA PRIVATE KEY-----` header
//!
//! # Example
//!
//! ```ignore
//! use hyperdb_api_core::client::{Client, Config};
//! use hyperdb_api_core::client::tls::TlsConfig;
//!
//! # fn example() -> hyperdb_api_core::client::Result<()> {
//! let config = Config::new()
//!     .with_host("secure-hyper.example.com")
//!     .with_port(7484)
//!     .with_tls(TlsConfig::default()); // TODO: with_tls not yet on Config
//!
//! let client = Client::connect(&config)?;
//! # Ok(())
//! # }
//! ```

use std::path::PathBuf;

/// TLS configuration options.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// Whether to verify the server certificate.
    pub verify_server: bool,
    /// Path to CA certificate file (PEM format).
    pub ca_cert_path: Option<PathBuf>,
    /// Path to client certificate file (PEM format).
    pub client_cert_path: Option<PathBuf>,
    /// Path to client key file (PEM format).
    pub client_key_path: Option<PathBuf>,
    /// Server name for SNI (Server Name Indication).
    pub server_name: Option<String>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        TlsConfig {
            verify_server: true,
            ca_cert_path: None,
            client_cert_path: None,
            client_key_path: None,
            server_name: None,
        }
    }
}

impl TlsConfig {
    /// Creates a new TLS configuration with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    /// Disables server certificate verification.
    ///
    /// # Security Warning
    ///
    /// **This is a serious security risk in production environments.**
    /// Disabling certificate verification allows man-in-the-middle attacks.
    ///
    /// Only use this for:
    /// - Local development with self-signed certificates
    /// - Testing environments
    /// - Debugging certificate issues (temporarily)
    ///
    /// A warning will be logged at runtime when this is enabled.
    pub fn danger_accept_invalid_certs(mut self) -> Self {
        tracing::warn!(
            "TLS certificate verification disabled - this should only be used for testing. \
             Man-in-the-middle attacks are possible."
        );

        self.verify_server = false;
        self
    }

    #[must_use]
    /// Sets the CA certificate file for verifying the server.
    pub fn ca_cert(mut self, path: impl Into<PathBuf>) -> Self {
        self.ca_cert_path = Some(path.into());
        self
    }

    #[must_use]
    /// Sets the client certificate for mutual TLS.
    pub fn client_cert(
        mut self,
        cert_path: impl Into<PathBuf>,
        key_path: impl Into<PathBuf>,
    ) -> Self {
        self.client_cert_path = Some(cert_path.into());
        self.client_key_path = Some(key_path.into());
        self
    }

    #[must_use]
    /// Sets the server name for SNI.
    pub fn server_name(mut self, name: impl Into<String>) -> Self {
        self.server_name = Some(name.into());
        self
    }

    /// Returns true if client certificates are configured.
    #[must_use]
    pub fn has_client_cert(&self) -> bool {
        self.client_cert_path.is_some() && self.client_key_path.is_some()
    }
}

/// TLS mode for the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TlsMode {
    /// No TLS (plain TCP).
    #[default]
    Disable,
    /// Prefer TLS if available, fall back to plain TCP.
    Prefer,
    /// Require TLS, fail if not available.
    Require,
    /// Require TLS and verify the server certificate.
    VerifyCA,
    /// Require TLS, verify server certificate, and verify hostname.
    VerifyFull,
}

impl TlsMode {
    /// Returns true if TLS is enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        !matches!(self, TlsMode::Disable)
    }

    /// Returns true if TLS is required (not just preferred).
    #[must_use]
    pub fn is_required(&self) -> bool {
        matches!(
            self,
            TlsMode::Require | TlsMode::VerifyCA | TlsMode::VerifyFull
        )
    }

    /// Returns true if server certificate verification is required.
    #[must_use]
    pub fn verify_server(&self) -> bool {
        matches!(self, TlsMode::VerifyCA | TlsMode::VerifyFull)
    }

    /// Returns true if hostname verification is required.
    #[must_use]
    pub fn verify_hostname(&self) -> bool {
        matches!(self, TlsMode::VerifyFull)
    }
}

/// TLS implementation using the `rustls` crate.
pub mod rustls_impl {
    use super::TlsConfig;
    use std::io::BufReader;
    use std::sync::Arc;

    use tokio::net::TcpStream;
    use tokio_rustls::rustls::{ClientConfig, RootCertStore};
    use tokio_rustls::TlsConnector;

    use crate::client::error::{Error, ErrorKind, Result};

    /// Creates a TLS connector from the configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Config`] when:
    /// - The CA cert path is set but cannot be opened or the PEM bytes
    ///   cannot be parsed / added to the root store.
    /// - The client cert / key path is set but cannot be opened, the
    ///   PEM payload is invalid, the key section is missing, or
    ///   `rustls` rejects the cert/key pair.
    /// - Rustls cannot build a `ClientConfig` with the configured
    ///   protocol versions.
    ///
    /// # Panics
    ///
    /// Does not panic in practice. The `client_cert_path` and
    /// `client_key_path` `.unwrap()` calls are guarded by the
    /// preceding [`TlsConfig::has_client_cert`] check, which only
    /// returns `true` when both paths are `Some`.
    pub fn create_connector(config: &TlsConfig, _host: &str) -> Result<TlsConnector> {
        let mut root_store = RootCertStore::empty();

        // Add system root certificates
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        // Add custom CA certificate if provided
        if let Some(ref ca_path) = config.ca_cert_path {
            let ca_file = std::fs::File::open(ca_path).map_err(|e| {
                Error::new(ErrorKind::Config, format!("failed to open CA cert: {e}"))
            })?;
            let mut ca_reader = BufReader::new(ca_file);
            let certs = rustls_pemfile::certs(&mut ca_reader)
                .map(|r| {
                    r.map_err(|e| Error::new(ErrorKind::Config, format!("invalid CA cert: {e}")))
                })
                .collect::<Result<Vec<_>>>()?;
            for cert in certs {
                root_store.add(cert).map_err(|e| {
                    Error::new(ErrorKind::Config, format!("failed to add CA cert: {e}"))
                })?;
            }
        }

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| Error::new(ErrorKind::Config, format!("TLS protocol config error: {e}")))?
            .with_root_certificates(root_store);

        let client_config = if config.has_client_cert() {
            // Load client certificate and key
            let cert_path = config.client_cert_path.as_ref().unwrap();
            let key_path = config.client_key_path.as_ref().unwrap();

            let cert_file = std::fs::File::open(cert_path).map_err(|e| {
                Error::new(
                    ErrorKind::Config,
                    format!("failed to open client cert: {e}"),
                )
            })?;
            let mut cert_reader = BufReader::new(cert_file);
            let certs = rustls_pemfile::certs(&mut cert_reader)
                .map(|r| {
                    r.map_err(|e| {
                        Error::new(ErrorKind::Config, format!("invalid client cert: {e}"))
                    })
                })
                .collect::<Result<Vec<_>>>()?;

            let key_file = std::fs::File::open(key_path).map_err(|e| {
                Error::new(ErrorKind::Config, format!("failed to open client key: {e}"))
            })?;
            let mut key_reader = BufReader::new(key_file);
            let key = rustls_pemfile::private_key(&mut key_reader)
                .map_err(|e| Error::new(ErrorKind::Config, format!("invalid client key: {e}")))?
                .ok_or_else(|| Error::new(ErrorKind::Config, "no private key found"))?;

            builder
                .with_client_auth_cert(certs, key)
                .map_err(|e| Error::new(ErrorKind::Config, format!("invalid client auth: {e}")))?
        } else {
            builder.with_no_client_auth()
        };

        Ok(TlsConnector::from(Arc::new(client_config)))
    }

    /// Type alias for a TLS-wrapped TCP stream.
    pub type TlsStream = tokio_rustls::client::TlsStream<TcpStream>;

    /// Wraps a TCP stream with TLS.
    ///
    /// # Errors
    ///
    /// - Returns [`ErrorKind::Config`] if `server_name` is not a
    ///   valid DNS name or IP literal accepted by `rustls`.
    /// - Returns [`ErrorKind::Connection`] if the TLS handshake with
    ///   the peer fails (certificate rejected, protocol error, I/O
    ///   failure).
    pub async fn wrap_stream(
        stream: TcpStream,
        connector: &TlsConnector,
        server_name: &str,
    ) -> Result<TlsStream> {
        let domain = rustls::pki_types::ServerName::try_from(server_name.to_string())
            .map_err(|_| Error::new(ErrorKind::Config, "invalid server name"))?;

        connector
            .connect(domain, stream)
            .await
            .map_err(|e| Error::new(ErrorKind::Connection, format!("TLS handshake failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tls_config_default() {
        let config = TlsConfig::default();
        assert!(config.verify_server);
        assert!(config.ca_cert_path.is_none());
        assert!(!config.has_client_cert());
    }

    #[test]
    fn test_tls_config_builder() {
        let config = TlsConfig::new()
            .ca_cert("/path/to/ca.pem")
            .client_cert("/path/to/cert.pem", "/path/to/key.pem")
            .server_name("example.com");

        assert!(config.has_client_cert());
        assert_eq!(config.server_name, Some("example.com".to_string()));
    }

    #[test]
    fn test_tls_mode() {
        assert!(!TlsMode::Disable.is_enabled());
        assert!(TlsMode::Require.is_required());
        assert!(TlsMode::VerifyFull.verify_hostname());
    }
}

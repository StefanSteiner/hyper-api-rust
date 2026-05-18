// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for TLS support.
//!
//! These tests verify the rustls backend by:
//! 1. Generating self-signed certificates at test time using `rcgen`
//! 2. Starting a local TLS echo server
//! 3. Connecting through `hyperdb_api_core::client::tls` module functions
//! 4. Verifying data can be exchanged over the encrypted connection
//!
//! Run with:
//!   cargo test --test `tls_tests`

use std::io::Write;
use tempfile::NamedTempFile;

use hyperdb_api_core::client::tls::TlsConfig;

/// A CA: its params (needed as issuer reference), key pair, and self-signed cert PEM.
struct TestCa {
    params: rcgen::CertificateParams,
    key_pair: rcgen::KeyPair,
    cert_pem: String,
}

/// A leaf cert: its key pair and cert PEM.
struct TestCert {
    key_pair: rcgen::KeyPair,
    cert_pem: String,
}

/// Generate a self-signed CA.
fn generate_ca() -> TestCa {
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Test CA");
    let key_pair = rcgen::KeyPair::generate().unwrap();
    let cert = params.self_signed(&key_pair).unwrap();
    let cert_pem = cert.pem();
    TestCa {
        params,
        key_pair,
        cert_pem,
    }
}

/// Generate a server certificate signed by the CA.
fn generate_server_cert(ca: &TestCa, san: &str) -> TestCert {
    let params = rcgen::CertificateParams::new(vec![san.to_string()]).unwrap();
    let key_pair = rcgen::KeyPair::generate().unwrap();
    let issuer = rcgen::Issuer::from_params(&ca.params, &ca.key_pair);
    let cert = params.signed_by(&key_pair, &issuer).unwrap();
    TestCert {
        key_pair,
        cert_pem: cert.pem(),
    }
}

/// Generate a client certificate signed by the CA.
fn generate_client_cert(ca: &TestCa) -> TestCert {
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Test Client");
    let key_pair = rcgen::KeyPair::generate().unwrap();
    let issuer = rcgen::Issuer::from_params(&ca.params, &ca.key_pair);
    let cert = params.signed_by(&key_pair, &issuer).unwrap();
    TestCert {
        key_pair,
        cert_pem: cert.pem(),
    }
}

/// Write PEM data to a temp file. The returned handle keeps the file alive.
fn write_pem_file(pem: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(pem.as_bytes()).unwrap();
    f.flush().unwrap();
    f
}

// =============================================================================
// rustls tests
// =============================================================================

mod rustls_tests {
    use super::*;
    use hyperdb_api_core::client::tls::rustls_impl;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio_rustls::TlsAcceptor;

    /// Start a TLS echo server using rustls. Returns (addr, `join_handle`).
    /// The server accepts one connection, reads data, and echoes it back prefixed with "echo:".
    async fn start_echo_server(
        server_cert_pem: &str,
        server_key_pem: &str,
        require_client_cert_ca_pem: Option<&str>,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let provider = Arc::new(rustls::crypto::ring::default_provider());

        let certs: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(server_cert_pem.as_bytes())
                .map(|r| r.unwrap())
                .collect();
        let key = PrivateKeyDer::from_pem_slice(server_key_pem.as_bytes()).unwrap();

        let server_config = if let Some(ca_pem) = require_client_cert_ca_pem {
            let mut root_store = rustls::RootCertStore::empty();
            let ca_certs: Vec<CertificateDer<'static>> =
                CertificateDer::pem_slice_iter(ca_pem.as_bytes())
                    .map(|r| r.unwrap())
                    .collect();
            for cert in ca_certs {
                root_store.add(cert).unwrap();
            }
            let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
                Arc::new(root_store),
                Arc::clone(&provider),
            )
            .build()
            .unwrap();
            rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_client_cert_verifier(verifier)
                .with_single_cert(certs, key)
                .unwrap()
        } else {
            rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .unwrap()
        };

        let acceptor = TlsAcceptor::from(Arc::new(server_config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                if let Ok(mut tls_stream) = acceptor.accept(stream).await {
                    let mut buf = [0u8; 1024];
                    if let Ok(n) = tls_stream.read(&mut buf).await {
                        let msg = std::str::from_utf8(&buf[..n]).unwrap_or("?");
                        let response = format!("echo:{msg}");
                        let _ = tls_stream.write_all(response.as_bytes()).await;
                        let _ = tls_stream.shutdown().await;
                    }
                }
            }
        });

        (addr, handle)
    }

    #[tokio::test]
    async fn test_rustls_connector_creation_with_ca() {
        let ca = generate_ca();
        let ca_pem_file = write_pem_file(&ca.cert_pem);

        let config = TlsConfig::new().ca_cert(ca_pem_file.path());
        let connector = rustls_impl::create_connector(&config, "localhost");
        assert!(
            connector.is_ok(),
            "Failed to create rustls connector: {:?}",
            connector.err()
        );
    }

    #[tokio::test]
    async fn test_rustls_connector_creation_system_roots() {
        let config = TlsConfig::new();
        let connector = rustls_impl::create_connector(&config, "localhost");
        assert!(
            connector.is_ok(),
            "Failed to create connector with system roots: {:?}",
            connector.err()
        );
    }

    #[tokio::test]
    async fn test_rustls_tls_handshake_and_data_exchange() {
        let ca = generate_ca();
        let server = generate_server_cert(&ca, "localhost");

        let (addr, server_handle) =
            start_echo_server(&server.cert_pem, &server.key_pair.serialize_pem(), None).await;

        let ca_pem_file = write_pem_file(&ca.cert_pem);
        let config = TlsConfig::new().ca_cert(ca_pem_file.path());
        let connector = rustls_impl::create_connector(&config, "localhost").unwrap();

        let tcp_stream = TcpStream::connect(addr).await.unwrap();
        let mut tls_stream = rustls_impl::wrap_stream(tcp_stream, &connector, "localhost")
            .await
            .expect("TLS handshake should succeed");

        tls_stream.write_all(b"hello TLS").await.unwrap();
        tls_stream.shutdown().await.unwrap();

        let mut response = String::new();
        tls_stream.read_to_string(&mut response).await.unwrap();
        assert_eq!(response, "echo:hello TLS");

        server_handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_rustls_rejects_unknown_ca() {
        let ca = generate_ca();
        let server = generate_server_cert(&ca, "localhost");

        let (addr, _server_handle) =
            start_echo_server(&server.cert_pem, &server.key_pair.serialize_pem(), None).await;

        // Use a different CA that didn't sign the server cert
        let wrong_ca = generate_ca();
        let wrong_ca_file = write_pem_file(&wrong_ca.cert_pem);
        let config = TlsConfig::new().ca_cert(wrong_ca_file.path());
        let connector = rustls_impl::create_connector(&config, "localhost").unwrap();

        let tcp_stream = TcpStream::connect(addr).await.unwrap();
        let result = rustls_impl::wrap_stream(tcp_stream, &connector, "localhost").await;
        assert!(
            result.is_err(),
            "Should reject connection signed by unknown CA"
        );
    }

    #[tokio::test]
    async fn test_rustls_rejects_wrong_hostname() {
        let ca = generate_ca();
        let server = generate_server_cert(&ca, "localhost");

        let (addr, _server_handle) =
            start_echo_server(&server.cert_pem, &server.key_pair.serialize_pem(), None).await;

        let ca_pem_file = write_pem_file(&ca.cert_pem);
        let config = TlsConfig::new().ca_cert(ca_pem_file.path());
        let connector = rustls_impl::create_connector(&config, "wrong.host.name").unwrap();

        let tcp_stream = TcpStream::connect(addr).await.unwrap();
        let result = rustls_impl::wrap_stream(tcp_stream, &connector, "wrong.host.name").await;
        assert!(
            result.is_err(),
            "Should reject connection when server name doesn't match cert SAN"
        );
    }

    #[tokio::test]
    async fn test_rustls_mutual_tls() {
        let ca = generate_ca();
        let server = generate_server_cert(&ca, "localhost");
        let client = generate_client_cert(&ca);

        // Server requires client certificates (mTLS)
        let (addr, server_handle) = start_echo_server(
            &server.cert_pem,
            &server.key_pair.serialize_pem(),
            Some(&ca.cert_pem),
        )
        .await;

        let ca_pem_file = write_pem_file(&ca.cert_pem);
        let client_cert_file = write_pem_file(&client.cert_pem);
        let client_key_file = write_pem_file(&client.key_pair.serialize_pem());

        let config = TlsConfig::new()
            .ca_cert(ca_pem_file.path())
            .client_cert(client_cert_file.path(), client_key_file.path());
        let connector = rustls_impl::create_connector(&config, "localhost").unwrap();

        let tcp_stream = TcpStream::connect(addr).await.unwrap();
        let mut tls_stream = rustls_impl::wrap_stream(tcp_stream, &connector, "localhost")
            .await
            .expect("mTLS handshake should succeed");

        tls_stream.write_all(b"mtls works").await.unwrap();
        tls_stream.shutdown().await.unwrap();

        let mut response = String::new();
        tls_stream.read_to_string(&mut response).await.unwrap();
        assert_eq!(response, "echo:mtls works");

        server_handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_rustls_mtls_rejected_without_client_cert() {
        let ca = generate_ca();
        let server = generate_server_cert(&ca, "localhost");

        // Server requires client certs (mTLS)
        let (addr, _server_handle) = start_echo_server(
            &server.cert_pem,
            &server.key_pair.serialize_pem(),
            Some(&ca.cert_pem),
        )
        .await;

        // Client omits client certificate
        let ca_pem_file = write_pem_file(&ca.cert_pem);
        let config = TlsConfig::new().ca_cert(ca_pem_file.path());
        let connector = rustls_impl::create_connector(&config, "localhost").unwrap();

        let tcp_stream = TcpStream::connect(addr).await.unwrap();
        let result = rustls_impl::wrap_stream(tcp_stream, &connector, "localhost").await;

        // The handshake may fail outright, or succeed but the server drops the connection
        if let Ok(mut stream) = result {
            let write_result = stream.write_all(b"test").await;
            let mut buf = [0u8; 1024];
            let read_result = stream.read(&mut buf).await;
            assert!(
                write_result.is_err() || read_result.is_err() || read_result.unwrap_or(0) == 0,
                "Server should reject connection without client cert"
            );
        }
    }
}

// =============================================================================
// TlsConfig unit tests (no feature gates needed)
// =============================================================================

mod config_tests {
    use super::*;
    use hyperdb_api_core::client::tls::TlsMode;

    #[test]
    fn test_tls_config_default_values() {
        let config = TlsConfig::default();
        assert!(config.verify_server);
        assert!(config.ca_cert_path.is_none());
        assert!(config.client_cert_path.is_none());
        assert!(config.client_key_path.is_none());
        assert!(config.server_name.is_none());
        assert!(!config.has_client_cert());
    }

    #[test]
    fn test_tls_config_builder_pattern() {
        let ca = generate_ca();
        let client = generate_client_cert(&ca);

        let ca_file = write_pem_file(&ca.cert_pem);
        let cert_file = write_pem_file(&client.cert_pem);
        let key_file = write_pem_file(&client.key_pair.serialize_pem());

        let config = TlsConfig::new()
            .ca_cert(ca_file.path())
            .client_cert(cert_file.path(), key_file.path())
            .server_name("example.com");

        assert!(config.verify_server);
        assert!(config.ca_cert_path.is_some());
        assert!(config.has_client_cert());
        assert_eq!(config.server_name, Some("example.com".to_string()));
    }

    #[test]
    fn test_tls_config_danger_accept_invalid_certs() {
        let config = TlsConfig::new().danger_accept_invalid_certs();
        assert!(!config.verify_server);
    }

    #[test]
    fn test_tls_mode_disable() {
        assert!(!TlsMode::Disable.is_enabled());
        assert!(!TlsMode::Disable.is_required());
        assert!(!TlsMode::Disable.verify_server());
        assert!(!TlsMode::Disable.verify_hostname());
    }

    #[test]
    fn test_tls_mode_prefer() {
        assert!(TlsMode::Prefer.is_enabled());
        assert!(!TlsMode::Prefer.is_required());
    }

    #[test]
    fn test_tls_mode_require() {
        assert!(TlsMode::Require.is_enabled());
        assert!(TlsMode::Require.is_required());
        assert!(!TlsMode::Require.verify_server());
    }

    #[test]
    fn test_tls_mode_verify_ca() {
        assert!(TlsMode::VerifyCA.is_enabled());
        assert!(TlsMode::VerifyCA.is_required());
        assert!(TlsMode::VerifyCA.verify_server());
        assert!(!TlsMode::VerifyCA.verify_hostname());
    }

    #[test]
    fn test_tls_mode_verify_full() {
        assert!(TlsMode::VerifyFull.is_enabled());
        assert!(TlsMode::VerifyFull.is_required());
        assert!(TlsMode::VerifyFull.verify_server());
        assert!(TlsMode::VerifyFull.verify_hostname());
    }
}

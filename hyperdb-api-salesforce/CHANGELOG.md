# Changelog

All notable changes to the `hyperdb-api-salesforce` crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- **BREAKING:** the minimum supported Rust version is now **1.88**, up from
  1.81, and the crate is compiled with **edition 2024**. 1.88 is the version
  Red Hat Enterprise Linux 9.7 ships as `rust-toolset`.
- **BREAKING:** the TLS crypto provider is now **ring** rather than AWS-LC.
  This crate's `reqwest` dependency asked for the `rustls` feature, which
  expands to `__rustls-aws-lc-rs` and forces the `aws-lc-rs` provider — quietly
  overriding the workspace's deliberate `rustls = { features = ["ring"] }`
  selection, because Cargo unifies features across the graph. It now uses
  `rustls-no-provider`, which enables reqwest's rustls plumbing without picking
  a provider and lets the ring selection apply.

  This removes `aws-lc-sys`, which compiled AWS-LC (Amazon's BoringSSL fork)
  from source and required `cmake` plus a C++ toolchain to build. Verified
  against live HTTPS endpoints, not just a green compile.

## [0.1.1] - 2026-05-13

### Added

- `SalesforceAuthConfig` for configuring Data Cloud OAuth credentials
- `AuthMode` enum for selecting between JWT-bearer and other authentication flows
- `DataCloudTokenProvider` with automatic token caching and refresh
- `SharedTokenProvider` for thread-safe concurrent token access (wraps `DataCloudTokenProvider` in an `Arc`)
- `SalesforceAuthError` and `SalesforceAuthResult` for structured error handling
- `DataCloudToken` and `OAuthToken` types representing the issued credentials
- RSA private key signing for JWT assertions
- Integration with `hyperdb-api-core::client::grpc` for authenticated gRPC queries

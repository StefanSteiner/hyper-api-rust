# hyper-client Development Guide

Contributor-facing documentation for the `hyper-client` crate -- the connection layer
between `hyperapi` (high-level API) and `hyper-protocol` (wire protocol).

For user-facing documentation, see [README.md](README.md).

---

## Architecture Overview

`hyper-client` provides two transport families with both sync and async variants:

| Transport | Protocol | Capabilities | Variants |
|-----------|----------|-------------|----------|
| **TCP** | PostgreSQL wire protocol (v3) | Full read/write, DDL, COPY | `Client`, `AsyncClient` |
| **gRPC** | Protobuf over HTTP/2 | Read-only (SELECT), Arrow IPC results | `GrpcClient`, `GrpcClientSync` |

### Module Map

```
src/
  lib.rs                  # Crate root, re-exports
  config.rs               # Config builder for TCP connections
  client.rs               # Sync TCP client (Client, CopyInWriter, QueryStream)
  async_client.rs         # Async TCP client (AsyncClient, AsyncCopyInWriter)
  connection.rs           # RawConnection<S> -- sync wire protocol engine
  async_connection.rs     # AsyncRawConnection<S> -- async wire protocol engine
  auth.rs                 # Authentication: cleartext, MD5, SCRAM-SHA-256
  tls.rs                  # TLS config and rustls integration
  cancel.rs               # Cancellable trait (transport-agnostic cancel)
  endpoint.rs             # ConnectionEndpoint (TCP, Unix, Named Pipe)
  sync_stream.rs          # SyncStream (TCP/Unix/Pipe wrapper for sync I/O)
  async_stream.rs         # AsyncStream (TCP/Unix/Pipe wrapper for async I/O)
  error.rs                # Error types
  notice.rs               # Server notice/warning handling
  row.rs                  # Row, StreamRow, BatchRow
  statement.rs            # Column metadata, ColumnFormat
  prepare.rs              # Prepared statement support
  grpc/
    mod.rs                # gRPC module root, re-exports
    client.rs             # GrpcClient, GrpcClientSync
    config.rs             # GrpcConfig builder
    executor.rs           # GrpcQueryExecutor state machine
    params.rs             # QueryParameters, ParameterStyle
    result.rs             # GrpcQueryResult, GrpcResultChunk
    error.rs              # gRPC error conversion
    proto.rs              # Generated protobuf types
    authenticated_client.rs  # AuthenticatedGrpcClient (salesforce-auth feature)
tests/
  common/mod.rs           # TestServer helper
  client_tests.rs         # Sync client integration tests
  copy_tests.rs           # COPY protocol tests
  prepared_statement_tests.rs
  tls_tests.rs            # TLS with self-signed certs (rcgen)
  async_copy_cancel_tests.rs
```

---

## TCP Transport Internals

### Connection Lifecycle

1. **TCP connect** -- `TcpStream::connect` (or Unix/Named Pipe equivalent)
2. **Startup** -- Send `StartupMessage` with user, database, and protocol version
3. **Authentication** -- Server selects method; client responds (see `auth.rs` module docs)
4. **Parameter exchange** -- Server sends `ParameterStatus` messages (server_version, etc.)
5. **Ready** -- Server sends `ReadyForQuery('I')`, connection is usable

The startup and auth handshake is implemented in `RawConnection::startup()` and
`AsyncRawConnection::startup()`.

### Query Execution Modes

The TCP client supports three query modes. Implementation details (protocol messages,
format codes, memory behavior) are documented in `client.rs` module-level rustdoc.

| Mode | Method | Format | Memory Model |
|------|--------|--------|-------------|
| Simple Query | `query()` | Text | All rows materialized in `Vec<Row>` |
| Extended Query | `query_fast()` | HyperBinary (format code 2, LE binary) | All rows in `Vec<StreamRow>` |
| Streaming | `query_streaming()` | HyperBinary | Chunked via `QueryStream` iterator |

### COPY Protocol

COPY IN uses the PostgreSQL COPY subprotocol:

1. Client sends `Query("COPY ... FROM STDIN WITH (FORMAT ...)")` 
2. Server responds with `CopyInResponse`
3. Client sends `CopyData` messages (buffered in `CopyInWriter`)
4. Client sends `CopyDone` (success) or `CopyFail` (abort)
5. Server responds with `CommandComplete` + `ReadyForQuery`

Supported formats: `HYPERBINARY` (default), `ARROWSTREAM`, `CSV`.
The higher-level `hyperapi::Inserter` handles binary encoding; `CopyInWriter` is
a transport-level data pump.

### Connection Health and Desynchronization

`RawConnection` tracks a `desynchronized` flag. Once set (e.g., a bounded drain
exceeded its budget before seeing `ReadyForQuery`), all operations fast-fail.
Recovery requires dropping the connection and opening a new one.

Key methods:
- `ensure_healthy()` -- checked before every new request
- `is_healthy()` -- used by pool layers during recycle
- `drain_until_ready_bounded(cap)` -- used by `QueryStream::drop` and error recovery
- `consume_error()` -- drains through `ReadyForQuery` after an `ErrorResponse`

### Cancel Mechanism

PG wire cancellation opens a *separate* TCP connection and sends a 16-byte
`CancelRequest` packet (process ID + secret key). This design allows cancellation
from any thread without acquiring the connection mutex.

See `cancel.rs` for the `Cancellable` trait and `Client::cancel()` for the
user-facing fallible API. `QueryStream::drop` uses `Cancellable` to stop
in-flight streaming queries.

---

## gRPC Transport Internals

### Executor State Machine

`GrpcQueryExecutor` (in `grpc/executor.rs`) drives query execution through a state
machine that handles all three transfer modes. The state transitions and per-mode
paths are documented in that module's rustdoc. Key states:

```
ReadInitialResults -> RequestStatus -> ReadStatus -> RequestResults -> ReadResults -> Finished
```

SYNC-mode queries go directly from `ReadInitialResults` to `Finished`.
ADAPTIVE-mode queries may take either path depending on result size.

### Cancel via gRPC

Unlike PG wire, gRPC cancels travel as a regular `CancelQuery` RPC multiplexed
over the existing HTTP/2 channel. The cancel carries `x-hyperdb-query-id` metadata
for server-side routing. See `GrpcClient::cancel_query()` rustdoc for the full
design discussion, including why `GrpcClient` does not implement `Cancellable`.

### Protobuf Generation

The `build.rs` script compiles `.proto` files (from the crate-local `protos/`
directory) into Rust types via `tonic-build`. The generated code lives in
`grpc/proto.rs` (included via `include!`).

To regenerate after proto changes:
```bash
cargo build -p hyper-client
```

---

## Authentication Internals

### Protocol Flow

Authentication is server-directed: the server sends an `AuthenticationRequest`
message indicating which method to use. The client responds accordingly.

The supported methods, their wire formats, and the SCRAM-SHA-256 multi-step
handshake are documented in `auth.rs` module-level rustdoc.

### Key Security Properties

- All intermediate SCRAM key material uses `zeroize::Zeroizing<Vec<u8>>`
- `AuthState` is consumed (moved) by `scram_verify_server`, ensuring keys
  are dropped after verification
- MD5 uses a double-hash scheme: `MD5(MD5(password + user) + salt)`

---

## TLS Internals

TLS is implemented via `rustls` (pure Rust, no OpenSSL dependency). The
implementation lives in `tls.rs` and `tls::rustls_impl`.

- Root certificates: system roots via `webpki-roots` + optional custom CA
- Client certificates: optional mutual TLS (mTLS)
- Key formats: PEM (PKCS#8 or PKCS#1 for private keys)
- TLS modes: `Disable`, `Prefer`, `Require`, `VerifyCA`, `VerifyFull`

gRPC TLS is handled separately by `tonic`'s built-in TLS support, configured
via `GrpcConfig` (auto-detected from `https://` endpoints).

---

## Testing

### Integration Test Setup

Integration tests require a running Hyper server. The `tests/common/mod.rs` module
provides `TestServer`, which:

1. Starts a `HyperProcess` (via `hyperapi` dev-dependency)
2. Creates a temporary database
3. Provides `Config` and `Client` helpers for the test
4. Cleans up on drop

```rust
use crate::common::TestServer;

#[test]
fn test_something() -> hyperapi::Result<()> {
    let server = TestServer::new()?;
    let client = server.connect()?;
    // ... test with real server ...
    Ok(())
}
```

Test output (databases, logs) goes to `hyper-client/test_results/`.

### Running Tests

```bash
# All hyper-client tests (requires hyperd on PATH or HYPERD_PATH set)
cargo test -p hyper-client

# Unit tests only (no server needed)
cargo test -p hyper-client --lib

# Specific test file
cargo test -p hyper-client --test client_tests
```

### TLS Tests

`tls_tests.rs` generates self-signed certificates at runtime using `rcgen`
(dev-dependency) to test TLS handshake, mTLS, and certificate verification
without requiring pre-generated certificates.

### Writing New Tests

- Use `TestServer::new()` for tests that need a database
- Use `TestServer::without_database()` for tests that manage databases explicitly
- Use `#[test]` for sync tests, `#[tokio::test]` for async tests
- Keep test databases in `test_results/` (auto-managed by `TestServer`)
- gRPC tests live in `hyperapi/tests/` since they exercise the full stack

---

## Feature Flags

| Feature | Dependencies Added | What It Enables |
|---------|--------------------|-----------------|
| `salesforce-auth` | `hyperapi-salesforce`, `chrono`, `arrow` | `AuthenticatedGrpcClient`, `with_data_cloud_token()` on `GrpcConfig` |

Everything else (TCP clients, gRPC clients, TLS, auth) is always available.

---

## Design Decisions

### Why Two Client Types (Client / AsyncClient)?

The sync `Client` uses `std::net::TcpStream` + `std::sync::Mutex` for zero
async runtime overhead. The async `AsyncClient` uses `tokio::net::TcpStream` +
`tokio::sync::Mutex`. They share the same wire protocol logic via
`RawConnection<S>` / `AsyncRawConnection<S>`, parameterized over the stream type.

### Why GrpcClientSync Wraps GrpcClient?

`GrpcClientSync` creates an internal `tokio::runtime::Runtime` and blocks on
the async `GrpcClient`. This avoids duplicating the gRPC logic but adds ~1ms
runtime creation overhead. For sync-heavy workloads, create one `GrpcClientSync`
and reuse it.

### Why Format Code 2 (HyperBinary)?

Standard PostgreSQL binary format (code 1) uses big-endian. Hyper extends the
protocol with format code 2 for little-endian binary (`HyperBinary`), which
avoids byte-swapping on x86/ARM and enables zero-copy field access in
`StreamRow`. This is a Hyper-specific extension not supported by standard
PostgreSQL servers.

### Cancellable Trait Design

The `Cancellable` trait is intentionally minimal (`fn cancel(&self)` with no
return value and no arguments) to serve as an internal cleanup abstraction
usable from `Drop` impls. User-facing cancel APIs (`Client::cancel() -> Result`,
`GrpcClient::cancel_query(id) -> Result`) are separate and fallible.
See `cancel.rs` for the full rationale.

---

## Known Tech Debt

- `GrpcClient` duplicates query-building logic between `execute_query_with_options`
  and `execute_query_with_params_and_options` -- should be unified
- `AsyncClient` lacks a streaming query mode equivalent to `QueryStream`
- Connection pooling exists as `deadpool` integration but is not yet exposed
  as a first-class API in this crate
- `TlsConfig` / `TlsMode` are defined but not yet wired into `Config`'s
  builder (TLS is configured at a lower level currently)

---

## Related Documentation

- [Root DEVELOPMENT.md](../DEVELOPMENT.md) -- workspace-wide build, test, CI
- [hyper-protocol README](../hyper-protocol/README.md) -- wire protocol details
- [hyper-types README](../hyper-types/README.md) -- type system and binary formats
- [hyperapi README](../hyperapi/README.md) -- high-level API built on this crate

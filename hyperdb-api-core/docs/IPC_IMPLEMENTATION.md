# IPC Transport Implementation Guide

This document describes the Unix Domain Socket (UDS) implementation and provides a roadmap for implementing Windows Named Pipes support.

## Overview

The Hyper Rust API supports multiple transport mechanisms:
- **TCP**: Traditional TCP/IP connections (cross-platform)
- **Unix Domain Sockets (UDS)**: High-performance IPC on Unix/macOS (completed)
- **Windows Named Pipes**: High-performance IPC on Windows (TODO)

## Architecture

### Core Abstraction: `ConnectionEndpoint`

Location: `hyper-client/src/endpoint.rs`

```rust
pub enum ConnectionEndpoint {
    /// TCP endpoint: `tab.tcp://host:port`
    Tcp { host: String, port: u16 },

    /// Unix Domain Socket: `tab.domain://<directory>/domain/<name>`
    #[cfg(unix)]
    DomainSocket { directory: PathBuf, name: String },

    // TODO: Windows Named Pipe
    // #[cfg(windows)]
    // NamedPipe { host: String, name: String },
}
```

### Stream Abstractions

**Async Stream** (`hyper-client/src/async_stream.rs`):
```rust
pub enum AsyncStream {
    Tcp(tokio::net::TcpStream),
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
    // TODO: #[cfg(windows)]
    // NamedPipe(tokio::net::windows::named_pipe::NamedPipeClient),
}
```

**Sync Stream** (`hyper-client/src/sync_stream.rs`):
```rust
pub enum SyncStream {
    Tcp(std::net::TcpStream),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
    // TODO: #[cfg(windows)]
    // NamedPipe(std::os::windows::...),
}
```

## Connection Descriptor Formats

The C API uses the following descriptor formats:
- TCP: `tab.tcp://host:port`
- Unix Domain Socket: `tab.domain://<directory>/domain/<socket_name>`
- Windows Named Pipe: `tab.pipe://<host>/pipe/<pipe_name>`

## Unix Domain Socket Implementation (Reference)

### Files Modified

1. **`hyper-client/src/endpoint.rs`** - ConnectionEndpoint enum with parsing
2. **`hyper-client/src/async_stream.rs`** - AsyncStream with Unix variant
3. **`hyper-client/src/sync_stream.rs`** - SyncStream with Unix variant
4. **`hyper-client/src/async_client.rs`** - `connect_unix()`, `connect_endpoint()`
5. **`hyper-client/src/client.rs`** - `connect_unix()`, `connect_endpoint()`
6. **`hyper-client/src/prepare.rs`** - Updated to use SyncStream
7. **`hyperapi/src/process.rs`** - TransportMode, UDS listen connection
8. **`hyperapi/src/transport.rs`** - TransportType::UnixSocket
9. **`hyperapi/src/connection_builder.rs`** - `build_unix()`
10. **`hyperapi/src/connection.rs`** - `connect_with_endpoint()`

### Key Implementation Details

#### 1. Stream Trait Implementations

Both `AsyncStream` and `SyncStream` implement the necessary I/O traits by delegating to the underlying stream:

```rust
// async_stream.rs
impl AsyncRead for AsyncStream { ... }
impl AsyncWrite for AsyncStream { ... }

// sync_stream.rs
impl Read for SyncStream { ... }
impl Write for SyncStream { ... }
```

#### 2. Client Connection Methods

Each client has three connection methods:
- `connect(&Config)` - TCP via Config (original)
- `connect_unix(path, &Config)` - UDS via socket path
- `connect_endpoint(&ConnectionEndpoint, &Config)` - Generic via endpoint

#### 3. HyperProcess Integration

In `hyperapi/src/process.rs`:
- `TransportMode` enum: `Ipc` (default on Unix) or `Tcp`
- Socket directory created in temp folder: `/tmp/hyper-<pid>/`
- Listen connection format: `tab.domain://<dir>/domain/hyper`
- Cleanup on drop: removes socket directory

#### 4. Connection::new() Auto-Detection

When connecting via `Connection::new(&HyperProcess, ...)`:
- On Unix: Uses `connection_endpoint()` if available (supports UDS)
- Falls back to string endpoint parsing (TCP)

---

## Windows Named Pipes Implementation Plan

### Phase 1: Add NamedPipe Variant to ConnectionEndpoint

**File: `hyper-client/src/endpoint.rs`**

```rust
pub enum ConnectionEndpoint {
    Tcp { host: String, port: u16 },
    
    #[cfg(unix)]
    DomainSocket { directory: PathBuf, name: String },
    
    #[cfg(windows)]
    NamedPipe {
        /// Server name (use "." for local)
        host: String,
        /// Pipe name
        name: String,
    },
}
```

Add methods:
```rust
#[cfg(windows)]
pub fn named_pipe(host: impl Into<String>, name: impl Into<String>) -> Self { ... }

// Update parse() to handle "tab.pipe://<host>/pipe/<name>"
```

### Phase 2: Implement Windows Stream Abstractions

**File: `hyper-client/src/sync_stream.rs`**

```rust
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, RawHandle};
// Or use the `windows` crate for named pipe support

#[cfg(windows)]
NamedPipe(/* Windows named pipe type */),
```

For synchronous named pipes, you may need:
- `windows` crate or direct Win32 API calls
- `CreateFile` to open the pipe
- Implement `Read` and `Write` traits

**File: `hyper-client/src/async_stream.rs`**

For async named pipes with Tokio:
```rust
#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeClient;

#[cfg(windows)]
NamedPipe(NamedPipeClient),
```

### Phase 3: Update Client Connection Methods

**File: `hyper-client/src/client.rs`**

Add:
```rust
#[cfg(windows)]
pub fn connect_named_pipe(pipe_path: &str, config: &Config) -> Result<Self> { ... }
```

Update `connect_endpoint()` to handle NamedPipe variant.

**File: `hyper-client/src/async_client.rs`**

Similar updates for async client.

### Phase 4: HyperProcess Named Pipe Support

**File: `hyperapi/src/process.rs`**

Update `start_server()`:
```rust
#[cfg(windows)]
let listen_connection = if transport_mode == TransportMode::Ipc {
    // Format: tab.pipe://./pipe/hyper-<unique_id>
    let pipe_name = format!("hyper-{}", std::process::id());
    format!("tab.pipe://./pipe/{}", pipe_name)
} else {
    "tab.tcp://localhost:0".to_string()
};
```

Update `parse_connection_descriptor()` to handle `tab.pipe://` format.

### Phase 5: Update Transport Detection

**File: `hyperapi/src/transport.rs`**

```rust
pub enum TransportType {
    Tcp,
    #[cfg(unix)]
    UnixSocket,
    #[cfg(windows)]
    NamedPipe,
    Grpc,
}

pub fn detect_transport_type(endpoint: &str) -> TransportType {
    // Add detection for "tab.pipe://" or "\\\\.\\pipe\\"
}
```

### Phase 6: ConnectionBuilder Support

**File: `hyperapi/src/connection_builder.rs`**

Add:
```rust
#[cfg(windows)]
fn build_named_pipe(self) -> Result<Connection> { ... }
```

Update `build()` match to handle `TransportType::NamedPipe`.

---

## Testing Strategy

1. **Unit Tests**: Add tests in `endpoint.rs` for `tab.pipe://` parsing
2. **Integration Tests**: Test named pipe connections with HyperProcess
3. **Benchmark**: Extend `benchmark.rs` to compare Named Pipe vs TCP on Windows

## Key Crates for Windows

- `tokio` - Has `tokio::net::windows::named_pipe` module
- `windows` crate - For low-level Win32 API if needed
- `winapi` crate - Alternative for Win32 bindings

## Named Pipe Format Reference

Windows named pipe paths:
- Local: `\\.\pipe\<pipe_name>`
- Remote: `\\<server>\pipe\<pipe_name>`

Hyper descriptor format:
- `tab.pipe://./pipe/<name>` (local)
- `tab.pipe://<server>/pipe/<name>` (remote)

## Build Verification

After implementing, verify:
```powershell
cargo build -p hyper-client -p hyperapi
cargo test -p hyper-client -p hyperapi
cargo run -p hyperapi --release --example benchmark 100000
```

## Notes

- Default transport is `TransportMode::Tcp` (see performance investigation below)
- The callback connection remains TCP (simpler, cross-platform)
- Tests force TCP mode for parallel test stability

---

## Performance Investigation: UDS vs TCP on macOS

**Status:** Pending further investigation

### Observed Issue

Initial benchmarks on macOS showed UDS performing **slower** than TCP loopback, contrary to expectations. This section documents the analysis for future investigation.

### Potential Root Causes

#### 1. macOS TCP Loopback is Highly Optimized

macOS XNU kernel has specialized fast paths for TCP loopback (`127.0.0.1`). The kernel detects loopback connections and bypasses most of the TCP/IP stack, performing direct memory copies between socket buffers. This is a mature, heavily optimized code path.

#### 2. Default Socket Buffer Sizes

On macOS:
- **TCP loopback**: Default `SO_SNDBUF`/`SO_RCVBUF` ~128KB-256KB
- **Unix sockets**: Default buffers often ~8KB-64KB (significantly smaller)

Smaller buffers mean more syscalls for the same data volume. The current implementation does not tune socket buffer sizes for UDS.

**Relevant code** (`hyper-client/src/client.rs:228-241`):
```rust
let unix_stream = UnixStream::connect(path).map_err(...)?;
let stream = SyncStream::unix(unix_stream);
// No setsockopt(SO_SNDBUF/SO_RCVBUF) called
```

#### 3. macOS UDS Kernel Path

Unlike Linux (where UDS is heavily optimized for Docker, systemd, etc.), macOS UDS sees less optimization focus. The XNU kernel's UDS implementation has:
- More locking overhead
- Less aggressive batching
- No `MSG_ZEROCOPY` equivalent

#### 4. Path-Based Socket Overhead

macOS only supports path-based Unix sockets (no abstract namespace like Linux). Each operation involves VFS lookups.

#### 5. TCP_NODELAY vs UDS

In `sync_stream.rs`:
```rust
pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
    match self {
        SyncStream::Tcp(stream) => stream.set_nodelay(nodelay),
        SyncStream::Unix(_) => Ok(()), // No-op for Unix sockets
    }
}
```
TCP with `TCP_NODELAY` sends immediately. UDS doesn't have Nagle algorithm, but behavior may differ.

### Potential Fixes

| Fix | Effort | Expected Impact |
|-----|--------|-----------------|
| Set `SO_SNDBUF`/`SO_RCVBUF` to 256KB on UDS | Low | 10-30% improvement |
| Use `writev`/`readv` for vectored I/O | Medium | 5-15% improvement |
| Implement buffered writer wrapper | Medium | Reduce syscall count |
| Test on Linux for comparison | Low | Validate macOS-specific issue |

### Diagnostic Commands

To check socket buffer sizes on macOS:
```bash
# Check system defaults
sysctl kern.ipc.maxsockbuf
sysctl net.local.stream.sendspace
sysctl net.local.stream.recvspace
sysctl net.inet.tcp.sendspace
sysctl net.inet.tcp.recvspace
```

### Recommendation

1. **Default to TCP** until performance is validated across platforms
2. **Test on Linux** to confirm macOS-specific behavior
3. **Add socket buffer tuning** if UDS is to be re-enabled as default
4. Consider **platform-specific defaults**: TCP on macOS, IPC on Linux/Windows

### Code to Add Buffer Tuning (Future)

```rust
#[cfg(unix)]
fn configure_unix_socket(stream: &UnixStream) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    
    let fd = stream.as_raw_fd();
    let buffer_size: libc::c_int = 256 * 1024; // 256KB
    
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            &buffer_size as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            &buffer_size as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
    Ok(())
}
```

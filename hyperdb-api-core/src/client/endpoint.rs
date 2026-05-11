// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Connection endpoint types for Hyper database.
//!
//! This module provides [`ConnectionEndpoint`] for representing different
//! transport mechanisms (TCP, Unix Domain Socket, Windows Named Pipe).

use std::fmt;
#[cfg(unix)]
use std::path::PathBuf;

use super::error::{Error, Result};

/// Represents a connection endpoint for Hyper database.
///
/// Supports different transport mechanisms:
/// - TCP: `tab.tcp://host:port`
/// - Unix Domain Socket: `tab.domain://<directory>/domain/<name>` (Unix only)
/// - Windows Named Pipe: `tab.pipe://<host>/pipe/<name>` (Windows only, future)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEndpoint {
    /// TCP endpoint: `tab.tcp://host:port`
    Tcp {
        /// Hostname or IP address
        host: String,
        /// Port number (0 for auto-assign)
        port: u16,
    },

    /// Unix Domain Socket: `tab.domain://<directory>/domain/<name>`
    #[cfg(unix)]
    DomainSocket {
        /// Directory containing the socket file
        directory: PathBuf,
        /// Socket name (e.g., `.s.PGSQL.12345`)
        name: String,
    },
    /// Windows Named Pipe: `tab.pipe://<host>/pipe/<name>`
    ///
    /// **TODO (Windows)**: Implement Named Pipe support for Windows IPC.
    /// See IPC_IMPLEMENTATION.md for detailed implementation guide.
    ///
    /// Example format: `tab.pipe://./pipe/hyper-12345` for local pipe
    #[cfg(windows)]
    NamedPipe {
        /// Server name ("." for local machine)
        host: String,
        /// Pipe name (e.g., "hyper-12345")
        name: String,
    },
}

impl ConnectionEndpoint {
    /// Creates a new TCP endpoint.
    pub fn tcp(host: impl Into<String>, port: u16) -> Self {
        ConnectionEndpoint::Tcp {
            host: host.into(),
            port,
        }
    }

    /// Creates a new Unix Domain Socket endpoint.
    #[cfg(unix)]
    pub fn domain_socket(directory: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        ConnectionEndpoint::DomainSocket {
            directory: directory.into(),
            name: name.into(),
        }
    }

    /// Creates a new Windows Named Pipe endpoint.
    ///
    /// # Arguments
    /// * `host` - Server name ("." for local machine)
    /// * `name` - Pipe name (e.g., "hyper-12345")
    #[cfg(windows)]
    pub fn named_pipe(host: impl Into<String>, name: impl Into<String>) -> Self {
        ConnectionEndpoint::NamedPipe {
            host: host.into(),
            name: name.into(),
        }
    }

    /// Parses a connection descriptor string.
    ///
    /// Supported formats:
    /// - `tab.tcp://host:port` or `host:port` → TCP
    /// - `tab.domain://<dir>/domain/<name>` → Unix Domain Socket
    /// - `tab.pipe://<host>/pipe/<name>` → Named Pipe (future)
    ///
    /// # Errors
    ///
    /// Returns [`Error`] (connection) when:
    /// - The descriptor has the `tab.domain://` prefix but is missing
    ///   the `/domain/` separator, has an empty socket name, or is
    ///   used on a non-Unix platform.
    /// - The descriptor has the `tab.pipe://` prefix but malformed
    ///   `/pipe/` segment, empty name, or is used on a non-Windows
    ///   platform.
    /// - The TCP descriptor cannot be parsed into a `host:port` pair,
    ///   or the port is not a valid `u16`.
    pub fn parse(descriptor: &str) -> Result<Self> {
        // Unix Domain Socket: tab.domain://<directory>/domain/<name>
        #[allow(unused_variables, reason = "`rest` is unused on non-unix platforms")]
        if let Some(rest) = descriptor.strip_prefix("tab.domain://") {
            #[cfg(unix)]
            {
                let idx = rest.find("/domain/").ok_or_else(|| {
                    Error::connection(format!(
                        "Invalid domain socket format: '{descriptor}'. Expected 'tab.domain://<dir>/domain/<name>'"
                    ))
                })?;
                let directory = &rest[..idx];
                let name = &rest[idx + 8..]; // "/domain/".len() == 8

                if name.is_empty() {
                    return Err(Error::connection("Domain socket name cannot be empty"));
                }

                return Ok(ConnectionEndpoint::DomainSocket {
                    directory: PathBuf::from(directory),
                    name: name.to_string(),
                });
            }
            #[cfg(not(unix))]
            {
                return Err(Error::connection(
                    "Unix domain sockets are not supported on this platform",
                ));
            }
        }

        // Named Pipe: tab.pipe://<host>/pipe/<name>
        #[allow(unused_variables, reason = "`rest` is unused on non-windows platforms")]
        if let Some(rest) = descriptor.strip_prefix("tab.pipe://") {
            #[cfg(windows)]
            {
                // Format: tab.pipe://<host>/pipe/<name>
                // Example: tab.pipe://./pipe/hyper-12345
                let idx = rest.find("/pipe/").ok_or_else(|| {
                    Error::connection(format!(
                        "Invalid named pipe format: '{descriptor}'. Expected 'tab.pipe://<host>/pipe/<name>'"
                    ))
                })?;
                let host = &rest[..idx];
                let name = &rest[idx + 6..]; // "/pipe/".len() == 6

                if name.is_empty() {
                    return Err(Error::connection("Named pipe name cannot be empty"));
                }

                return Ok(ConnectionEndpoint::NamedPipe {
                    host: host.to_string(),
                    name: name.to_string(),
                });
            }
            #[cfg(not(windows))]
            {
                return Err(Error::connection(
                    "Named pipes are not supported on this platform",
                ));
            }
        }

        // TCP: tab.tcp://host:port or just host:port
        let tcp_part = descriptor
            .strip_prefix("tab.tcp://")
            .or_else(|| descriptor.strip_prefix("tcp.libpq://"))
            .unwrap_or(descriptor);

        Self::parse_tcp(tcp_part)
    }

    /// Parses a TCP host:port string.
    fn parse_tcp(s: &str) -> Result<Self> {
        // Handle IPv6 addresses: [::1]:port
        if s.starts_with('[') {
            let end_bracket = s
                .find(']')
                .ok_or_else(|| Error::connection(format!("Invalid IPv6 address format: '{s}'")))?;
            let host = &s[1..end_bracket];
            let port_str = s[end_bracket + 1..]
                .strip_prefix(':')
                .ok_or_else(|| Error::connection(format!("Missing port in: '{s}'")))?;

            let port = Self::parse_port(port_str)?;
            return Ok(ConnectionEndpoint::Tcp {
                host: host.to_string(),
                port,
            });
        }

        // Regular host:port
        let colon_idx = s.rfind(':').ok_or_else(|| {
            Error::connection(format!(
                "Invalid endpoint format: '{s}'. Expected 'host:port'"
            ))
        })?;

        let host = &s[..colon_idx];
        let port_str = &s[colon_idx + 1..];

        if host.is_empty() {
            return Err(Error::connection("Host cannot be empty"));
        }

        let port = Self::parse_port(port_str)?;

        Ok(ConnectionEndpoint::Tcp {
            host: host.to_string(),
            port,
        })
    }

    /// Parses a port string, handling "auto" as 0.
    fn parse_port(s: &str) -> Result<u16> {
        if s == "auto" {
            return Ok(0);
        }
        s.parse::<u16>()
            .map_err(|_| Error::connection(format!("Invalid port number: '{s}'")))
    }

    /// Returns the connection descriptor string format.
    ///
    /// This is the format used by Hyper for `--listen-connection` and `--callback-connection`.
    #[must_use]
    pub fn to_descriptor(&self) -> String {
        match self {
            ConnectionEndpoint::Tcp { host, port } => {
                let port_str = if *port == 0 {
                    "auto".to_string()
                } else {
                    port.to_string()
                };
                // Handle IPv6 addresses
                if host.contains(':') {
                    format!("tab.tcp://[{host}]:{port_str}")
                } else {
                    format!("tab.tcp://{host}:{port_str}")
                }
            }
            #[cfg(unix)]
            ConnectionEndpoint::DomainSocket { directory, name } => {
                format!("tab.domain://{}/domain/{}", directory.display(), name)
            }
            #[cfg(windows)]
            ConnectionEndpoint::NamedPipe { host, name } => {
                format!("tab.pipe://{host}/pipe/{name}")
            }
        }
    }

    /// Returns the socket file path for Unix Domain Sockets.
    #[cfg(unix)]
    #[must_use]
    pub fn socket_path(&self) -> Option<PathBuf> {
        match self {
            ConnectionEndpoint::DomainSocket { directory, name } => Some(directory.join(name)),
            ConnectionEndpoint::Tcp { .. } => None,
        }
    }

    /// Returns the Windows pipe path (e.g., `\\.\pipe\hyper-12345`).
    #[cfg(windows)]
    pub fn pipe_path(&self) -> Option<String> {
        match self {
            ConnectionEndpoint::NamedPipe { host, name } => {
                Some(format!("\\\\{host}\\pipe\\{name}"))
            }
            ConnectionEndpoint::Tcp { .. } => None,
        }
    }

    /// Returns true if this is a TCP endpoint.
    #[must_use]
    pub fn is_tcp(&self) -> bool {
        matches!(self, ConnectionEndpoint::Tcp { .. })
    }

    /// Returns true if this is a Unix Domain Socket endpoint.
    #[cfg(unix)]
    #[must_use]
    pub fn is_domain_socket(&self) -> bool {
        matches!(self, ConnectionEndpoint::DomainSocket { .. })
    }

    /// Returns true if this is a Windows Named Pipe endpoint.
    #[cfg(windows)]
    pub fn is_named_pipe(&self) -> bool {
        matches!(self, ConnectionEndpoint::NamedPipe { .. })
    }

    /// Returns the host and port for TCP endpoints.
    #[must_use]
    pub fn tcp_addr(&self) -> Option<(&str, u16)> {
        match self {
            ConnectionEndpoint::Tcp { host, port } => Some((host, *port)),
            #[cfg(unix)]
            ConnectionEndpoint::DomainSocket { .. } => None,
            #[cfg(windows)]
            ConnectionEndpoint::NamedPipe { .. } => None,
        }
    }
}

impl fmt::Display for ConnectionEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectionEndpoint::Tcp { host, port } => {
                if host.contains(':') {
                    write!(f, "[{host}]:{port}")
                } else {
                    write!(f, "{host}:{port}")
                }
            }
            #[cfg(unix)]
            ConnectionEndpoint::DomainSocket { directory, name } => {
                write!(f, "{}/{}", directory.display(), name)
            }
            #[cfg(windows)]
            ConnectionEndpoint::NamedPipe { host, name } => {
                write!(f, "\\\\{host}\\pipe\\{name}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tcp_simple() {
        let ep = ConnectionEndpoint::parse("localhost:7483").unwrap();
        assert_eq!(
            ep,
            ConnectionEndpoint::Tcp {
                host: "localhost".to_string(),
                port: 7483
            }
        );
    }

    #[test]
    fn test_parse_tcp_with_scheme() {
        let ep = ConnectionEndpoint::parse("tab.tcp://127.0.0.1:7483").unwrap();
        assert_eq!(
            ep,
            ConnectionEndpoint::Tcp {
                host: "127.0.0.1".to_string(),
                port: 7483
            }
        );
    }

    #[test]
    fn test_parse_tcp_auto_port() {
        let ep = ConnectionEndpoint::parse("tab.tcp://localhost:auto").unwrap();
        assert_eq!(
            ep,
            ConnectionEndpoint::Tcp {
                host: "localhost".to_string(),
                port: 0
            }
        );
    }

    #[test]
    fn test_parse_tcp_ipv6() {
        let ep = ConnectionEndpoint::parse("tab.tcp://[::1]:7483").unwrap();
        assert_eq!(
            ep,
            ConnectionEndpoint::Tcp {
                host: "::1".to_string(),
                port: 7483
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_parse_domain_socket() {
        let ep =
            ConnectionEndpoint::parse("tab.domain:///tmp/hyper/domain/.s.PGSQL.12345").unwrap();
        assert_eq!(
            ep,
            ConnectionEndpoint::DomainSocket {
                directory: PathBuf::from("/tmp/hyper"),
                name: ".s.PGSQL.12345".to_string()
            }
        );
    }

    #[test]
    fn test_to_descriptor_tcp() {
        let ep = ConnectionEndpoint::tcp("localhost", 7483);
        assert_eq!(ep.to_descriptor(), "tab.tcp://localhost:7483");
    }

    #[test]
    fn test_to_descriptor_tcp_auto() {
        let ep = ConnectionEndpoint::tcp("localhost", 0);
        assert_eq!(ep.to_descriptor(), "tab.tcp://localhost:auto");
    }

    #[cfg(unix)]
    #[test]
    fn test_to_descriptor_domain_socket() {
        let ep = ConnectionEndpoint::domain_socket("/tmp/hyper", ".s.PGSQL.12345");
        assert_eq!(
            ep.to_descriptor(),
            "tab.domain:///tmp/hyper/domain/.s.PGSQL.12345"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_socket_path() {
        let ep = ConnectionEndpoint::domain_socket("/tmp/hyper", ".s.PGSQL.12345");
        assert_eq!(
            ep.socket_path(),
            Some(PathBuf::from("/tmp/hyper/.s.PGSQL.12345"))
        );
    }

    #[test]
    fn test_display_tcp() {
        let ep = ConnectionEndpoint::tcp("localhost", 7483);
        assert_eq!(format!("{ep}"), "localhost:7483");
    }

    #[cfg(unix)]
    #[test]
    fn test_display_domain_socket() {
        let ep = ConnectionEndpoint::domain_socket("/tmp/hyper", ".s.PGSQL.12345");
        assert_eq!(format!("{ep}"), "/tmp/hyper/.s.PGSQL.12345");
    }

    #[cfg(windows)]
    #[test]
    fn test_parse_named_pipe() {
        let ep = ConnectionEndpoint::parse("tab.pipe://./pipe/hyper-12345").unwrap();
        assert_eq!(
            ep,
            ConnectionEndpoint::NamedPipe {
                host: ".".to_string(),
                name: "hyper-12345".to_string()
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_parse_named_pipe_remote() {
        let ep = ConnectionEndpoint::parse("tab.pipe://server1/pipe/hyper-db").unwrap();
        assert_eq!(
            ep,
            ConnectionEndpoint::NamedPipe {
                host: "server1".to_string(),
                name: "hyper-db".to_string()
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_to_descriptor_named_pipe() {
        let ep = ConnectionEndpoint::named_pipe(".", "hyper-12345");
        assert_eq!(ep.to_descriptor(), "tab.pipe://./pipe/hyper-12345");
    }

    #[cfg(windows)]
    #[test]
    fn test_pipe_path() {
        let ep = ConnectionEndpoint::named_pipe(".", "hyper-12345");
        assert_eq!(ep.pipe_path(), Some(r"\\.\pipe\hyper-12345".to_string()));
    }

    #[cfg(windows)]
    #[test]
    fn test_display_named_pipe() {
        let ep = ConnectionEndpoint::named_pipe(".", "hyper-12345");
        assert_eq!(format!("{ep}"), r"\\.\pipe\hyper-12345");
    }

    #[cfg(windows)]
    #[test]
    fn test_named_pipe_is_methods() {
        let ep = ConnectionEndpoint::named_pipe(".", "test");
        assert!(!ep.is_tcp());
        assert!(ep.is_named_pipe());
    }
}

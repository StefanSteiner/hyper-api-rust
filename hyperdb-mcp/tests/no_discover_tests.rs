// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Regression tests for the `NoDiscover` stdio transport wrapper.
//!
//! Claude Code opens with a `server/discover` probe. The wrapper must answer
//! it with JSON-RPC `-32601` (method not found) without rmcp seeing it, so
//! the client falls back to `initialize`, which must then succeed on the
//! same connection. No `hyperd` is needed: the server is built with no
//! persistent database and never touches the engine during the handshake.

use hyperdb_mcp::no_discover::NoDiscover;
use hyperdb_mcp::server::HyperMcpServer;
use rmcp::ServiceExt;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

async fn read_json_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let mut line = String::new();
    let n = tokio::time::timeout(Duration::from_secs(30), reader.read_line(&mut line)).await??;
    assert!(n > 0, "server closed the stream without replying");
    Ok(serde_json::from_str(line.trim())?)
}

#[tokio::test]
async fn discover_gets_method_not_found_and_initialize_still_succeeds() -> TestResult {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server_io);
    let (client_read, mut client_write) = tokio::io::split(client_io);
    let mut client_read = BufReader::new(client_read);

    let server = HyperMcpServer::with_no_daemon(None, true, true);
    let transport = NoDiscover(rmcp::transport::IntoTransport::<
        rmcp::service::RoleServer,
        _,
        _,
    >::into_transport((server_read, server_write)));
    let server_task = tokio::spawn(async move {
        let service = server.serve(transport).await?;
        service.waiting().await?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    });

    // 1. The discover probe is rejected as an unknown method.
    let discover = json!({
        "jsonrpc": "2.0", "id": 1, "method": "server/discover", "params": {}
    });
    client_write
        .write_all(format!("{discover}\n").as_bytes())
        .await?;
    let reply = read_json_line(&mut client_read).await?;
    assert_eq!(reply["id"], 1, "reply: {reply}");
    assert_eq!(reply["error"]["code"], -32601, "reply: {reply}");
    assert!(reply.get("result").is_none(), "reply: {reply}");

    // 2. The fallback `initialize` succeeds on the same connection.
    let initialize = json!({
        "jsonrpc": "2.0", "id": 2, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "no-discover-test", "version": "0"}
        }
    });
    client_write
        .write_all(format!("{initialize}\n").as_bytes())
        .await?;
    let reply = read_json_line(&mut client_read).await?;
    assert_eq!(reply["id"], 2, "reply: {reply}");
    assert!(reply.get("error").is_none(), "initialize failed: {reply}");
    assert!(
        reply["result"]["serverInfo"].is_object(),
        "initialize result lacks serverInfo: {reply}"
    );

    drop(client_write);
    drop(client_read);
    let _ = tokio::time::timeout(Duration::from_secs(10), server_task).await;
    Ok(())
}

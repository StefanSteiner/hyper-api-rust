// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Stdio transport wrapper that hides `server/discover` from rmcp.

/// Stdio transport that answers `server/discover` itself with `-32601`.
///
/// Claude Code opens with a `server/discover` probe naming protocol
/// `2026-07-28`. rmcp would either serve it, which locks the session into the
/// stateless lifecycle and makes `initialize` answer `2026-07-28`, or reject it
/// with `-32022`; both end in the client's "Server's protocol version is not
/// supported". A server that predates `server/discover` answers `Method not
/// found`, which makes the client fall back to `initialize`, so rmcp never sees
/// the probe.
#[derive(Debug)]
pub struct NoDiscover<T>(pub T);

impl<T: rmcp::transport::Transport<rmcp::service::RoleServer>>
    rmcp::transport::Transport<rmcp::service::RoleServer> for NoDiscover<T>
{
    type Error = T::Error;

    fn send(
        &mut self,
        item: rmcp::model::ServerJsonRpcMessage,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.0.send(item)
    }

    async fn receive(&mut self) -> Option<rmcp::model::ClientJsonRpcMessage> {
        loop {
            let message = self.0.receive().await?;
            if let rmcp::model::ClientJsonRpcMessage::Request(request) = &message
                && matches!(
                    request.request,
                    rmcp::model::ClientRequest::DiscoverRequest(_)
                )
            {
                let error = rmcp::model::ErrorData::method_not_found::<
                    rmcp::model::DiscoverRequestMethod,
                >();
                let reply =
                    rmcp::model::ServerJsonRpcMessage::error(error, Some(request.id.clone()));
                if self.0.send(reply).await.is_err() {
                    return None;
                }
                continue;
            }
            return Some(message);
        }
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        self.0.close()
    }
}

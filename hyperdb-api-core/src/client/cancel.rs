// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Query cancellation abstraction.
//!
//! Hyper supports two transport flavors — the `PostgreSQL` wire protocol over
//! TCP / domain sockets / named pipes, and gRPC over HTTP/2 (used by
//! Salesforce Data 360). Each transport has its own idiomatic way to signal
//! "cancel the query currently running on this connection":
//!
//! | Transport | Cancel mechanism |
//! |-----------|------------------|
//! | PG wire   | Open a *separate* connection to the same endpoint and send a pre-auth `CancelRequest` packet (process id + secret key). The server recognizes it and signals the in-flight query on the original connection to abort. |
//! | gRPC      | Send a `cancel_query(query_id)` RPC over the *same* HTTP/2 channel (HTTP/2 natively multiplexes streams). |
//!
//! Both boil down to a "fire-and-forget best-effort cancel" from the
//! caller's point of view, so we expose them through the [`Cancellable`]
//! trait. Consumers (e.g. [`super::client::QueryStream`]'s `Drop` impl)
//! take a `&dyn Cancellable` and don't care which transport is underneath.
//!
//! Cancellation is inherently racy: the server may have already emitted
//! the last `DataRow` before the cancel lands, the query may complete
//! normally before the cancel is processed, or the network may drop the
//! cancel packet entirely. All implementations therefore treat cancel as
//! best-effort: errors are logged via `tracing::warn!` and then swallowed,
//! because callers are typically in destructor or error-cleanup paths
//! where propagating an error is inappropriate.

/// Anything that can signal the server to abort the query currently running
/// on an associated connection.
///
/// Implementations must be `Send + Sync` so that streaming result types can
/// hold a `&dyn Cancellable` across await points and thread boundaries.
///
/// # Guarantees
///
/// - `cancel()` is **fire-and-forget**. It does not block waiting for the
///   server to acknowledge the cancel or for the in-flight query to
///   actually stop. Callers that need to observe the query's final state
///   should drain the original connection (see
///   [`super::connection::RawConnection::drain_until_ready_bounded`]).
/// - `cancel()` **never panics and never returns an error**. Any
///   transport-level failures (e.g. the cancel connection can't be opened)
///   are logged and swallowed. This keeps `cancel()` usable from `Drop`
///   impls, which cannot propagate errors.
/// - `cancel()` is **late-safe, provided it is synchronous with the
///   server**. A cancel can arrive after the query it targeted finished (the
///   natural `ReadyForQuery` and the cancel race). Hyper does **not** discard
///   such a cancel: if it is processed while the next query is running, that
///   query fails with `57014 canceled` (reproduced by
///   `late_cancel_never_fails_the_next_query`). The PG wire implementation
///   therefore waits for the server to close the cancel connection (its
///   "request processed" signal, as libpq's `PQcancel` does) before
///   returning, which orders the cancel before anything the caller sends
///   next. Implementations must preserve that ordering. (Windows named
///   pipes do not wait today, so the race remains possible there.) The gRPC
///   `cancel_query` RPC is keyed on a server-assigned query id and returns
///   gracefully when the id corresponds to a completed query.
///
/// # Writing an implementation
///
/// `Cancellable` is an **internal cleanup abstraction**, not a user-facing
/// cancel API. Most transports already have a natural
/// `pub fn cancel_...(..) -> Result<(), TransportError>` method that users
/// call directly when they want error-aware cancellation (metrics, retry,
/// user feedback, etc). A `Cancellable` impl is a thin wrapper around that
/// fallible API that swallows transport errors (logged via
/// `tracing::warn!`) so the trait method can satisfy its no-error
/// guarantee.
///
/// The canonical example is
/// [`impl Cancellable for super::client::Client`](super::client::Client),
/// which wraps the fallible
/// [`Client::cancel`](super::client::Client::cancel) PG wire
/// `CancelRequest` method:
///
/// ```ignore
/// impl Cancellable for Client {
///     fn cancel(&self) {
///         if let Err(e) = Client::cancel(self) {
///             tracing::warn!(error = %e, "cancel failed (swallowed)");
///         }
///     }
/// }
/// ```
///
/// The gRPC transport has the same fallible user API
/// ([`GrpcClient::cancel_query`](super::grpc::GrpcClient::cancel_query))
/// but **no `Cancellable` impl today** — gRPC has no streaming result type
/// whose `Drop` would consume `&dyn Cancellable`, and `Cancellable::cancel`
/// takes no arguments so it cannot be implemented directly on `GrpcClient`
/// (which doesn't know *which* `query_id` to cancel). A future gRPC
/// streaming type will introduce a per-query handle along the lines of
/// `GrpcCancelHandle { client, query_id }` that implements `Cancellable`
/// by wrapping and swallowing `GrpcClient::cancel_query`.
pub trait Cancellable: Send + Sync {
    /// Send a best-effort cancel signal for the query currently in flight
    /// on the associated connection.
    fn cancel(&self);
}

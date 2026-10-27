//! The HTTP server loop and the request deadline (docs/phase1/conventions.md §3).
//!
//! [`run`] does what `axum::serve` does, plus the timeout `axum::serve` cannot set: hyper
//! enforces its header read timeout only when it has a timer, and `axum::serve` gives it none,
//! so a client that trickles its headers would hold a connection forever. [`deadline`] bounds
//! the rest of a request: reading its body and producing the response.

use std::future::Future;
use std::pin::pin;
use std::time::Duration;

use axum::Router;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tower::ServiceExt;

use crate::error::{AppError, msg};

/// How long a client may take to send the headers of a request (hyper's own default).
pub const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a request may take from its headers to the start of its response, reading the body
/// included: long enough for a 50 MB picture over a slow uplink, short enough that a client
/// that sends one byte at a time cannot hold a task for days. A streamed response body (the
/// export) is not counted.
pub const REQUEST_DEADLINE: Duration = Duration::from_secs(600);

/// Serves `router` on `listener` until `shutdown` resolves, then waits for the open connections
/// to finish their requests. Every request carries `ConnectInfo<SocketAddr>` (the `ClientIp`
/// extractor reads it). A connection that has not sent a complete request head within
/// `header_timeout` is closed.
pub async fn run(
    listener: TcpListener,
    router: Router,
    header_timeout: Duration,
    shutdown: impl Future<Output = ()>,
) {
    let mut builder = Builder::new(TokioExecutor::new());
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(header_timeout);
    builder.http2().timer(TokioTimer::new());
    let graceful = GracefulShutdown::new();
    let mut shutdown = pin!(shutdown);
    loop {
        let (stream, remote) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    // Out of file descriptors, say: wait for some to close instead of spinning.
                    tracing::warn!(%error, "cannot accept a connection");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        let service = router
            .clone()
            .map_request(move |mut request: Request<Incoming>| {
                request.extensions_mut().insert(ConnectInfo(remote));
                request
            });
        let connection = builder
            .serve_connection_with_upgrades(TokioIo::new(stream), TowerToHyperService::new(service))
            .into_owned();
        let connection = graceful.watch(connection);
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::debug!(%error, "connection closed with an error");
            }
        });
    }
    drop(listener);
    graceful.shutdown().await;
}

/// Middleware (lib.rs): 408 `The request took too long` when a request has not produced its
/// response within `limit` ([`REQUEST_DEADLINE`]). The handler is dropped, which rolls its
/// transaction back.
pub async fn deadline(limit: Duration, request: axum::extract::Request, next: Next) -> Response {
    match tokio::time::timeout(limit, next.run(request)).await {
        Ok(response) => response,
        Err(_) => AppError::new(StatusCode::REQUEST_TIMEOUT, msg::REQUEST_TIMEOUT).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::body::to_bytes;
    use axum::middleware;
    use axum::routing::get;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::sync::oneshot;
    use tokio::time::timeout;

    use super::*;

    const WAIT: Duration = Duration::from_secs(10);

    /// Everything a connection sends until it closes (or `WAIT` passes).
    async fn read_all(stream: &mut TcpStream) -> String {
        let mut received = Vec::new();
        let _ = timeout(WAIT, stream.read_to_end(&mut received)).await;
        String::from_utf8_lossy(&received).into_owned()
    }

    #[tokio::test]
    async fn requests_are_served_with_the_peer_address_and_slow_headers_are_cut_off() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new().route(
            "/",
            get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.ip().to_string() }),
        );
        let (stop, stopped) = oneshot::channel::<()>();
        let server = tokio::spawn(run(
            listener,
            router,
            Duration::from_millis(300),
            async move {
                let _ = stopped.await;
            },
        ));

        let mut client = TcpStream::connect(address).await.unwrap();
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let answer = read_all(&mut client).await;
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        assert!(answer.ends_with("127.0.0.1"), "{answer}");

        // Headers that never end: the connection is closed (hyper may answer 408 first).
        let mut slow = TcpStream::connect(address).await.unwrap();
        slow.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n")
            .await
            .unwrap();
        let started = tokio::time::Instant::now();
        let answer = read_all(&mut slow).await;
        assert!(started.elapsed() < WAIT, "the connection stayed open");
        assert!(
            answer.is_empty() || answer.starts_with("HTTP/1.1 408"),
            "{answer}"
        );

        stop.send(()).unwrap();
        timeout(WAIT, server)
            .await
            .expect("the server stops")
            .unwrap();
    }

    #[tokio::test]
    async fn a_request_past_its_deadline_is_a_json_408() {
        let router = Router::new()
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    "late"
                }),
            )
            .route("/fast", get(|| async { "ok" }))
            .layer(middleware::from_fn(|request, next| {
                deadline(Duration::from_millis(100), request, next)
            }));
        let call = |path: &'static str| {
            let router = router.clone();
            async move {
                let request = Request::get(path).body(axum::body::Body::empty()).unwrap();
                router.oneshot(request).await.unwrap()
            }
        };
        let response = call("/slow").await;
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body[..], br#"{"detail":"The request took too long"}"#);
        assert_eq!(call("/fast").await.status(), StatusCode::OK);
    }
}

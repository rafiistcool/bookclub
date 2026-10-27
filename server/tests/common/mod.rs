//! Test harness (docs/phase1/conventions.md §17.2).
//!
//! This is the minimal Wave 1 step 1 version: a DEV_MODE configuration, an `AppState` built from
//! it, and a one-shot request helper. Wave 1 step 3 extends it into the full frozen harness
//! (`TestApp`, `TestClient`, `FakeCatalog`, `CapturingEmail`, `MemMediaStore`, wiremock).
#![allow(dead_code)]

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use bookclub_server::AppState;
use bookclub_server::auth::password::Passwords;
use bookclub_server::config::Config;
use serde_json::Value;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;

/// A DEV_MODE configuration plus `extra` variables (later entries win).
pub fn test_config(extra: &[(&str, &str)]) -> Config {
    let mut pairs: Vec<(String, String)> = vec![
        ("DATABASE_URL".into(), "postgres://unused".into()),
        ("DEV_MODE".into(), "true".into()),
    ];
    pairs.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    Config::from_lookup(|key| {
        pairs
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    })
    .expect("valid test configuration")
}

/// An `AppState` over `db` with the test configuration and cheap argon2 parameters.
pub fn test_state_with(db: PgPool, config: Config) -> AppState {
    let mut state = AppState::new(db, config).expect("test state");
    state.passwords = Arc::new(Passwords::for_tests());
    state
}

/// [`test_state_with`] and the default test configuration.
pub fn test_state(db: PgPool) -> AppState {
    test_state_with(db, test_config(&[]))
}

/// A pool that never connects, for tests that need no database.
pub fn unreachable_db() -> PgPool {
    PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(500))
        .connect_lazy("postgres://bookclub:bookclub@127.0.0.1:1/bookclub")
        .expect("valid URL")
}

/// A response with its body read.
pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl TestResponse {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|error| {
            panic!(
                "body is not JSON ({error}): {}",
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    /// Asserts the status and the JSON `{"detail": …}` body.
    pub fn assert_detail(&self, status: u16, detail: &str) -> &Self {
        assert_eq!(self.status.as_u16(), status, "body: {}", self.text());
        assert_eq!(self.json(), serde_json::json!({ "detail": detail }));
        self
    }
}

/// Sends one request through `app`.
pub async fn send(app: Router, request: Request<Body>) -> TestResponse {
    let response = app.oneshot(request).await.expect("infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    TestResponse {
        status,
        headers,
        body,
    }
}

/// `GET path` through `app`.
pub async fn get(app: Router, path: &str) -> TestResponse {
    send(app, Request::get(path).body(Body::empty()).unwrap()).await
}

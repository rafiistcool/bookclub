//! Bookclub server: the HTTP API for the hosted multi-club service.

pub mod config;
mod health;

use axum::Router;
use axum::routing::get;
use sqlx::PgPool;

/// Shared state handed to every request handler.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
}

/// Builds the router with every API route mounted under `/api`.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health::health))
        .with_state(state)
}

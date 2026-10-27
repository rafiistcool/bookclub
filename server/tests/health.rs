use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use bookclub_server::{AppState, app};
use http_body_util::BodyExt;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

async fn get_health(state: AppState) -> (StatusCode, serde_json::Value) {
    let response = app(state)
        .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[sqlx::test]
async fn health_is_ok_when_the_database_answers(db: PgPool) {
    let (status, body) = get_health(AppState { db }).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!({ "status": "ok" }));
}

#[tokio::test]
async fn health_is_unavailable_when_the_database_is_down() {
    // Port 1 on loopback refuses connections, so the pool can never connect.
    let db = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(500))
        .connect_lazy("postgres://bookclub:bookclub@127.0.0.1:1/bookclub")
        .unwrap();

    let (status, body) = get_health(AppState { db }).await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, serde_json::json!({ "status": "unavailable" }));
}

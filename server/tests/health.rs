mod common;

use axum::http::StatusCode;
use bookclub_server::app;
use common::{get, test_state, unreachable_db};
use sqlx::PgPool;

#[sqlx::test]
async fn health_is_ok_when_the_database_answers(db: PgPool) {
    let res = get(app(test_state(db)), "/api/health").await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json(), serde_json::json!({ "status": "ok" }));
}

#[tokio::test]
async fn health_is_unavailable_when_the_database_is_down() {
    // Port 1 on loopback refuses connections, so the pool can never connect.
    let res = get(app(test_state(unreachable_db())), "/api/health").await;

    assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(res.json(), serde_json::json!({ "status": "unavailable" }));
}

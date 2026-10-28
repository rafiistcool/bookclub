use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;

const DB_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Serialize, ToSchema)]
pub struct Health {
    /// `"ok"` or `"unavailable"`.
    status: &'static str,
}

/// `GET /api/health`.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(health))
}

/// Reports whether the server can reach its database. Uptime monitors and
/// the deploy check use this, so it answers within a few seconds either way.
#[utoipa::path(
    get,
    path = "/api/health",
    operation_id = "platform_health",
    tag = "platform",
    responses(
        (status = 200, body = Health, description = "The database answers"),
        (status = 503, body = Health, description = "The database does not answer within 2 s")
    )
)]
pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<Health>) {
    let check = sqlx::query("SELECT 1").execute(&state.db);
    match tokio::time::timeout(DB_CHECK_TIMEOUT, check).await {
        Ok(Ok(_)) => (StatusCode::OK, Json(Health { status: "ok" })),
        Ok(Err(error)) => {
            tracing::warn!(%error, "health check: database query failed");
            unavailable()
        }
        Err(_) => {
            tracing::warn!("health check: database query timed out");
            unavailable()
        }
    }
}

fn unavailable() -> (StatusCode, Json<Health>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(Health {
            status: "unavailable",
        }),
    )
}

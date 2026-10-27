//! The home and club activity feeds (`GET /api/activity`, `GET /api/clubs/{slug}/activity`).
//! Owned by CLUB from Wave 2 on (docs/phase1/conventions.md §6.4).

use utoipa_axum::router::OpenApiRouter;

use crate::AppState;

/// The feed routes.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

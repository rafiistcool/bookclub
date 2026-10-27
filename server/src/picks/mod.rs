//! Club picks, meetings and milestones (#45, owner CLUB).

use utoipa_axum::router::OpenApiRouter;

use crate::AppState;

/// The pick routes.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

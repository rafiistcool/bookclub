//! The next-up vote (#45, owner CLUB).

use utoipa_axum::router::OpenApiRouter;

use crate::AppState;

/// The vote routes.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

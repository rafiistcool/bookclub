//! Books two or more club members want to read (#45, owner CLUB).

use utoipa_axum::router::OpenApiRouter;

use crate::AppState;

/// The overlap route.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

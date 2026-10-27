//! Diary entries and pick discussions in one table (#44b, owner POSTS): threads, reactions and
//! feeds.

use utoipa_axum::router::OpenApiRouter;

use crate::AppState;

/// The post routes.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

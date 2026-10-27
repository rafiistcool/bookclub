//! The personal shelf (#44a, owner SHELF): shelf, favourites, physical layout, the Goodreads
//! import, quotes, search history, personal and club stats.

pub mod goodreads;

use utoipa_axum::router::OpenApiRouter;

use crate::AppState;

/// The shelf routes.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

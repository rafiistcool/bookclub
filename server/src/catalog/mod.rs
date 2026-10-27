//! The catalog (#41, owner CAT): search, works, editions, ISBN lookups, user-added books, edit
//! requests and merges. `shared/` holds the frozen interfaces every package uses.

pub mod service;
pub mod shared;

use utoipa_axum::router::OpenApiRouter;

pub use service::CatalogService;

use crate::AppState;

/// Catalog routes under `/api/books`.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

/// Instance-admin routes; every path starts with `/api/admin/`.
pub fn admin_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

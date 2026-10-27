//! Media (#43, owner MED): `/media/{public_id}/{variant}`, covers, avatars, reports and
//! takedowns. Wave 1 registers the module with empty routers.

pub mod service;
pub mod shared;

use utoipa_axum::router::OpenApiRouter;

pub use service::MediaService;

use crate::AppState;

/// Media routes outside `/api/admin`.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

/// Instance-admin routes; every path starts with `/api/admin/`.
pub fn admin_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

//! Closed-beta sign-up invites minted by an instance admin (`POST/GET /api/admin/invites`,
//! endpoint-map §3). Wave 1 step 4 fills them in.

use utoipa_axum::router::OpenApiRouter;

use crate::AppState;

/// Instance-admin routes; every path starts with `/api/admin/`.
pub fn admin_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

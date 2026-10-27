//! Clubs (#40): clubs, memberships, roles, invites, join, leave and transfer
//! (docs/phase1/conventions.md §5.3, endpoint-map §3).

pub mod extract;
pub mod invites;

use utoipa_axum::router::OpenApiRouter;

use crate::AppState;

/// The club routes. Wave 1 step 4 fills them in.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

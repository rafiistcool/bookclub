//! The DNB OAI-PMH harvester (#42, owner HAR) and `GET /api/books/new-releases`.

use tokio_util::sync::CancellationToken;
use utoipa_axum::router::OpenApiRouter;

use crate::AppState;

/// The harvest routes.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}

/// Harvests every `DNB_HARVEST_SETS` set every `DNB_HARVEST_INTERVAL_MINUTES` until `shutdown`.
/// Started by `jobs::spawn_all` only when `DNB_HARVEST` is set. W1 stub: returns at once.
pub async fn run_forever(_state: AppState, _shutdown: CancellationToken) {}

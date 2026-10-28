//! Activity events: `record` (W1, frozen) and the feeds (CLUB).

pub mod feed;
pub mod record;

use utoipa_axum::router::OpenApiRouter;

pub use record::{EventKind, NewEvent, PRIVATE_KEY, record, seal_finished_ratings};

use crate::AppState;

/// The activity routes, all in `feed`.
pub fn routes() -> OpenApiRouter<AppState> {
    feed::routes()
}

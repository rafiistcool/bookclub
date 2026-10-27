//! MED-owned media state: fetch coalescing, image limits and the cover prefetch queue
//! (docs/phase1/conventions.md §3, §8). Wave 1 stub; MED fills it in.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::AppState;
use crate::config::Config;
use crate::media::shared::MediaStore;
use crate::outbound::Outbound;

/// MED's private state, kept in `AppState::media_service`. MED adds fields freely.
pub struct MediaService {}

impl MediaService {
    /// Frozen constructor.
    pub fn new(_config: &Config, _store: Arc<dyn MediaStore>, _outbound: Arc<Outbound>) -> Self {
        Self {}
    }

    /// Queue Open Library covers for background fetching (bounded queue, drops when full).
    /// Called by CAT after it persisted search, trending or subject hits. W1 stub: no-op.
    pub fn prefetch(&self, _media_ids: &[i64]) {}
}

/// The prefetch worker that drains [`MediaService::prefetch`]'s queue until `shutdown`.
/// Started by `jobs::spawn_all`, never in tests. W1 stub: returns at once.
pub async fn run_prefetch(_state: AppState, _shutdown: CancellationToken) {}

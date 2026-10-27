//! The Goodreads CSV import job (owner SHELF).

use crate::AppState;

/// Re-queues `queued` and `running` import jobs after a restart. Called once at startup by
/// `jobs::spawn_all`. W1 stub: no-op.
pub async fn resume(_state: AppState) {}

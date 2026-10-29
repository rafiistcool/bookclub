//! Dead pictures (docs/phase2/notes/cover.md §1.1): an Open Library picture whose fetch failed
//! is fetched again a day later; a 404 or 410 then marks it **gone**, but only while Open
//! Library's cover server still serves a picture we know (the probe). A gone picture is cleared
//! from every work and edition and never attached again; the works it left empty are searched.
//!
//! A refused redirect, an oversized or undecodable picture stays failing: those can come from
//! our configuration or a format change, so they are never cleared on their own.

use sqlx::{Connection, PgConnection, PgPool};

use super::search::Pacer;
use crate::AppState;
use crate::catalog::shared::ingest;
use crate::media::service::RecheckOutcome;
use crate::outbound::RetryPolicy;

/// What a dead check did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeadReport {
    /// Failing rows fetched again.
    pub rechecked: u64,
    /// The picture loaded this time and is stored.
    pub stored: u64,
    /// Confirmed gone and cleared.
    pub gone: u64,
    /// Still unusable (a refused redirect, too large, undecodable).
    pub unusable: u64,
    /// The bytes were a taken-down picture's.
    pub removed: u64,
    /// A 404 or 410 while the probe failed: left failing for another day.
    pub unconfirmed: u64,
    /// Open Library was throttled or failing: the check stopped.
    pub stopped: bool,
}

/// The failing Open Library rows a work or an edition names whose failure is at least a day
/// old: in-use books first, then the oldest failure.
pub async fn due_rows(conn: &mut PgConnection, limit: i64) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar!(
        r#"SELECT m.id FROM media m
           WHERE m.storage_key IS NULL AND m.fetch_failed_at IS NOT NULL AND m.removed_at IS NULL
             AND m.ol_gone_at IS NULL AND m.source = 'openlibrary'
             AND m.fetch_failed_at <= now() - interval '24 hours'
             AND (EXISTS (SELECT 1 FROM works w WHERE w.cover_media_id = m.id)
                  OR EXISTS (SELECT 1 FROM editions e WHERE e.cover_media_id = m.id))
           ORDER BY (EXISTS (SELECT 1 FROM works w
                             WHERE w.cover_media_id = m.id AND bc_cover_in_use(w.id))
                     OR EXISTS (SELECT 1 FROM editions e
                                WHERE e.cover_media_id = m.id AND bc_cover_in_use(e.work_id)))
                        DESC,
                    m.fetch_failed_at, m.id
           LIMIT $1"#,
        limit
    )
    .fetch_all(conn)
    .await
}

/// The failing Open Library pictures of one work and its editions (an admin's "Search now"
/// re-checks them at once, without the day's wait).
pub async fn failing_rows_of(conn: &mut PgConnection, work_id: i64) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar!(
        r#"SELECT DISTINCT m.id FROM media m
           WHERE m.storage_key IS NULL AND m.fetch_failed_at IS NOT NULL AND m.removed_at IS NULL
             AND m.ol_gone_at IS NULL AND m.source = 'openlibrary'
             AND (m.id = (SELECT cover_media_id FROM works WHERE id = $1)
                  OR m.id IN (SELECT cover_media_id FROM editions WHERE work_id = $1))
           ORDER BY m.id"#,
        work_id
    )
    .fetch_all(conn)
    .await
}

/// Fetches each row again (paced) and settles it: stored, gone (once the run's probe passed),
/// still unusable. Stops at the first throttled or failed fetch. `probe` carries the run's
/// probe result, asked once before the first row is marked gone.
pub async fn recheck(
    state: &AppState,
    rows: &[i64],
    pacer: &mut Pacer,
    probe: &mut Option<bool>,
    policy: RetryPolicy,
) -> sqlx::Result<DeadReport> {
    let mut report = DeadReport::default();
    for &media_id in rows {
        if !pacer.wait().await {
            report.stopped = true;
            break;
        }
        report.rechecked += 1;
        match state
            .media_service
            .recheck_cover(&state.db, media_id, policy)
            .await
        {
            RecheckOutcome::Stored => report.stored += 1,
            RecheckOutcome::Removed => report.removed += 1,
            RecheckOutcome::Unusable => report.unusable += 1,
            RecheckOutcome::NotFetchable => {}
            RecheckOutcome::Missing => {
                if probe.is_none() {
                    if !pacer.wait().await {
                        report.stopped = true;
                        break;
                    }
                    *probe = Some(state.media_service.probe_covers(&state.db, policy).await);
                }
                if *probe == Some(true) {
                    mark_gone(&state.db, media_id).await?;
                    report.gone += 1;
                } else {
                    // The cover server is broken or has moved: nothing is marked gone, and the
                    // row waits another day.
                    sqlx::query!(
                        "UPDATE media SET fetch_failed_at = now()
                         WHERE id = $1 AND storage_key IS NULL AND removed_at IS NULL",
                        media_id
                    )
                    .execute(&state.db)
                    .await?;
                    report.unconfirmed += 1;
                }
            }
            RecheckOutcome::Busy | RecheckOutcome::Failed => {
                report.stopped = true;
                break;
            }
        }
    }
    Ok(report)
}

/// Marks a failing row gone and clears every reference to it, in one transaction: the media row
/// `FOR UPDATE` first, then works, then editions (the takedown's order). A row that was stored,
/// removed or marked meanwhile is left alone.
pub async fn mark_gone(db: &PgPool, media_id: i64) -> sqlx::Result<bool> {
    let mut conn = db.acquire().await?;
    let mut tx = conn.begin().await?;
    let marked = sqlx::query_scalar!(
        "UPDATE media SET ol_gone_at = now()
         WHERE id = $1 AND ol_cover_id IS NOT NULL AND storage_key IS NULL
           AND removed_at IS NULL AND ol_gone_at IS NULL
         RETURNING id",
        media_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    if marked {
        let works = ingest::clear_gone_media(&mut tx, media_id).await?;
        tracing::info!(
            media_id,
            works = works.len(),
            "an Open Library cover is gone"
        );
    }
    tx.commit().await?;
    Ok(marked)
}

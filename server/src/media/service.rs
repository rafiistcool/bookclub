//! MED-owned media state (docs/phase1/conventions.md §3, §8): the picture pipeline's
//! concurrency limit, the Open Library cover fetch with its coalescing, and the prefetch queue
//! with its background worker.
//!
//! **Open Library covers.** A cover row (`ingest::ol_cover_media`) has no bytes until someone
//! needs them: `GET /media/{public_id}/{variant}`, the spine-colour backfill or the prefetch
//! worker calls [`MediaService::ensure_cover`]. It fetches
//! `{OPENLIBRARY_COVERS_URL}/b/id/{cover id}-L.jpg?default=false` through `outbound::get_bytes`
//! (the `ol_covers` throttle, checked redirects, 1.5 MB at most), converts the image to the `l`
//! and `s` WebP variants, stores them, samples the spine colour and fills the row. Concurrent
//! calls for one media id share one fetch, which runs in its own task, so a caller that goes
//! away does not cancel it for the others.
//!
//! **Outcomes** (docs/phase1/spec/shelf.md §6 risk 15): a refused redirect, a body over 1.5 MB,
//! Open Library's 404 or 410, or bytes that are no usable picture settle as "no cover":
//! `fetch_failed_at` is set (not retried for 7 days) and the spine colour becomes `''`. Throttling
//! is [`CoverOutcome::Busy`]; transport errors, timeouts and other statuses are
//! [`CoverOutcome::Failed`] and change nothing, so the next request tries again.
//!
//! **Takedowns** always win: `removed_at` is read first, and the row is only filled while it is
//! not removed. Bytes stored by a fetch that lost the race against a takedown are deleted again.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use sqlx::PgPool;
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::color::dominant_color;
use super::pipeline::{self, PictureError, PictureKind, Processed};
use crate::AppState;
use crate::config::Config;
use crate::media::shared::MediaStore;
use crate::media::shared::urls::{object_key, storage_prefix};
use crate::outbound::{Outbound, RetryPolicy, UpstreamError, get_bytes};

/// Ids the prefetch queue holds at most; further ids are dropped until the worker catches up.
pub const PREFETCH_QUEUE: usize = 500;
/// Pictures decoded at the same time (each can take a few hundred MB at 25 Mpx).
const PICTURES_AT_ONCE: usize = 2;
/// A cover Open Library had no picture for is not asked for again within this time.
pub const FETCH_FAILED_RETRY_DAYS: i32 = 7;
/// The prefetch worker waits longer for a throttle slot than a request does.
pub const PREFETCH_POLICY: RetryPolicy = RetryPolicy {
    max_wait: Duration::from_secs(30),
    ..RetryPolicy::MEDIA
};
/// The worker's pause when Open Library is cooling down and says nothing about how long.
const BUSY_PAUSE: Duration = Duration::from_secs(5);

/// What [`MediaService::ensure_cover`] found or did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverOutcome {
    /// The bytes are in the store (they were, or the fetch stored them).
    Stored,
    /// The row was taken down; it is never fetched.
    Removed,
    /// No picture to be had: an unknown row, a row without an Open Library cover id, a recent
    /// failure, or a fetch that settled as "no cover" now.
    NoCover,
    /// Open Library is throttled or cooling down.
    Busy,
    /// A transport error, a timeout or an unexpected status; nothing was recorded.
    Failed,
}

/// What one Open Library cover request gave.
#[derive(Debug)]
pub enum CoverFetch {
    /// The image bytes (at most 1.5 MB).
    Image(Bytes),
    /// Settled: there is no usable cover (404, 410, a refused redirect, too large).
    NoCover,
    /// Throttled or cooling down.
    Busy,
    /// Try again later.
    Failed,
}

type InFlight = Shared<BoxFuture<'static, CoverOutcome>>;

/// MED's private state, kept in `AppState::media_service`.
pub struct MediaService {
    inner: Arc<Inner>,
    queue: mpsc::Sender<i64>,
    /// Taken by the running [`run_prefetch`] and put back when it stops.
    queue_rx: Mutex<Option<mpsc::Receiver<i64>>>,
}

struct Inner {
    store: Arc<dyn MediaStore>,
    outbound: Arc<Outbound>,
    /// The pipeline's permits. Each job moves an owned permit into its blocking task
    /// ([`with_picture_permit`]), so a request that goes away cannot free it mid-decode.
    pictures: Arc<Semaphore>,
    /// Cover fetches in flight by media id, with the generation that started them.
    in_flight: Mutex<HashMap<i64, (u64, InFlight)>>,
    generation: AtomicU64,
}

impl MediaService {
    /// Frozen constructor.
    pub fn new(_config: &Config, store: Arc<dyn MediaStore>, outbound: Arc<Outbound>) -> Self {
        let (queue, queue_rx) = mpsc::channel(PREFETCH_QUEUE);
        Self {
            inner: Arc::new(Inner {
                store,
                outbound,
                pictures: Arc::new(Semaphore::new(PICTURES_AT_ONCE)),
                in_flight: Mutex::new(HashMap::new()),
                generation: AtomicU64::new(0),
            }),
            queue,
            queue_rx: Mutex::new(Some(queue_rx)),
        }
    }

    /// Queue Open Library covers for background fetching (bounded queue, drops when full).
    /// Called by CAT after it persisted search, trending or subject hits.
    pub fn prefetch(&self, media_ids: &[i64]) {
        for (queued, &media_id) in media_ids.iter().enumerate() {
            if self.queue.try_send(media_id).is_err() {
                tracing::debug!(
                    dropped = media_ids.len() - queued,
                    "the cover prefetch queue is full"
                );
                return;
            }
        }
    }

    /// Runs the picture pipeline on the blocking pool, at most two pictures at a time.
    pub async fn process(&self, kind: PictureKind, data: Bytes) -> Result<Processed, PictureError> {
        self.inner.process(kind, data).await
    }

    /// The spine colour of stored picture bytes (`color::dominant_color`; `""` when none is
    /// usable), sampled on the blocking pool under the pipeline's two-at-a-time limit.
    pub async fn dominant_color(&self, data: Bytes) -> anyhow::Result<String> {
        let color = with_picture_permit(&self.inner.pictures, move || dominant_color(&data))
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(color.unwrap_or_default())
    }

    /// Makes sure the bytes of an Open Library cover row are stored, fetching them once if
    /// needed. Concurrent calls for one row share one fetch (with the first caller's policy).
    pub async fn ensure_cover(
        &self,
        db: &PgPool,
        media_id: i64,
        policy: RetryPolicy,
    ) -> CoverOutcome {
        let fetch = {
            let mut in_flight = self.inner.in_flight.lock().unwrap();
            match in_flight.get(&media_id) {
                Some((_, fetch)) => fetch.clone(),
                None => {
                    let generation = self.inner.generation.fetch_add(1, Ordering::Relaxed);
                    let inner = self.inner.clone();
                    let db = db.clone();
                    let task = tokio::spawn(async move {
                        let outcome = inner.fetch_and_store(&db, media_id, &policy).await;
                        inner.finished(media_id, generation);
                        outcome
                    });
                    let fetch = async move {
                        task.await.unwrap_or_else(|error| {
                            tracing::error!(media_id, %error, "a cover fetch task failed");
                            CoverOutcome::Failed
                        })
                    }
                    .boxed()
                    .shared();
                    in_flight.insert(media_id, (generation, fetch.clone()));
                    fetch
                }
            }
        };
        fetch.await
    }
}

impl Inner {
    async fn process(&self, kind: PictureKind, data: Bytes) -> Result<Processed, PictureError> {
        with_picture_permit(&self.pictures, move || pipeline::process(kind, &data))
            .await
            .unwrap_or_else(|error| Err(PictureError::Encode(error)))
    }

    fn finished(&self, media_id: i64, generation: u64) {
        let mut in_flight = self.in_flight.lock().unwrap();
        if in_flight
            .get(&media_id)
            .is_some_and(|(started, _)| *started == generation)
        {
            in_flight.remove(&media_id);
        }
    }

    async fn fetch_and_store(
        &self,
        db: &PgPool,
        media_id: i64,
        policy: &RetryPolicy,
    ) -> CoverOutcome {
        match self.try_fetch_and_store(db, media_id, policy).await {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(media_id, error = %error, "storing an Open Library cover failed");
                CoverOutcome::Failed
            }
        }
    }

    async fn try_fetch_and_store(
        &self,
        db: &PgPool,
        media_id: i64,
        policy: &RetryPolicy,
    ) -> anyhow::Result<CoverOutcome> {
        let Some(row) = sqlx::query!(
            r#"SELECT public_id, ol_cover_id, storage_key, removed_at IS NOT NULL AS "removed!",
                      COALESCE(fetch_failed_at > now() - make_interval(days => $2), false)
                          AS "recently_failed!"
               FROM media WHERE id = $1 AND kind = 'cover'"#,
            media_id,
            FETCH_FAILED_RETRY_DAYS
        )
        .fetch_optional(db)
        .await?
        else {
            return Ok(CoverOutcome::NoCover);
        };
        if row.removed {
            return Ok(CoverOutcome::Removed);
        }
        if row.storage_key.is_some() {
            return Ok(CoverOutcome::Stored);
        }
        let Some(ol_cover_id) = row.ol_cover_id else {
            return Ok(CoverOutcome::NoCover);
        };
        if row.recently_failed {
            return Ok(CoverOutcome::NoCover);
        }
        let bytes = match fetch_ol_cover(&self.outbound, ol_cover_id, policy).await {
            CoverFetch::Image(bytes) => bytes,
            CoverFetch::NoCover => {
                settle_without_cover(db, media_id).await?;
                return Ok(CoverOutcome::NoCover);
            }
            CoverFetch::Busy => return Ok(CoverOutcome::Busy),
            CoverFetch::Failed => return Ok(CoverOutcome::Failed),
        };
        let processed = match self.process(PictureKind::Cover, bytes).await {
            Ok(processed) => processed,
            Err(PictureError::Encode(error)) => anyhow::bail!("cannot encode a cover: {error}"),
            Err(refused) => {
                tracing::info!(
                    media_id,
                    ol_cover_id,
                    ?refused,
                    "Open Library sent no usable cover"
                );
                settle_without_cover(db, media_id).await?;
                return Ok(CoverOutcome::NoCover);
            }
        };
        store_variants(
            self.store.as_ref(),
            PictureKind::Cover,
            &row.public_id,
            &processed,
        )
        .await?;
        let prefix = storage_prefix(PictureKind::Cover.as_str(), &row.public_id);
        let variants: Vec<String> = processed
            .variants
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .collect();
        let filled = sqlx::query!(
            "UPDATE media SET storage_key = $2, content_type = 'image/webp', variants = $3,
                    width = $4, height = $5, byte_size = $6, sha256 = $7, dominant_color = $8,
                    fetch_failed_at = NULL
             WHERE id = $1 AND removed_at IS NULL AND storage_key IS NULL",
            media_id,
            prefix,
            &variants,
            processed.width as i32,
            processed.height as i32,
            processed.byte_size(),
            &processed.sha256[..],
            processed.dominant_color
        )
        .execute(db)
        .await?
        .rows_affected()
            == 1;
        if filled {
            return Ok(CoverOutcome::Stored);
        }
        let removed = sqlx::query_scalar!(
            r#"SELECT removed_at IS NOT NULL AS "removed!" FROM media WHERE id = $1"#,
            media_id
        )
        .fetch_optional(db)
        .await?
        .unwrap_or(true);
        if removed {
            // A takedown came first: the bytes must not stay.
            if let Err(error) = self.store.delete_prefix(&prefix).await {
                tracing::warn!(prefix, %error, "cannot delete the bytes of a removed cover");
            }
            return Ok(CoverOutcome::Removed);
        }
        // Stored meanwhile under the same keys (another server instance).
        Ok(CoverOutcome::Stored)
    }
}

/// Puts every variant under `object_key(kind, public_id, variant)`.
pub async fn store_variants(
    store: &dyn MediaStore,
    kind: PictureKind,
    public_id: &str,
    processed: &Processed,
) -> anyhow::Result<()> {
    for (variant, bytes) in &processed.variants {
        store
            .put(
                &object_key(kind.as_str(), public_id, variant),
                Bytes::copy_from_slice(bytes),
                "image/webp",
            )
            .await?;
    }
    Ok(())
}

/// SHA-256 of each of `data`, computed on the blocking pool.
pub async fn sha256_all(data: Vec<Bytes>) -> anyhow::Result<Vec<[u8; 32]>> {
    let hashes = tokio::task::spawn_blocking(move || {
        data.iter()
            .map(|bytes| pipeline::sha256(bytes))
            .collect::<Vec<_>>()
    })
    .await?;
    Ok(hashes)
}

/// Records that Open Library has no usable picture for the row: not asked again for 7 days,
/// and no spine colour (`''`) until a later fetch succeeds.
async fn settle_without_cover(db: &PgPool, media_id: i64) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE media SET fetch_failed_at = now(), dominant_color = COALESCE(dominant_color, '')
         WHERE id = $1 AND removed_at IS NULL AND storage_key IS NULL",
        media_id
    )
    .execute(db)
    .await?;
    Ok(())
}

/// The large Open Library cover of a cover id: `{base}/b/id/{id}-L.jpg?default=false` (Open
/// Library answers 404 instead of a placeholder pixel for a missing cover).
pub fn ol_cover_url(base: &Url, ol_cover_id: i64) -> Url {
    let mut url = base.clone();
    let path = format!(
        "{}/b/id/{ol_cover_id}-L.jpg",
        base.path().trim_end_matches('/')
    );
    url.set_path(&path);
    url.set_query(Some("default=false"));
    url
}

/// One cover request through the `ol_covers` throttle, sorted into the outcomes of
/// docs/phase1/spec/shelf.md §6 risk 15.
pub async fn fetch_ol_cover(
    outbound: &Outbound,
    ol_cover_id: i64,
    policy: &RetryPolicy,
) -> CoverFetch {
    let url = ol_cover_url(&outbound.urls.openlibrary_covers, ol_cover_id);
    match get_bytes(outbound, &outbound.ol_covers, url, policy).await {
        Ok(upstream) => CoverFetch::Image(upstream.body),
        Err(UpstreamError::Redirect(_) | UpstreamError::TooLarge(_)) => CoverFetch::NoCover,
        Err(UpstreamError::Status(404 | 410)) => CoverFetch::NoCover,
        Err(UpstreamError::Throttled) => CoverFetch::Busy,
        Err(UpstreamError::Status(_) | UpstreamError::Transport(_) | UpstreamError::Decode(_)) => {
            CoverFetch::Failed
        }
    }
}

/// The prefetch worker that drains [`MediaService::prefetch`]'s queue until `shutdown`, one
/// cover at a time through the `ol_covers` throttle. Started by `jobs::spawn_all`, never in
/// tests (tests call it directly).
pub async fn run_prefetch(state: AppState, shutdown: CancellationToken) {
    let service = state.media_service.clone();
    let Some(mut queue) = service.queue_rx.lock().unwrap().take() else {
        tracing::warn!("the cover prefetch worker runs already");
        return;
    };
    tracing::info!("cover prefetch worker started");
    loop {
        let media_id = tokio::select! {
            () = shutdown.cancelled() => break,
            next = queue.recv() => match next {
                Some(media_id) => media_id,
                None => break,
            },
        };
        let outcome = tokio::select! {
            () = shutdown.cancelled() => break,
            outcome = service.ensure_cover(&state.db, media_id, PREFETCH_POLICY) => outcome,
        };
        if outcome == CoverOutcome::Busy {
            let pause = state
                .outbound
                .ol_covers
                .cooldown_remaining()
                .unwrap_or(BUSY_PAUSE);
            tokio::select! {
                () = shutdown.cancelled() => break,
                () = tokio::time::sleep(pause) => {}
            }
        }
    }
    *service.queue_rx.lock().unwrap() = Some(queue);
    tracing::info!("cover prefetch worker stopped");
}

/// Runs `job` on the blocking pool under one of `permits`. The owned permit moves into the
/// blocking task: when the request that started the job goes away (its client disconnected),
/// the decode still holds the permit until it ends, so cancelled uploads cannot start more
/// decodes than the pipeline allows (security review 2026-10-28 #4).
async fn with_picture_permit<T: Send + 'static>(
    permits: &Arc<Semaphore>,
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let permit = permits
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "the picture pipeline is closed".to_owned())?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        job()
    })
    .await
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::shared::MemMediaStore;

    #[tokio::test]
    async fn a_cancelled_caller_leaves_the_permit_with_the_running_job() {
        let permits = Arc::new(Semaphore::new(1));
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let caller = tokio::spawn({
            let permits = permits.clone();
            async move {
                with_picture_permit(&permits, move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                })
                .await
            }
        });
        tokio::task::spawn_blocking(move || started_rx.recv().unwrap())
            .await
            .unwrap();
        caller.abort();
        let _ = caller.await;
        assert_eq!(permits.available_permits(), 0, "the decode still runs");
        release_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while permits.available_permits() == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the permit comes back when the job ends");
    }

    fn service() -> MediaService {
        let config = Config::from_lookup(|key| match key {
            "DATABASE_URL" => Some("postgres://unused".to_owned()),
            "DEV_MODE" => Some("true".to_owned()),
            _ => None,
        })
        .unwrap();
        let outbound = Arc::new(Outbound::new(&config).unwrap());
        MediaService::new(&config, Arc::new(MemMediaStore::new()), outbound)
    }

    #[test]
    fn the_prefetch_queue_drops_ids_when_full() {
        let service = service();
        let ids: Vec<i64> = (1..=600).collect();
        service.prefetch(&ids);
        service.prefetch(&[601]);
        let mut queue = service.queue_rx.lock().unwrap().take().unwrap();
        assert_eq!(queue.len(), PREFETCH_QUEUE);
        let mut seen = Vec::new();
        while let Ok(id) = queue.try_recv() {
            seen.push(id);
        }
        assert_eq!(seen, (1..=500).collect::<Vec<i64>>());
    }

    #[test]
    fn cover_urls_ask_for_the_large_image_without_a_placeholder() {
        let base = Url::parse("https://covers.openlibrary.org").unwrap();
        assert_eq!(
            ol_cover_url(&base, 111).as_str(),
            "https://covers.openlibrary.org/b/id/111-L.jpg?default=false"
        );
        let base = Url::parse("http://127.0.0.1:4000/").unwrap();
        assert_eq!(
            ol_cover_url(&base, 7).as_str(),
            "http://127.0.0.1:4000/b/id/7-L.jpg?default=false"
        );
    }
}

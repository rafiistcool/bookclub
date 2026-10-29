//! Helpers the `tests/cover_*.rs` files share (included with `#[path]`): Open Library answers,
//! works in use, covers, the job with no pace, and a lookup that does something during the
//! remote step.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bookclub_server::catalog::covers::job::{self, JobSettings, RunReport};
use bookclub_server::catalog::covers::search::{Context, Outcome, Pacer, search_one};
use bookclub_server::catalog::shared::ingest::{
    self, AuthorRecord, CoverTarget, Mode, Source, WorkRecord,
};
use bookclub_server::catalog::shared::isbn::Isbn13;
use bookclub_server::catalog::shared::lookup::{
    CatalogLookup, CoverAnswer, CoverDoc, CoverEdition, CoverQuery, LookupError, Resolved,
};
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageEncoder, Rgb, RgbImage};
use sqlx::{AssertSqlSafe, PgConnection, PgPool};
use tokio::sync::Barrier;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use crate::common::TestApp;

/// The job's settings without the 2-second pace.
pub fn settings() -> JobSettings {
    JobSettings {
        pace: Duration::ZERO,
        ..JobSettings::default()
    }
}

/// One run of the background job.
pub async fn run_job(app: &TestApp) -> RunReport {
    run_job_with(app, &settings()).await
}

pub async fn run_job_with(app: &TestApp, settings: &JobSettings) -> RunReport {
    let mut conn = app.db.acquire().await.unwrap();
    job::run(&app.state, &mut conn, &CancellationToken::new(), settings)
        .await
        .unwrap()
}

/// An Open Library work of a cover search answer: `OL{id}W` with `edition_count` editions.
pub fn doc(
    id: u32,
    title: &str,
    subtitle: &str,
    author: &str,
    cover: Option<i64>,
    editions: i32,
) -> CoverDoc {
    CoverDoc {
        ol_work_id: format!("OL{id}W"),
        title: title.into(),
        subtitle: subtitle.into(),
        authors: vec![author.into()],
        cover_id: cover,
        edition_count: Some(editions),
        first_publish_year: Some(2001),
        languages: vec!["eng".into()],
        edition: None,
    }
}

/// `doc` with its best edition `OL{id}M`.
pub fn with_edition(
    mut doc: CoverDoc,
    id: u32,
    title: &str,
    subtitle: &str,
    cover: Option<i64>,
    languages: &[&str],
) -> CoverDoc {
    doc.edition = Some(CoverEdition {
        ol_edition_id: format!("OL{id}M"),
        title: title.into(),
        subtitle: subtitle.into(),
        cover_id: cover,
        languages: languages.iter().map(|l| l.to_string()).collect(),
    });
    doc
}

pub fn answer(docs: Vec<CoverDoc>) -> CoverAnswer {
    CoverAnswer {
        num_found: docs.len() as i64,
        docs,
        refused: None,
    }
}

static NEXT_BOOK: AtomicU64 = AtomicU64::new(1);

/// Marks a work as used by readers: a legacy key names it.
pub async fn use_work(app: &TestApp, work_id: i64) {
    let n = NEXT_BOOK.fetch_add(1, Ordering::Relaxed) as i64 * 1000 + work_id;
    app.exec(&format!(
        "INSERT INTO legacy_work_keys (legacy_key, legacy_book_id, work_id, match)
         VALUES ('/works/GBuse{n}', {n}, {work_id}, 'google_volume')"
    ))
    .await;
}

/// A bare-facts legacy work (an imported Google row) with `authors`, used by readers.
pub async fn legacy_work(app: &TestApp, title: &str, authors: &str) -> i64 {
    let record = WorkRecord {
        author_names: authors.to_owned(),
        authors: authors
            .split(", ")
            .filter(|name| !name.is_empty())
            .map(AuthorRecord::named)
            .collect(),
        ..WorkRecord::new(Source::Legacy, title)
    };
    let mut conn = app.db.acquire().await.unwrap();
    let id = ingest::upsert_work(&mut conn, &record, Mode::Refresh)
        .await
        .unwrap();
    use_work(app, id).await;
    id
}

/// The media row of an Open Library cover id (created when missing).
pub async fn ol_media(app: &TestApp, ol_cover_id: i64) -> i64 {
    let mut conn = app.db.acquire().await.unwrap();
    ingest::ol_cover_media(&mut conn, ol_cover_id)
        .await
        .unwrap()
        .expect("a usable cover row")
}

static NEXT_UPLOAD: AtomicU64 = AtomicU64::new(1);

/// A stored upload (a reader's photo) as a media row.
pub async fn upload_media(app: &TestApp) -> i64 {
    let n = NEXT_UPLOAD.fetch_add(1, Ordering::Relaxed);
    let public_id = format!("u{n:0>25}");
    app.scalar_i64(&format!(
        "INSERT INTO media (public_id, kind, source, storage_key, content_type, variants,
                            width, height, byte_size)
         VALUES ('{public_id}', 'cover', 'upload', 'cover/{public_id}', 'image/webp', '{{l,s}}',
                 10, 15, 100)
         RETURNING id"
    ))
    .await
}

/// Sets a cover through ingest.
pub async fn set_cover(app: &TestApp, target: CoverTarget, media_id: i64, by: Source) {
    let mut conn = app.db.acquire().await.unwrap();
    assert!(
        ingest::set_cover(&mut conn, target, media_id, by, false)
            .await
            .unwrap(),
        "the cover is set"
    );
}

/// The work's cover as (Open Library cover id or 0 for an upload, provenance).
pub async fn cover_of(app: &TestApp, work_id: i64) -> Option<(i64, String)> {
    sqlx::query_as::<_, (Option<i64>, Option<String>)>(
        "SELECT m.ol_cover_id, w.field_sources ->> 'cover_media_id'
         FROM works w JOIN media m ON m.id = w.cover_media_id WHERE w.id = $1",
    )
    .bind(work_id)
    .fetch_optional(&app.db)
    .await
    .unwrap()
    .map(|(cover, by)| (cover.unwrap_or(0), by.unwrap_or_default()))
}

/// A `cover_search` row, as the tests look at it.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchRow {
    pub outcome: Option<String>,
    pub misses: i32,
    pub failures: i32,
    /// Hours until `next_search_at`, rounded; `None` when due.
    pub next_hours: Option<i64>,
    pub searched: bool,
    pub takedown: bool,
    pub last_error: Option<String>,
    pub attached_cover: Option<i64>,
}

pub async fn search_row(app: &TestApp, work_id: i64) -> Option<SearchRow> {
    type Row = (
        Option<String>,
        i32,
        i32,
        Option<f64>,
        bool,
        bool,
        Option<String>,
        Option<i64>,
    );
    sqlx::query_as::<_, Row>(
        "SELECT outcome, misses, failures,
                extract(epoch FROM next_search_at - now())::float8 / 3600,
                searched_at IS NOT NULL, takedown_at IS NOT NULL, last_error,
                attached_ol_cover_id
         FROM cover_search WHERE work_id = $1",
    )
    .bind(work_id)
    .fetch_optional(&app.db)
    .await
    .unwrap()
    .map(|row| SearchRow {
        outcome: row.0,
        misses: row.1,
        failures: row.2,
        next_hours: row.3.map(|h| h.round() as i64),
        searched: row.4,
        takedown: row.5,
        last_error: row.6,
        attached_cover: row.7,
    })
}

/// The suggestions of a work as (Open Library work, cover id, reason, status), oldest first.
pub async fn suggestions(app: &TestApp, work_id: i64) -> Vec<(String, i64, String, String)> {
    sqlx::query_as(
        "SELECT ol_work_id, ol_cover_id, reason, status FROM cover_suggestions
         WHERE work_id = $1 ORDER BY id",
    )
    .bind(work_id)
    .fetch_all(&app.db)
    .await
    .unwrap()
}

/// One search of one work as the job does it (no pace).
pub async fn search_with(
    db: &PgPool,
    lookup: &dyn CatalogLookup,
    work_id: i64,
    forced: bool,
) -> Outcome {
    let mut conn = db.acquire().await.unwrap();
    search_on(&mut conn, lookup, work_id, forced).await
}

pub async fn search_on(
    conn: &mut PgConnection,
    lookup: &dyn CatalogLookup,
    work_id: i64,
    forced: bool,
) -> Outcome {
    let mut pacer = Pacer::none();
    let mut ctx = Context {
        lookup,
        pacer: &mut pacer,
        always_suggest: true,
        forced,
    };
    search_one(conn, &mut ctx, work_id).await.unwrap()
}

/// A JPEG of one colour.
pub fn jpeg(color: [u8; 3]) -> Vec<u8> {
    let image = RgbImage::from_pixel(60, 90, Rgb(color));
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 90)
        .write_image(image.as_raw(), 60, 90, ExtendedColorType::Rgb8)
        .unwrap();
    out
}

/// Open Library's cover server answers `status` with `body` for this cover id.
pub async fn serve_cover(app: &TestApp, ol_cover_id: i64, status: u16, body: Vec<u8>) {
    Mock::given(method("GET"))
        .and(path(format!("/b/id/{ol_cover_id}-L.jpg")))
        .respond_with(ResponseTemplate::new(status).set_body_bytes(body))
        .mount(&app.upstream)
        .await;
}

/// A lookup that runs `sql` during the remote step (a reader's upload or a takedown landing
/// meanwhile), waits for `barrier` when given (two searches at once), then answers `answer`.
pub struct Hook {
    pub db: PgPool,
    pub sql: Option<String>,
    pub barrier: Option<Arc<Barrier>>,
    pub answer: CoverAnswer,
}

#[async_trait::async_trait]
impl CatalogLookup for Hook {
    async fn resolve_isbn(
        &self,
        _conn: &mut PgConnection,
        _isbn: &Isbn13,
    ) -> Result<Option<Resolved>, LookupError> {
        Ok(None)
    }

    async fn resolve_isbn_remote(
        &self,
        _conn: &mut PgConnection,
        _isbn: &Isbn13,
    ) -> Result<Option<Resolved>, LookupError> {
        Ok(None)
    }

    async fn resolve_text(
        &self,
        _conn: &mut PgConnection,
        _title: &str,
        _authors: &str,
    ) -> Result<Option<Resolved>, LookupError> {
        Ok(None)
    }

    async fn cover_candidates(&self, _query: &CoverQuery) -> Result<CoverAnswer, LookupError> {
        if let Some(barrier) = &self.barrier {
            barrier.wait().await;
        }
        if let Some(sql) = &self.sql {
            sqlx::raw_sql(AssertSqlSafe(sql.clone()))
                .execute(&self.db)
                .await
                .unwrap();
        }
        Ok(self.answer.clone())
    }
}

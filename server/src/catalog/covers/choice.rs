//! `GET /api/books/works/{work_id}/cover-candidates` (docs/phase3/notes/cover-choice.md): the
//! covers a reader can choose for a book on their shelf (`PUT /api/shelf/{entry_id}/cover`).
//!
//! Only sources the catalog already uses, never Google (docs/plan.md, "Catalog"):
//! 1. the reader's current choice, first (also a photo of their own copy);
//! 2. with `DNB_PORTAL_COVERS` (private instance only), for a book on the reader's shelf: for at
//!    most [`PORTAL_EDITIONS`] German-group editions without a cover that the portal had none
//!    for in the last [`PORTAL_RECHECK_DAYS`] days (`editions.portal_checked_at`), the
//!    publisher's cover from the DNB portal, which becomes the edition's cover under the
//!    provenance rules (`dnb`, only where there is none), as the cover search would make it,
//!    and then lists as the edition's cover. Never on a book a takedown concerns (also through a
//!    work merged into it), checked again under the work's lock before the write;
//! 3. the work's and its editions' covers in the catalog;
//! 4. Open Library: the covers of the Open Library work's editions (`works/{id}/editions.json`,
//!    also editions that are not in the catalog) and of the work record, or, for a work without
//!    an Open Library id, the cover of at most [`ISBN_CHECKS`] of its ISBNs.
//!
//! At most [`MAX_CANDIDATES`] (plus the current choice). Every candidate is a media row served
//! from our domain: an Open Library cover gets its lazy row through
//! `ingest::ol_cover_medias` (the bytes are fetched on the first `/media/…` request, so the
//! browser never contacts Open Library). The Open Library requests go through the response cache
//! (18 hours) and the `ol_api` throttle; the account's catalog turn (`upstream_turn`) is taken
//! only when one of them is not cached. A source that does not answer leaves the list
//! `partial`; the request still answers 200 with what it has.

use std::collections::{HashMap, HashSet};

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use sqlx::{Connection, PgConnection};
use utoipa::ToSchema;

use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::catalog::covers::search::under_takedown;
use crate::catalog::detail::require_local_work;
use crate::catalog::shared::ingest::{self, CoverTarget, Source};
use crate::catalog::shared::isbn::{Isbn13, is_german_group, parse_isbn};
use crate::catalog::shared::lookup::LookupError;
use crate::catalog::upstream_turn;
use crate::error::{ApiResult, ErrorBody};
use crate::extract::ApiPath;
use crate::media::shared::register;
use crate::media::shared::urls::cover_urls;
use crate::shelf::cover::CoverKey;

/// Candidates per list, besides the reader's current choice.
pub const MAX_CANDIDATES: usize = 24;
/// ISBNs of a work without an Open Library id whose Open Library cover is asked.
pub const ISBN_CHECKS: usize = 3;
/// German-group editions without a cover the DNB portal is asked about per list.
pub const PORTAL_EDITIONS: usize = 3;
/// Days before an edition the portal had no cover for is asked again.
pub const PORTAL_RECHECK_DAYS: i32 = 7;
/// The media source of a publisher cover (migration 3901).
const PORTAL_SOURCE: &str = "dnb_portal";

/// Where a candidate comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CoverCandidateSource {
    /// The cover of the work or of one of its editions in the catalog.
    Catalog,
    /// An Open Library edition's or work's cover the catalog does not show.
    Openlibrary,
    /// A publisher cover from the DNB portal (private instance only).
    DnbPortal,
    /// The reader's photo of their own copy.
    Photo,
}

/// The edition a candidate shows, as far as it is known.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, ToSchema)]
pub struct CoverEditionLabel {
    /// The edition's own title (with its subtitle).
    pub title: Option<String>,
    pub publisher: Option<String>,
    pub year: Option<i32>,
    /// A MARC language code, e.g. `ger`.
    pub language: Option<String>,
    pub isbn13: Option<String>,
}

/// One cover a reader can choose.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CoverCandidateOut {
    /// What `PUT /api/shelf/{entry_id}/cover` takes: `ol:<cover id>` or `media:<public id>`.
    pub key: String,
    pub source: CoverCandidateSource,
    /// Same-origin `/media/{public_id}/l`. An Open Library picture is fetched on the first
    /// request; one that turns out to have no picture answers 404 (hide the candidate then).
    pub cover_url: Option<String>,
    /// `/media/{public_id}/s`.
    pub cover_thumb_url: Option<String>,
    /// The media public id (the key for "Report cover").
    pub cover_public_id: Option<String>,
    /// The edition the picture belongs to; null for a work's cover or a photo.
    pub edition_label: Option<CoverEditionLabel>,
    /// The reader's shelf entry for this book shows this cover now.
    pub chosen: bool,
}

/// The covers of one book a reader can choose.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CoverCandidatesOut {
    /// The current choice first, then the catalog's covers, then Open Library's.
    pub items: Vec<CoverCandidateOut>,
    /// A source did not answer (Open Library, the DNB portal) or the account's catalog turn was
    /// used up: the list may be incomplete, ask again later.
    pub partial: bool,
}

/// A candidate before it is written out.
#[derive(Debug, Clone)]
struct Candidate {
    media_id: i64,
    public_id: String,
    source: CoverCandidateSource,
    key: String,
    label: Option<CoverEditionLabel>,
}

/// The list as it grows: each picture once (by media id), a later label filling a missing one.
#[derive(Debug, Default)]
struct Candidates {
    items: Vec<Candidate>,
    /// The Open Library cover ids of the pictures listed, with their place in `items`.
    ol_ids: HashMap<i64, usize>,
}

impl Candidates {
    fn push(&mut self, candidate: Candidate, ol_cover_id: Option<i64>) {
        if let Some(known) = self
            .items
            .iter_mut()
            .find(|known| known.media_id == candidate.media_id)
        {
            if known.label.is_none() {
                known.label = candidate.label;
            }
            return;
        }
        if let Some(id) = ol_cover_id {
            self.ol_ids.insert(id, self.items.len());
        }
        self.items.push(candidate);
    }
}

/// A remote Open Library cover before its media row exists.
struct RemoteCover {
    cover_id: i64,
    label: Option<CoverEditionLabel>,
}

/// The covers the reader can choose for this book (docs/phase3/notes/cover-choice.md). A local
/// book only: an unknown Open Library id is not imported here.
#[utoipa::path(
    get,
    path = "/api/books/works/{work_id}/cover-candidates",
    operation_id = "catalog_cover_candidates",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("work_id" = String, Path, description = "A work id, `OL…W`, or a legacy key")),
    responses(
        (status = 200, body = CoverCandidatesOut, description = "The candidates; `partial` when a source did not answer"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`")
    )
)]
pub async fn cover_candidates(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(raw): ApiPath<String>,
) -> ApiResult<Json<CoverCandidatesOut>> {
    let (work_id, facts) = {
        let mut conn = state.db.acquire().await?;
        let work_id = require_local_work(&mut conn, &raw).await?;
        let portal_on = state.config.dnb_portal_covers;
        (
            work_id,
            facts(&mut conn, user.id, work_id, portal_on).await?,
        )
    };
    let mut partial = false;
    let cached = match &facts.ol_work_id {
        Some(id) => {
            state.catalog.ol_work_editions_cached(id).await
                && state.catalog.ol_work_cached(id).await
        }
        None => facts.isbns.is_empty(),
    } && facts.portal.is_empty();
    let turn = if cached {
        None
    } else {
        match upstream_turn(&state, user.id).await {
            Ok(turn) => Some(turn),
            Err(_) => {
                partial = true;
                None
            }
        }
    };
    let may_ask = cached || turn.is_some();

    if may_ask && !facts.portal.is_empty() {
        partial |= ask_portal(&state, work_id, &facts.portal).await?;
    }
    let mut list = Candidates::default();
    {
        let mut conn = state.db.acquire().await?;
        if let Some(chosen) = facts.chosen {
            chosen_candidate(&mut conn, chosen, &mut list).await?;
        }
        local_candidates(&mut conn, work_id, &mut list).await?;
    }
    if may_ask {
        let (remote, failed) = remote_covers(&state, &facts).await;
        partial |= failed;
        let room =
            (MAX_CANDIDATES + usize::from(facts.chosen.is_some())).saturating_sub(list.items.len());
        let mut wanted: Vec<RemoteCover> = Vec::new();
        for cover in remote {
            match list.ol_ids.get(&cover.cover_id) {
                // Listed already (the catalog's cover, or the current choice): Open Library's
                // edition record may still label it.
                Some(&at) => {
                    let known = &mut list.items[at];
                    if known.label.is_none() {
                        known.label = cover.label;
                    }
                }
                None if wanted.len() < room => wanted.push(cover),
                None => {}
            }
        }
        if !wanted.is_empty() {
            let mut conn = state.db.acquire().await?;
            open_library_candidates(&mut conn, wanted, &mut list).await?;
        }
    }

    // The picture the reader's entry shows now: their choice, else the standard cover (the
    // entry's edition's, else the work's), as far as the list has it.
    let shown = facts.chosen.or_else(|| {
        facts
            .standard
            .iter()
            .copied()
            .find(|id| list.items.iter().any(|candidate| candidate.media_id == *id))
    });
    let items = list
        .items
        .into_iter()
        .map(|candidate| {
            let (cover_url, cover_thumb_url) = cover_urls(Some(&candidate.public_id));
            CoverCandidateOut {
                key: candidate.key,
                source: candidate.source,
                cover_url,
                cover_thumb_url,
                cover_public_id: Some(candidate.public_id),
                edition_label: candidate.label,
                chosen: shown == Some(candidate.media_id),
            }
        })
        .collect();
    Ok(Json(CoverCandidatesOut { items, partial }))
}

/// What the list needs to know of the work and the reader before any remote step.
struct Facts {
    ol_work_id: Option<String>,
    /// The ISBNs whose Open Library cover is asked (a work without an Open Library id).
    isbns: Vec<Isbn13>,
    /// The German-group editions without a cover the DNB portal is asked about.
    portal: Vec<(i64, Isbn13)>,
    /// The reader's chosen picture for this book, if any.
    chosen: Option<i64>,
    /// The standard cover of the reader's entry, the edition's before the work's (no entry: none).
    standard: Vec<i64>,
}

async fn facts(
    conn: &mut PgConnection,
    user_id: i64,
    work_id: i64,
    portal_on: bool,
) -> sqlx::Result<Facts> {
    let work = sqlx::query!(
        "SELECT w.ol_work_id, w.source FROM works w WHERE w.id = $1",
        work_id
    )
    .fetch_one(&mut *conn)
    .await?;
    // As the cover search: a takedown on this work or on a work merged into it.
    let takedown = under_takedown(&mut *conn, work_id).await?;
    let editions = sqlx::query!(
        r#"SELECT id, isbn13 AS "isbn13!",
                  cover_media_id IS NULL
                      AND (portal_checked_at IS NULL
                           OR portal_checked_at < now() - make_interval(days => $2))
                      AS "askable!"
           FROM editions WHERE work_id = $1 AND isbn13 IS NOT NULL
           ORDER BY id"#,
        work_id,
        PORTAL_RECHECK_DAYS
    )
    .fetch_all(&mut *conn)
    .await?;
    let entry = sqlx::query!(
        r#"SELECT s.cover_media_id, e.cover_media_id AS "edition_cover?", w.cover_media_id AS work_cover
           FROM shelf_entries s
           JOIN works w ON w.id = s.work_id
           LEFT JOIN editions e ON e.id = s.edition_id
           WHERE s.user_id = $1 AND s.work_id = $2"#,
        user_id,
        work_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    let on_shelf = entry.is_some();
    let chosen = entry.as_ref().and_then(|entry| entry.cover_media_id);
    let standard = entry
        .map(|entry| {
            [entry.edition_cover, entry.work_cover]
                .into_iter()
                .flatten()
                .collect()
        })
        .unwrap_or_default();
    let parsed: Vec<(i64, Isbn13, bool)> = editions
        .into_iter()
        .filter_map(|e| parse_isbn(&e.isbn13).map(|isbn| (e.id, isbn, e.askable)))
        .collect();
    let isbns = if work.ol_work_id.is_none() {
        parsed
            .iter()
            .map(|(_, isbn, _)| isbn.clone())
            .take(ISBN_CHECKS)
            .collect()
    } else {
        Vec::new()
    };
    // As the cover search: never for a book a reader added, and no picture without an admin on
    // a book whose cover was taken down. Only for a book on the reader's shelf (the chooser's
    // book), and not again for an edition the portal had nothing for lately.
    let portal_allowed = portal_on && on_shelf && work.source != "user" && !takedown;
    let portal = parsed
        .into_iter()
        .filter(|(_, isbn, askable)| portal_allowed && *askable && is_german_group(isbn))
        .map(|(id, isbn, _)| (id, isbn))
        .take(PORTAL_EDITIONS)
        .collect();
    Ok(Facts {
        ol_work_id: work.ol_work_id,
        isbns,
        portal,
        chosen,
        standard,
    })
}

/// The reader's current choice, when it is still usable (a cover, not taken down, not gone, not
/// with the bytes of a picture a takedown judged; [`local_candidates`] lists the same).
async fn chosen_candidate(
    conn: &mut PgConnection,
    media_id: i64,
    list: &mut Candidates,
) -> sqlx::Result<()> {
    let row = sqlx::query!(
        r#"SELECT m.id, m.public_id, m.source, m.ol_cover_id, m.personal FROM media m
           WHERE m.id = $1 AND m.kind = 'cover' AND m.removed_at IS NULL AND m.ol_gone_at IS NULL
             AND NOT EXISTS (SELECT 1 FROM media_removed_hashes h
                             WHERE h.sha256 = m.sha256 AND h.blocks_covers)"#,
        media_id
    )
    .fetch_optional(conn)
    .await?;
    if let Some(row) = row {
        let (source, key) = match (row.personal, row.source.as_str(), row.ol_cover_id) {
            (true, _, _) => (CoverCandidateSource::Photo, None),
            (false, "openlibrary", Some(id)) => (
                CoverCandidateSource::Openlibrary,
                Some(CoverKey::OpenLibrary(id)),
            ),
            (false, PORTAL_SOURCE, _) => (CoverCandidateSource::DnbPortal, None),
            _ => (CoverCandidateSource::Catalog, None),
        };
        let key = key.unwrap_or_else(|| CoverKey::Media(row.public_id.clone()));
        list.push(
            Candidate {
                media_id: row.id,
                public_id: row.public_id,
                source,
                key: key.format(),
                label: None,
            },
            row.ol_cover_id,
        );
    }
    Ok(())
}

/// The work's cover and its editions' covers, the work's first, then by edition id.
async fn local_candidates(
    conn: &mut PgConnection,
    work_id: i64,
    list: &mut Candidates,
) -> sqlx::Result<()> {
    let rows = sqlx::query!(
        r#"SELECT m.id AS "media_id!", m.public_id AS "public_id!", m.source AS "source!",
                  m.ol_cover_id, e.id AS "edition_id?", e.title AS "title?",
                  e.subtitle AS "subtitle?", e.publisher AS "publisher?",
                  e.publication_year AS "year?", e.language AS "language?",
                  e.isbn13 AS "isbn13?"
           FROM (SELECT w.cover_media_id AS media_id, NULL::int8 AS edition_id, 0 AS rank
                 FROM works w WHERE w.id = $1
                 UNION ALL
                 SELECT e.cover_media_id, e.id, 1 FROM editions e
                 WHERE e.work_id = $1 AND e.cover_media_id IS NOT NULL) c
           JOIN media m ON m.id = c.media_id
           LEFT JOIN editions e ON e.id = c.edition_id
           WHERE m.kind = 'cover' AND m.removed_at IS NULL AND m.ol_gone_at IS NULL
             AND NOT EXISTS (SELECT 1 FROM media_removed_hashes h
                             WHERE h.sha256 = m.sha256 AND h.blocks_covers)
           ORDER BY c.rank, c.edition_id"#,
        work_id
    )
    .fetch_all(conn)
    .await?;
    for row in rows {
        let label = row.edition_id.map(|_| CoverEditionLabel {
            title: row
                .title
                .map(|title| match row.subtitle.filter(|s| !s.is_empty()) {
                    Some(subtitle) => format!("{title}: {subtitle}"),
                    None => title,
                }),
            publisher: row.publisher.filter(|p| !p.is_empty()),
            year: row.year,
            language: row.language,
            isbn13: row.isbn13,
        });
        let source = if row.source == PORTAL_SOURCE {
            CoverCandidateSource::DnbPortal
        } else {
            CoverCandidateSource::Catalog
        };
        list.push(
            Candidate {
                media_id: row.media_id,
                key: CoverKey::Media(row.public_id.clone()).format(),
                public_id: row.public_id,
                source,
                label,
            },
            row.ol_cover_id,
        );
    }
    Ok(())
}

/// Asks the DNB portal for the publisher's cover of each edition and makes a found one the
/// edition's cover (provenance `dnb`, only where there is none), media row first (conventions
/// §7.2), then the work's lock, under which the edition must still be the work's and no takedown
/// may concern the work (as `search::attach_publisher_cover` checks). An edition the portal had
/// no usable picture for gets `portal_checked_at`. Returns whether a request had no answer.
async fn ask_portal(state: &AppState, work_id: i64, editions: &[(i64, Isbn13)]) -> ApiResult<bool> {
    let mut failed = false;
    for (edition_id, isbn) in editions {
        let bytes = match state.lookup.publisher_cover(isbn).await {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                portal_checked(&state.db, *edition_id).await?;
                continue;
            }
            Err(LookupError::Db(error)) => return Err(error.into()),
            Err(LookupError::Throttled | LookupError::Unavailable) => {
                failed = true;
                continue;
            }
        };
        let mut conn = state.db.acquire().await?;
        let stored = match state
            .media_service
            .store_found_cover(&mut conn, PORTAL_SOURCE, bytes)
            .await
        {
            Ok(Some(stored)) => stored,
            // No usable picture (not an image, or the bytes of a taken-down one).
            Ok(None) => {
                portal_checked(&mut *conn, *edition_id).await?;
                continue;
            }
            Err(error) => {
                tracing::warn!(error = %error, "cannot store a publisher cover");
                failed = true;
                continue;
            }
        };
        let (public_id, media) = stored;
        let attached = async {
            let mut tx = conn.begin().await?;
            let media_id = register(&mut tx, &public_id, &media).await?;
            // The work's lock (the order `set_cover` and a takedown's `clear_media` use, media
            // rows first): a takedown that cleared the book's cover while the portal answered is
            // seen here, and one that comes later clears this picture too.
            sqlx::query_scalar!("SELECT id FROM works WHERE id = $1 FOR UPDATE", work_id)
                .fetch_optional(&mut *tx)
                .await?;
            // The edition must still belong to the book (a merge may have moved it).
            let still = sqlx::query_scalar!(
                r#"SELECT EXISTS (SELECT 1 FROM editions WHERE id = $1 AND work_id = $2)
                       AS "still!""#,
                edition_id,
                work_id
            )
            .fetch_one(&mut *tx)
            .await?;
            // No picture without an admin on a book a takedown concerns.
            let set = still
                && !under_takedown(&mut tx, work_id).await?
                && ingest::set_cover(
                    &mut tx,
                    CoverTarget::Edition(*edition_id),
                    media_id,
                    Source::Dnb,
                    true,
                )
                .await?;
            if set {
                tx.commit().await?;
            } else {
                tx.rollback().await?;
            }
            Ok::<bool, sqlx::Error>(set)
        }
        .await;
        match attached {
            Ok(true) => {}
            Ok(false) => state.media_service.discard_cover(&public_id).await,
            Err(error) => {
                state.media_service.discard_cover(&public_id).await;
                return Err(error.into());
            }
        }
    }
    Ok(failed)
}

/// Records that the portal had no usable cover for the edition (asked again after
/// [`PORTAL_RECHECK_DAYS`]).
async fn portal_checked(db: impl sqlx::PgExecutor<'_>, edition_id: i64) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE editions SET portal_checked_at = now() WHERE id = $1",
        edition_id
    )
    .execute(db)
    .await?;
    Ok(())
}

/// The Open Library covers of the book, edition covers (with their labels) first, each cover id
/// once; and whether a request had no answer.
async fn remote_covers(state: &AppState, facts: &Facts) -> (Vec<RemoteCover>, bool) {
    let mut covers: Vec<RemoteCover> = Vec::new();
    let mut failed = false;
    if let Some(ol_work_id) = &facts.ol_work_id {
        let (editions, work) = tokio::join!(
            state.catalog.ol_work_editions(ol_work_id),
            state.catalog.ol_work(ol_work_id, false)
        );
        match editions {
            Ok(editions) => covers.extend(editions.into_iter().map(|edition| RemoteCover {
                cover_id: edition.cover_id,
                label: Some(CoverEditionLabel {
                    title: edition.title,
                    publisher: edition.publisher,
                    year: edition.year,
                    language: edition.language,
                    isbn13: edition.isbn13,
                }),
            })),
            Err(crate::catalog::cache::Fetch::Missing) => {}
            Err(_) => failed = true,
        }
        match work {
            Ok(work) => {
                let ids = work
                    .get("covers")
                    .and_then(|covers| covers.as_array())
                    .map(|covers| {
                        covers
                            .iter()
                            .filter_map(crate::catalog::openlibrary::positive_cover_id)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                covers.extend(ids.into_iter().map(|cover_id| RemoteCover {
                    cover_id,
                    label: None,
                }));
            }
            Err(crate::catalog::cache::Fetch::Missing) => {}
            Err(_) => failed = true,
        }
    } else {
        for isbn in &facts.isbns {
            match state.lookup.cover_by_isbn(isbn).await {
                Ok(Some(cover_id)) => covers.push(RemoteCover {
                    cover_id,
                    label: Some(CoverEditionLabel {
                        isbn13: Some(isbn.as_str().to_owned()),
                        ..CoverEditionLabel::default()
                    }),
                }),
                Ok(None) => {}
                Err(_) => failed = true,
            }
        }
    }
    let mut seen = HashSet::new();
    covers.retain(|cover| seen.insert(cover.cover_id));
    (covers, failed)
}

/// The lazy media rows of `wanted` (created when missing; taken-down, gone and blocked pictures
/// are left out), added in the order given.
async fn open_library_candidates(
    conn: &mut PgConnection,
    wanted: Vec<RemoteCover>,
    list: &mut Candidates,
) -> sqlx::Result<()> {
    let ids: Vec<i64> = wanted.iter().map(|cover| cover.cover_id).collect();
    let mut tx = conn.begin().await?;
    let media = ingest::ol_cover_medias(&mut tx, &ids).await?;
    let media_ids: Vec<i64> = media.values().copied().collect();
    let public_ids: HashMap<i64, String> = sqlx::query!(
        "SELECT id, public_id FROM media WHERE id = ANY($1)",
        &media_ids
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|row| (row.id, row.public_id))
    .collect();
    tx.commit().await?;
    for cover in wanted {
        let Some(&media_id) = media.get(&cover.cover_id) else {
            continue;
        };
        let Some(public_id) = public_ids.get(&media_id) else {
            continue;
        };
        list.push(
            Candidate {
                media_id,
                public_id: public_id.clone(),
                source: CoverCandidateSource::Openlibrary,
                key: CoverKey::OpenLibrary(cover.cover_id).format(),
                label: cover.label,
            },
            Some(cover.cover_id),
        );
    }
    Ok(())
}

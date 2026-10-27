//! `merge_works`: two works becoming one on live data (docs/phase1/conventions.md §7.6), with
//! the collision rules of `catalog::shared::merge_rules` that the importer applies too
//! (docs/phase1/legacy-mapping.md §3.4). Used by an approved `work_merge` request and the direct
//! admin merge.
//!
//! One transaction (a savepoint inside the caller's):
//! 1. The covers of both works are locked `FOR SHARE` before any catalog row (conventions §7.2),
//!    so a takedown of one of them waits for the merge or runs first, never deadlocks with it.
//!    Then both works are locked (`FOR NO KEY UPDATE`, which does not block the foreign-key
//!    checks of concurrent shelf writes), then every user with a shelf entry or a favourite on
//!    the loser (`FOR UPDATE`, in id order, like SHELF's writes). When the loser got another
//!    cover between the two locks, the merge starts over (twice at most).
//! 2. Shelf entries: a user with both works keeps the entry `shelf_winner` picks, with the
//!    other's fields merged in (`merge_shelf_fields`); the other entry goes and their physical
//!    layout is remapped. Positions are renumbered per (user, status).
//! 3. Favourites: the lower position is kept, then positions are compacted to 1..n.
//! 4. Nominations: the earliest of a vote is kept; the other's ballots and a vote's winner move
//!    to it.
//! 5. The winner's record stays the authority: its empty fields are filled from the loser, its
//!    legacy bare facts give way to the loser's better-ranked values, and an admin-approved
//!    loser value wins over anything but the winner's own admin value. A legacy winner becomes a
//!    DNB or Open Library work. `ol_work_id` moves when the winner has none.
//! 6. Editions, legacy keys, posts, quotes, events, picks and reports are re-pointed; earlier
//!    merges into the loser now point at the winner.
//! 7. Open requests on the loser are rejected with a note (requests on its editions follow them;
//!    merge requests into the loser now name the winner), and `merged_into_id` is set.

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use sqlx::types::Json;
use sqlx::{Connection, PgConnection};
use time::OffsetDateTime;

use super::shared::ingest::{Source, author_surname, match_key, search_extra};
use super::shared::merge_rules::{ShelfFacts, merge_shelf_fields, remap_layout, shelf_winner};
use crate::dto::{ShelfStatus, Visibility};

/// What a merge changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeReport {
    /// The work merged away (it keeps its id and redirects to `winner`).
    pub loser: i64,
    pub winner: i64,
    /// Shelf entries moved to the winner as they were.
    pub shelf_moved: u64,
    /// Users who had both works on their shelf: one entry each was folded into the other.
    pub shelf_merged: u64,
    pub favourites_moved: u64,
    pub favourites_merged: u64,
    pub nominations_moved: u64,
    pub nominations_merged: u64,
    pub editions_moved: u64,
    pub legacy_keys_moved: u64,
    /// Posts, quotes, events, club picks and reports re-pointed.
    pub references_moved: u64,
    pub requests_rejected: u64,
    pub ol_work_id_moved: bool,
}

/// Merges work `loser` into work `winner` (conventions §7.6). Both must exist and be current
/// (`RowNotFound` otherwise); the caller checks that and answers 404 or 400 first. Atomic: a
/// savepoint inside the caller's transaction (or a transaction of its own on a plain
/// connection).
pub async fn merge_works(
    conn: &mut PgConnection,
    loser: i64,
    winner: i64,
) -> sqlx::Result<MergeReport> {
    if loser == winner {
        return Err(sqlx::Error::Protocol(
            "a work cannot be merged into itself".into(),
        ));
    }
    let mut attempt = 0;
    loop {
        let mut tx = conn.begin().await?;
        let last_try = attempt == COVER_RETRIES;
        match merge_in(&mut tx, loser, winner, last_try).await? {
            Some(report) => {
                tx.commit().await?;
                return Ok(report);
            }
            None => {
                tracing::info!(
                    loser,
                    winner,
                    "the loser's cover changed while locking; retrying"
                );
                tx.rollback().await?;
                attempt += 1;
            }
        }
    }
}

/// How often [`merge_works`] starts over when the loser got another cover between the cover
/// lock and the works lock.
const COVER_RETRIES: u32 = 2;

/// One attempt. `None`: the loser's cover changed after the covers were locked, and this is not
/// the last try (the caller rolls back, which releases every lock, and starts over). On the last
/// try such a cover stays with the loser.
async fn merge_in(
    conn: &mut PgConnection,
    loser: i64,
    winner: i64,
    last_try: bool,
) -> sqlx::Result<Option<MergeReport>> {
    let mut report = MergeReport {
        loser,
        winner,
        ..MergeReport::default()
    };
    let pair = [loser.min(winner), loser.max(winner)];
    // Conventions §7.2: the media rows a write attaches are locked FOR SHARE before any catalog
    // row. MED's takedown updates the media row and then, in clear_media, the works that point
    // at it; locking the works first would deadlock with it.
    let covers = sqlx::query_scalar!(
        "SELECT id FROM media
         WHERE id IN (SELECT cover_media_id FROM works WHERE id = ANY($1))
           AND removed_at IS NULL
         ORDER BY id FOR SHARE",
        &pair[..]
    )
    .fetch_all(&mut *conn)
    .await?;
    let locked = sqlx::query!(
        "SELECT id, cover_media_id FROM works WHERE id = ANY($1) AND merged_into_id IS NULL
         ORDER BY id FOR NO KEY UPDATE",
        &pair[..]
    )
    .fetch_all(&mut *conn)
    .await?;
    if locked.len() != 2 {
        return Err(sqlx::Error::RowNotFound);
    }
    let loser_cover = locked
        .iter()
        .find(|row| row.id == loser)
        .and_then(|row| row.cover_media_id);
    if loser_cover.is_some_and(|id| !covers.contains(&id)) && !last_try {
        return Ok(None);
    }
    sqlx::query_scalar!(
        "SELECT id FROM users
         WHERE id IN (SELECT user_id FROM shelf_entries WHERE work_id = $1
                      UNION SELECT user_id FROM user_favorites WHERE work_id = $1)
         ORDER BY id FOR UPDATE",
        loser
    )
    .fetch_all(&mut *conn)
    .await?;

    merge_shelves(conn, loser, winner, &mut report).await?;
    merge_favourites(conn, loser, winner, &mut report).await?;
    merge_nominations(conn, loser, winner, &mut report).await?;
    merge_fields(conn, loser, winner, &covers, &mut report).await?;
    move_references(conn, loser, winner, &mut report).await?;
    settle_requests(conn, loser, winner, &mut report).await?;
    sqlx::query!(
        "UPDATE works SET merged_into_id = $2, updated_at = now() WHERE id = $1",
        loser,
        winner
    )
    .execute(&mut *conn)
    .await?;
    refresh_derived(conn, winner).await?;
    tracing::info!(?report, "merged two works");
    Ok(Some(report))
}

fn visibility_str(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Audience => "audience",
        Visibility::OnlyMe => "only_me",
    }
}

async fn merge_shelves(
    conn: &mut PgConnection,
    loser: i64,
    winner: i64,
    report: &mut MergeReport,
) -> sqlx::Result<()> {
    let rows = sqlx::query!(
        r#"SELECT id, user_id, work_id, status AS "status: ShelfStatus", updated_at, rating,
                  rating_visibility AS "rating_visibility: Visibility", take, dnf_reason,
                  progress, started_at, finished_at, edition_id
           FROM shelf_entries
           WHERE work_id = ANY($1)
             AND user_id IN (SELECT user_id FROM shelf_entries WHERE work_id = $2)
           ORDER BY user_id, id FOR UPDATE"#,
        &[loser, winner][..],
        loser
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut by_user: BTreeMap<i64, Vec<ShelfFacts>> = BTreeMap::new();
    for row in rows {
        by_user.entry(row.user_id).or_default().push(ShelfFacts {
            id: row.id,
            status: row.status,
            updated_at: row.updated_at,
            rating: row.rating,
            rating_visibility: row.rating_visibility,
            take: row.take,
            dnf_reason: row.dnf_reason,
            progress: row.progress,
            started_at: row.started_at,
            finished_at: row.finished_at,
            edition_id: row.edition_id,
        });
    }
    let users: Vec<i64> = by_user.keys().copied().collect();
    for (user_id, mut entries) in by_user {
        if entries.len() == 2 {
            let mut sorted = std::mem::take(&mut entries);
            sorted.sort_by(|a, b| shelf_winner(b, a));
            let (mut keep, drop) = (sorted[0].clone(), sorted[1].clone());
            merge_shelf_fields(&mut keep, &drop);
            sqlx::query!("DELETE FROM shelf_entries WHERE id = $1", drop.id)
                .execute(&mut *conn)
                .await?;
            sqlx::query!(
                "UPDATE shelf_entries
                 SET work_id = $2, rating = $3, rating_visibility = $4, take = $5,
                     dnf_reason = $6, progress = $7, started_at = $8, finished_at = $9,
                     edition_id = $10
                 WHERE id = $1",
                keep.id,
                winner,
                keep.rating,
                visibility_str(keep.rating_visibility),
                keep.take,
                keep.dnf_reason,
                keep.progress,
                keep.started_at,
                keep.finished_at,
                keep.edition_id
            )
            .execute(&mut *conn)
            .await?;
            remap_user_layout(conn, user_id, drop.id, keep.id).await?;
            report.shelf_merged += 1;
        } else {
            sqlx::query!(
                "UPDATE shelf_entries SET work_id = $2 WHERE id = $1",
                entries[0].id,
                winner
            )
            .execute(&mut *conn)
            .await?;
            report.shelf_moved += 1;
        }
    }
    if !users.is_empty() {
        sqlx::query!(
            "UPDATE shelf_entries s SET position = r.rn - 1
             FROM (SELECT id, (row_number() OVER (PARTITION BY user_id, status
                                                  ORDER BY position, id))::int AS rn
                   FROM shelf_entries WHERE user_id = ANY($1)) AS r
             WHERE s.id = r.id AND s.position <> r.rn - 1",
            &users
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn remap_user_layout(
    conn: &mut PgConnection,
    user_id: i64,
    loser_entry: i64,
    winner_entry: i64,
) -> sqlx::Result<()> {
    let Some(mut layout) = sqlx::query_scalar!(
        "SELECT layout FROM physical_shelf_layouts WHERE user_id = $1 FOR UPDATE",
        user_id
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(());
    };
    let before = layout.clone();
    remap_layout(&mut layout, loser_entry, winner_entry);
    if layout != before {
        sqlx::query!(
            "UPDATE physical_shelf_layouts SET layout = $2, updated_at = now() WHERE user_id = $1",
            user_id,
            layout
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn merge_favourites(
    conn: &mut PgConnection,
    loser: i64,
    winner: i64,
    report: &mut MergeReport,
) -> sqlx::Result<()> {
    let rows = sqlx::query!(
        "SELECT user_id, work_id, position FROM user_favorites
         WHERE work_id = ANY($1)
           AND user_id IN (SELECT user_id FROM user_favorites WHERE work_id = $2)
         ORDER BY user_id, position",
        &[loser, winner][..],
        loser
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut by_user: BTreeMap<i64, Vec<(i64, i32)>> = BTreeMap::new();
    for row in rows {
        by_user
            .entry(row.user_id)
            .or_default()
            .push((row.work_id, row.position));
    }
    let users: Vec<i64> = by_user.keys().copied().collect();
    for (user_id, favourites) in by_user {
        // Ordered by position: the first one is kept.
        if favourites.len() == 2 {
            let dropped = favourites[1].0;
            sqlx::query!(
                "DELETE FROM user_favorites WHERE user_id = $1 AND work_id = $2",
                user_id,
                dropped
            )
            .execute(&mut *conn)
            .await?;
            report.favourites_merged += 1;
        } else {
            report.favourites_moved += 1;
        }
        sqlx::query!(
            "UPDATE user_favorites SET work_id = $3 WHERE user_id = $1 AND work_id = $2",
            user_id,
            loser,
            winner
        )
        .execute(&mut *conn)
        .await?;
    }
    if !users.is_empty() {
        // user_favorites_position_key is deferrable, so it is checked once at the end.
        sqlx::query!(
            "UPDATE user_favorites f SET position = r.rn
             FROM (SELECT user_id, work_id,
                          (row_number() OVER (PARTITION BY user_id ORDER BY position))::int AS rn
                   FROM user_favorites WHERE user_id = ANY($1)) AS r
             WHERE f.user_id = r.user_id AND f.work_id = r.work_id AND f.position <> r.rn",
            &users
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn merge_nominations(
    conn: &mut PgConnection,
    loser: i64,
    winner: i64,
    report: &mut MergeReport,
) -> sqlx::Result<()> {
    sqlx::query_scalar!(
        "SELECT id FROM next_up_votes
         WHERE id IN (SELECT vote_id FROM next_up_nominations WHERE work_id = $1)
         ORDER BY id FOR UPDATE",
        loser
    )
    .fetch_all(&mut *conn)
    .await?;
    let pairs = sqlx::query!(
        "SELECT l.id AS loser_id, l.created_at AS loser_at, w.id AS winner_id,
                w.created_at AS winner_at
         FROM next_up_nominations l
         JOIN next_up_nominations w ON w.vote_id = l.vote_id AND w.work_id = $2
         WHERE l.work_id = $1
         ORDER BY l.vote_id",
        loser,
        winner
    )
    .fetch_all(&mut *conn)
    .await?;
    for pair in pairs {
        let loser_first = (pair.loser_at, pair.loser_id) < (pair.winner_at, pair.winner_id);
        let (kept, dropped) = if loser_first {
            (pair.loser_id, pair.winner_id)
        } else {
            (pair.winner_id, pair.loser_id)
        };
        // One ballot per user and vote, so moving them cannot collide.
        sqlx::query!(
            "UPDATE next_up_ballots SET nomination_id = $1 WHERE nomination_id = $2",
            kept,
            dropped
        )
        .execute(&mut *conn)
        .await?;
        sqlx::query!(
            "UPDATE next_up_votes SET winner_nomination_id = $1 WHERE winner_nomination_id = $2",
            kept,
            dropped
        )
        .execute(&mut *conn)
        .await?;
        sqlx::query!("DELETE FROM next_up_nominations WHERE id = $1", dropped)
            .execute(&mut *conn)
            .await?;
        report.nominations_merged += 1;
    }
    report.nominations_moved = sqlx::query!(
        "UPDATE next_up_nominations SET work_id = $2 WHERE work_id = $1",
        loser,
        winner
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(())
}

/// The provenance rank of a field: its `field_sources` entry, else the row's source.
fn field_rank(sources: &Map<String, Value>, field: &str, row_source: &str) -> u8 {
    sources
        .get(field)
        .and_then(Value::as_str)
        .and_then(Source::parse)
        .or_else(|| Source::parse(row_source))
        .unwrap_or(Source::Legacy)
        .rank()
}

/// The provenance a moved field takes along.
fn field_source(sources: &Map<String, Value>, field: &str, row_source: &str) -> Value {
    sources
        .get(field)
        .cloned()
        .unwrap_or_else(|| Value::from(row_source))
}

struct WorkFields {
    title: String,
    subtitle: String,
    author_names: String,
    first_publish_year: Option<i32>,
    description: String,
    subjects: Vec<String>,
    pages: Option<i32>,
    ol_rating: Option<f64>,
    ol_rating_count: Option<i32>,
    cover_media_id: Option<i64>,
    source: String,
    ol_work_id: Option<String>,
    details_fetched_at: Option<OffsetDateTime>,
    field_sources: Map<String, Value>,
}

async fn load_fields(conn: &mut PgConnection, id: i64) -> sqlx::Result<WorkFields> {
    let row = sqlx::query!(
        r#"SELECT title, subtitle, author_names, first_publish_year, description, subjects, pages,
                  ol_rating, ol_rating_count, cover_media_id, source, ol_work_id,
                  details_fetched_at,
                  field_sources AS "field_sources: Json<Map<String, Value>>"
           FROM works WHERE id = $1"#,
        id
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(WorkFields {
        title: row.title,
        subtitle: row.subtitle,
        author_names: row.author_names,
        first_publish_year: row.first_publish_year,
        description: row.description,
        subjects: row.subjects,
        pages: row.pages,
        ol_rating: row.ol_rating,
        ol_rating_count: row.ol_rating_count,
        cover_media_id: row.cover_media_id,
        source: row.source,
        ol_work_id: row.ol_work_id,
        details_fetched_at: row.details_fetched_at,
        field_sources: row.field_sources.0,
    })
}

/// Takes a loser field, with its provenance, where the winner's is empty, legacy-ranked below
/// it, or below an admin-approved loser value (a reader's value never replaces a source's on a
/// merge: the admin picked the winner's record); moves `ol_work_id` when the winner has none;
/// upgrades a legacy winner to the loser's source. The loser's cover moves only when it is
/// among `covers`, the usable media rows `merge_in` locked before the works.
async fn merge_fields(
    conn: &mut PgConnection,
    loser: i64,
    winner: i64,
    covers: &[i64],
    report: &mut MergeReport,
) -> sqlx::Result<()> {
    let l = load_fields(conn, loser).await?;
    let mut w = load_fields(conn, winner).await?;
    let mut sources = w.field_sources.clone();
    let (w_sources, w_source) = (w.field_sources.clone(), w.source.clone());
    // Whether the loser's value of `field` replaces the winner's.
    let wins = |field: &str, winner_empty: bool, loser_empty: bool| {
        let loser_rank = field_rank(&l.field_sources, field, &l.source);
        let winner_rank = field_rank(&w_sources, field, &w_source);
        !loser_empty
            && (winner_empty
                || (loser_rank > winner_rank
                    && (winner_rank == Source::Legacy.rank()
                        || loser_rank == Source::Admin.rank())))
    };
    let take = |sources: &mut Map<String, Value>, field: &str| {
        sources.insert(
            field.to_owned(),
            field_source(&l.field_sources, field, &l.source),
        );
    };
    if wins("title", false, false) {
        w.title = l.title.clone();
        take(&mut sources, "title");
    }
    if wins("subtitle", w.subtitle.is_empty(), l.subtitle.is_empty()) {
        w.subtitle = l.subtitle.clone();
        take(&mut sources, "subtitle");
    }
    let authors_moved = wins(
        "author_names",
        w.author_names.is_empty(),
        l.author_names.is_empty(),
    );
    if authors_moved {
        w.author_names = l.author_names.clone();
        take(&mut sources, "author_names");
    }
    if wins(
        "first_publish_year",
        w.first_publish_year.is_none(),
        l.first_publish_year.is_none(),
    ) {
        w.first_publish_year = l.first_publish_year;
        take(&mut sources, "first_publish_year");
    }
    if wins(
        "description",
        w.description.is_empty(),
        l.description.is_empty(),
    ) {
        w.description = l.description.clone();
        take(&mut sources, "description");
    }
    if wins("subjects", w.subjects.is_empty(), l.subjects.is_empty()) {
        w.subjects = l.subjects.clone();
        take(&mut sources, "subjects");
    }
    if wins("pages", w.pages.is_none(), l.pages.is_none()) {
        w.pages = l.pages;
        take(&mut sources, "pages");
    }
    if wins(
        "cover_media_id",
        w.cover_media_id.is_none(),
        l.cover_media_id.is_none(),
    ) {
        // Locked FOR SHARE before the works (merge_in), so a takedown of it waits for this
        // merge; a removed picture was never locked and stays behind.
        if l.cover_media_id.is_some_and(|id| covers.contains(&id)) {
            w.cover_media_id = l.cover_media_id;
            take(&mut sources, "cover_media_id");
        }
    }
    if w.ol_rating.is_none() && l.ol_rating.is_some() {
        w.ol_rating = l.ol_rating;
        w.ol_rating_count = l.ol_rating_count;
    }
    if w.ol_work_id.is_none() && l.ol_work_id.is_some() {
        // works_ol_work_id_key: the loser lets go first.
        sqlx::query!("UPDATE works SET ol_work_id = NULL WHERE id = $1", loser)
            .execute(&mut *conn)
            .await?;
        w.ol_work_id = l.ol_work_id.clone();
        w.details_fetched_at = w.details_fetched_at.or(l.details_fetched_at);
        report.ol_work_id_moved = true;
    }
    // A legacy work becomes a source's work when a record of that source is merged into it.
    if w.source == "legacy" {
        if report.ol_work_id_moved {
            w.source = "openlibrary".to_owned();
        } else if l.source == "dnb" {
            w.source = "dnb".to_owned();
        }
    }
    sqlx::query!(
        "UPDATE works SET title = $2, subtitle = $3, author_names = $4, first_publish_year = $5,
                description = $6, subjects = $7, pages = $8, ol_rating = $9,
                ol_rating_count = $10, cover_media_id = $11, source = $12, ol_work_id = $13,
                details_fetched_at = $14, field_sources = $15, updated_at = now()
         WHERE id = $1",
        winner,
        w.title,
        w.subtitle,
        w.author_names,
        w.first_publish_year,
        w.description,
        &w.subjects,
        w.pages,
        w.ol_rating,
        w.ol_rating_count,
        w.cover_media_id,
        w.source,
        w.ol_work_id,
        w.details_fetched_at,
        Value::Object(sources)
    )
    .execute(&mut *conn)
    .await?;
    if authors_moved {
        // The author records travel with the names they describe.
        sqlx::query!("DELETE FROM work_authors WHERE work_id = $1", winner)
            .execute(&mut *conn)
            .await?;
        sqlx::query!(
            "INSERT INTO work_authors (work_id, author_id, position)
             SELECT $2, author_id, position FROM work_authors WHERE work_id = $1",
            loser,
            winner
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn move_references(
    conn: &mut PgConnection,
    loser: i64,
    winner: i64,
    report: &mut MergeReport,
) -> sqlx::Result<()> {
    report.editions_moved = sqlx::query!(
        "UPDATE editions SET work_id = $2, updated_at = now() WHERE work_id = $1",
        loser,
        winner
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    report.legacy_keys_moved = sqlx::query!(
        "UPDATE legacy_work_keys SET work_id = $2 WHERE work_id = $1",
        loser,
        winner
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    // One statement per table: a thread (posts_parent_fk) moves as a whole.
    let mut moved = 0;
    moved += sqlx::query!(
        "UPDATE posts SET work_id = $2 WHERE work_id = $1",
        loser,
        winner
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    moved += sqlx::query!(
        "UPDATE quotes SET work_id = $2 WHERE work_id = $1",
        loser,
        winner
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    moved += sqlx::query!(
        "UPDATE events SET work_id = $2 WHERE work_id = $1",
        loser,
        winner
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    moved += sqlx::query!(
        "UPDATE club_picks SET work_id = $2 WHERE work_id = $1",
        loser,
        winner
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    moved += sqlx::query!(
        "UPDATE reports SET work_id = $2 WHERE work_id = $1",
        loser,
        winner
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    report.references_moved = moved;
    // Works merged into the loser earlier now redirect to the winner in one step.
    sqlx::query!(
        "UPDATE works SET merged_into_id = $2, updated_at = now() WHERE merged_into_id = $1",
        loser,
        winner
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn settle_requests(
    conn: &mut PgConnection,
    loser: i64,
    winner: i64,
    report: &mut MergeReport,
) -> sqlx::Result<()> {
    // Requests on the loser's editions follow them.
    sqlx::query!(
        "UPDATE catalog_edit_requests SET work_id = $2
         WHERE work_id = $1 AND status = 'open' AND edition_id IS NOT NULL",
        loser,
        winner
    )
    .execute(&mut *conn)
    .await?;
    // "X is the same book as the loser" now names the winner.
    sqlx::query!(
        "UPDATE catalog_edit_requests
         SET payload = jsonb_set(payload, '{into_work_id}', to_jsonb($2::bigint))
         WHERE kind = 'work_merge' AND status = 'open' AND payload ->> 'into_work_id' = ($1::bigint)::text",
        loser,
        winner
    )
    .execute(&mut *conn)
    .await?;
    let note = format!("Merged into book {winner}");
    report.requests_rejected = sqlx::query!(
        "UPDATE catalog_edit_requests
         SET status = 'rejected', decided_at = now(), decision_note = $3
         WHERE status = 'open'
           AND ((work_id = $1 AND edition_id IS NULL)
                OR (kind = 'work_merge' AND work_id = $2
                    AND payload ->> 'into_work_id' = ($2::bigint)::text))",
        loser,
        winner,
        note
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(())
}

/// `match_key` and `search_extra` of the winner, which now has the loser's editions (the same
/// rules ingest maintains, conventions §7.3).
async fn refresh_derived(conn: &mut PgConnection, work_id: i64) -> sqlx::Result<()> {
    let row = sqlx::query!(
        r#"SELECT w.title, w.author_names,
                  fa.name AS "first_name?", fa.sort_name AS "first_sort?",
                  ARRAY(SELECT e.title FROM editions e
                        WHERE e.work_id = w.id AND e.title IS NOT NULL ORDER BY e.id)
                      AS "edition_titles!: Vec<String>"
           FROM works w
           LEFT JOIN LATERAL (
               SELECT a.name, a.sort_name FROM work_authors wa JOIN authors a ON a.id = wa.author_id
               WHERE wa.work_id = w.id ORDER BY wa.position, a.id LIMIT 1
           ) fa ON true
           WHERE w.id = $1"#,
        work_id
    )
    .fetch_one(&mut *conn)
    .await?;
    let surname = match &row.first_name {
        Some(name) => author_surname(name, row.first_sort.as_deref()),
        None => author_surname(&row.author_names, None),
    };
    sqlx::query!(
        "UPDATE works SET match_key = $2, search_extra = $3 WHERE id = $1",
        work_id,
        match_key(&row.title, &surname),
        search_extra(&row.title, &row.edition_titles)
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

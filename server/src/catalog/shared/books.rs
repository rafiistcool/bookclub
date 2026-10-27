//! The shared catalog DTO loaders (docs/phase1/conventions.md §7.1, endpoint-map §0): every
//! package builds `BookOut` and `EditionOut` through these.

use std::collections::HashMap;

use sqlx::PgConnection;

use super::isbn::{Isbn13, isbn10_of};
use crate::dto::{BookOut, CatalogSource, ContributorOut, EditionOut};
use crate::error::{ApiResult, AppError, msg};
use crate::media::shared::urls::cover_urls;

/// A work, optionally in the context of one of its editions.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct BookRef {
    pub work_id: i64,
    pub edition_id: Option<i64>,
}

impl BookRef {
    /// The work without an edition context.
    pub fn work(work_id: i64) -> Self {
        Self {
            work_id,
            edition_id: None,
        }
    }
}

/// How far `follow_merges` walks; merge chains are one or two steps long in practice.
const MAX_MERGE_HOPS: i32 = 32;

/// The work `work_id` ends up as after following `works.merged_into_id`; `None` when it does
/// not exist.
pub async fn follow_merges(conn: &mut PgConnection, work_id: i64) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar!(
        r#"WITH RECURSIVE chain (id, merged_into_id, hops) AS (
               SELECT id, merged_into_id, 0 FROM works WHERE id = $1
               UNION ALL
               SELECT w.id, w.merged_into_id, c.hops + 1
               FROM works w JOIN chain c ON w.id = c.merged_into_id
               WHERE c.hops < $2
           )
           SELECT id AS "id!" FROM chain WHERE merged_into_id IS NULL"#,
        work_id,
        MAX_MERGE_HOPS
    )
    .fetch_optional(conn)
    .await
}

/// [`follow_merges`] for many works in one query: the current work per requested id; ids that
/// do not exist are left out.
pub async fn follow_merges_many(
    conn: &mut PgConnection,
    work_ids: &[i64],
) -> sqlx::Result<HashMap<i64, i64>> {
    let rows = sqlx::query!(
        r#"WITH RECURSIVE chain (start, id, merged_into_id, hops) AS (
               SELECT id, id, merged_into_id, 0 FROM works WHERE id = ANY($1)
               UNION ALL
               SELECT c.start, w.id, w.merged_into_id, c.hops + 1
               FROM works w JOIN chain c ON w.id = c.merged_into_id
               WHERE c.hops < $2
           )
           SELECT start AS "start!", id AS "id!" FROM chain WHERE merged_into_id IS NULL"#,
        work_ids,
        MAX_MERGE_HOPS
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|row| (row.start, row.id)).collect())
}

/// The non-empty spine colour (`''` means "sampled, no usable colour").
fn spine(color: Option<String>) -> Option<String> {
    color.filter(|c| !c.is_empty())
}

/// [`BookOut`]s keyed by the requested refs. With an edition of that work, the edition's own
/// title (when it has one), pages, cover and spine colour win; an edition of another work is
/// ignored. Missing works are left out. Removed media never produce a cover.
pub async fn book_outs(
    conn: &mut PgConnection,
    refs: &[BookRef],
) -> sqlx::Result<HashMap<BookRef, BookOut>> {
    let work_ids: Vec<i64> = refs.iter().map(|r| r.work_id).collect();
    let edition_ids: Vec<Option<i64>> = refs.iter().map(|r| r.edition_id).collect();
    let rows = sqlx::query!(
        r#"SELECT r.work_id AS "work_id!", r.edition_id AS "edition_id?",
                  w.ol_work_id, w.title AS "title!", w.author_names AS "author_names!",
                  w.first_publish_year, w.pages, w.source AS "source!: CatalogSource",
                  e.title AS "edition_title?", e.pages AS "edition_pages?",
                  wm.public_id AS "work_cover?", wm.dominant_color AS "work_color?",
                  em.public_id AS "edition_cover?", em.dominant_color AS "edition_color?"
           FROM unnest($1::int8[], $2::int8[]) AS r (work_id, edition_id)
           JOIN works w ON w.id = r.work_id
           LEFT JOIN editions e ON e.id = r.edition_id AND e.work_id = r.work_id
           LEFT JOIN media wm ON wm.id = w.cover_media_id AND wm.removed_at IS NULL
           LEFT JOIN media em ON em.id = e.cover_media_id AND em.removed_at IS NULL"#,
        &work_ids,
        &edition_ids as &[Option<i64>]
    )
    .fetch_all(conn)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let key = BookRef {
                work_id: row.work_id,
                edition_id: row.edition_id,
            };
            let (cover, color) = match row.edition_cover {
                Some(cover) => (Some(cover), row.edition_color),
                None => (row.work_cover, row.work_color),
            };
            let (cover_url, cover_thumb_url) = cover_urls(cover.as_deref());
            let out = BookOut {
                id: row.work_id,
                ol_work_key: row.ol_work_id.map(|id| format!("/works/{id}")),
                title: row.edition_title.unwrap_or(row.title),
                authors: row.author_names,
                year: row.first_publish_year,
                cover_url,
                cover_thumb_url,
                cover_public_id: cover,
                pages: row.edition_pages.or(row.pages),
                spine_color: spine(color),
                custom: row.source == CatalogSource::User,
                source: row.source,
            };
            (key, out)
        })
        .collect())
}

/// One [`BookOut`] (see [`book_outs`]).
pub async fn book_out(conn: &mut PgConnection, r: BookRef) -> sqlx::Result<Option<BookOut>> {
    Ok(book_outs(conn, &[r]).await?.remove(&r))
}

/// [`EditionOut`]s by id; missing ids are left out. Contributors come in record order;
/// `isbn10` is derived; a removed cover gives no URLs.
pub async fn edition_outs(
    conn: &mut PgConnection,
    ids: &[i64],
) -> sqlx::Result<HashMap<i64, EditionOut>> {
    let rows = sqlx::query!(
        r#"SELECT e.id, e.work_id, e.isbn13, e.title, e.subtitle, e.publisher, e.language,
                  e.format, e.pages, e.publication_year, e.planned_publication_date,
                  e.source AS "source: CatalogSource", m.public_id AS "cover?"
           FROM editions e
           LEFT JOIN media m ON m.id = e.cover_media_id AND m.removed_at IS NULL
           WHERE e.id = ANY($1)"#,
        ids
    )
    .fetch_all(&mut *conn)
    .await?;

    let mut contributors: HashMap<i64, Vec<ContributorOut>> = HashMap::new();
    let contributor_rows = sqlx::query!(
        r#"SELECT c.edition_id, a.name, c.role
           FROM edition_contributors c JOIN authors a ON a.id = c.author_id
           WHERE c.edition_id = ANY($1)
           ORDER BY c.edition_id, c.position, a.id, c.role"#,
        ids
    )
    .fetch_all(&mut *conn)
    .await?;
    for row in contributor_rows {
        contributors
            .entry(row.edition_id)
            .or_default()
            .push(ContributorOut {
                name: row.name,
                role: row.role,
            });
    }

    Ok(rows
        .into_iter()
        .map(|row| {
            let (cover_url, cover_thumb_url) = cover_urls(row.cover.as_deref());
            let isbn10 = row
                .isbn13
                .as_ref()
                .and_then(|isbn| isbn10_of(&Isbn13(isbn.clone())));
            let out = EditionOut {
                id: row.id,
                work_id: row.work_id,
                isbn13: row.isbn13,
                isbn10,
                title: row.title,
                subtitle: row.subtitle,
                publisher: row.publisher,
                language: row.language,
                format: row.format,
                pages: row.pages,
                publication_year: row.publication_year,
                planned_publication_date: row.planned_publication_date,
                contributors: contributors.remove(&row.id).unwrap_or_default(),
                cover_url,
                cover_thumb_url,
                cover_public_id: row.cover,
                source: row.source,
            };
            (row.id, out)
        })
        .collect())
}

/// The current id of `work_id`, following `works.merged_into_id`. 404 `No such book.` when the
/// work does not exist.
pub async fn require_work(conn: &mut PgConnection, work_id: i64) -> ApiResult<i64> {
    follow_merges(conn, work_id)
        .await?
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))
}

/// 400 `That edition belongs to another book` unless `edition_id` is an edition of `work_id`
/// (a missing edition included).
pub async fn require_edition_of(
    conn: &mut PgConnection,
    work_id: i64,
    edition_id: i64,
) -> ApiResult<()> {
    let belongs = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM editions WHERE id = $1 AND work_id = $2) AS "belongs!""#,
        edition_id,
        work_id
    )
    .fetch_one(conn)
    .await?;
    if belongs {
        Ok(())
    } else {
        Err(AppError::bad_request(msg::EDITION_OF_ANOTHER_BOOK))
    }
}

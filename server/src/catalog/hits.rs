//! Remote hits become catalog rows, and catalog rows become annotated `SearchHit`s
//! (endpoint-map §4): every remote hit is persisted as an insert-only stub, its Open Library
//! cover is queued for prefetching, and each hit carries the viewer's shelf entry and the
//! viewer's clubs whose current pick it is.

use std::collections::{HashMap, HashSet};

use sqlx::{Connection, PgConnection};

use super::openlibrary::OlDoc;
use super::shared::books::{BookRef, book_outs};
use super::shared::ingest;
use crate::dto::{ClubRef, SearchHit};
use crate::error::is_deadlock;
use crate::media::MediaService;

/// One hit before it is loaded: a current (not merged-away) work and the ISBN it was found by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitRef {
    pub work_id: i64,
    pub isbn: Option<String>,
}

impl HitRef {
    pub fn work(work_id: i64) -> Self {
        Self {
            work_id,
            isbn: None,
        }
    }
}

/// Persists Open Library hits as stub works (one batched insert-only upsert, never overwriting
/// a stored work) and queues their covers for prefetching. Returns one `HitRef` per doc, in
/// doc order; a doc whose work was merged away resolves to the surviving work.
///
/// The batch is inserted in `ol_work_id` order, so two concurrent pages that share works lock
/// them in the same order; a deadlock aborts the batch, which is retried once.
pub async fn persist_docs(
    conn: &mut PgConnection,
    media: &MediaService,
    docs: &[OlDoc],
) -> sqlx::Result<Vec<HitRef>> {
    if docs.is_empty() {
        return Ok(Vec::new());
    }
    let mut sorted: Vec<&OlDoc> = docs.iter().collect();
    sorted.sort_by(|a, b| a.ol_work_id.cmp(&b.ol_work_id));
    sorted.dedup_by(|a, b| a.ol_work_id == b.ol_work_id);
    let records: Vec<_> = sorted.iter().map(|doc| doc.work_record()).collect();
    let mut attempt = 0;
    let ids = loop {
        let mut tx = conn.begin().await?;
        match ingest::upsert_work_stubs(&mut tx, &records).await {
            Ok(ids) => {
                tx.commit().await?;
                break ids;
            }
            Err(error) if is_deadlock(&error) && attempt == 0 => {
                tracing::warn!("persisting search hits deadlocked; retrying once");
                tx.rollback().await?;
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    };
    let by_ol: HashMap<&str, i64> = sorted
        .iter()
        .map(|doc| doc.ol_work_id.as_str())
        .zip(ids.iter().copied())
        .collect();
    prefetch_covers(conn, media, &ids).await?;
    Ok(docs
        .iter()
        .filter_map(|doc| {
            by_ol.get(doc.ol_work_id.as_str()).map(|&work_id| HitRef {
                work_id,
                isbn: doc.isbn.as_ref().map(|isbn| isbn.as_str().to_owned()),
            })
        })
        .collect())
}

/// Queues the Open Library covers of these works that were never fetched (MED's prefetch
/// worker; a no-op until MED merges).
pub async fn prefetch_covers(
    conn: &mut PgConnection,
    media: &MediaService,
    work_ids: &[i64],
) -> sqlx::Result<()> {
    let media_ids = sqlx::query_scalar!(
        r#"SELECT DISTINCT m.id
           FROM works w
           JOIN media m ON m.id = w.cover_media_id
           WHERE w.id = ANY($1) AND m.source = 'openlibrary' AND m.storage_key IS NULL
             AND m.removed_at IS NULL"#,
        work_ids
    )
    .fetch_all(&mut *conn)
    .await?;
    if !media_ids.is_empty() {
        media.prefetch(&media_ids);
    }
    Ok(())
}

/// Hit refs in order without duplicates: a later hit with the work id, or the ISBN, of an
/// earlier one is dropped. At most `limit` remain.
pub fn dedupe(refs: impl IntoIterator<Item = HitRef>, limit: usize) -> Vec<HitRef> {
    let mut works = HashSet::new();
    let mut isbns = HashSet::new();
    let mut out = Vec::new();
    for hit in refs {
        if out.len() == limit {
            break;
        }
        if works.contains(&hit.work_id) || hit.isbn.as_ref().is_some_and(|i| isbns.contains(i)) {
            continue;
        }
        works.insert(hit.work_id);
        if let Some(isbn) = &hit.isbn {
            isbns.insert(isbn.clone());
        }
        out.push(hit);
    }
    out
}

/// The viewer's shelf entry id per work.
pub async fn shelf_entries(
    conn: &mut PgConnection,
    viewer: i64,
    work_ids: &[i64],
) -> sqlx::Result<HashMap<i64, i64>> {
    let rows = sqlx::query!(
        "SELECT work_id, id FROM shelf_entries WHERE user_id = $1 AND work_id = ANY($2)",
        viewer,
        work_ids
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|row| (row.work_id, row.id)).collect())
}

/// Per work, the viewer's clubs (from `bc_member_clubs`) whose current pick it is, newest
/// membership first.
pub async fn club_picks(
    conn: &mut PgConnection,
    viewer: i64,
    work_ids: &[i64],
) -> sqlx::Result<HashMap<i64, Vec<ClubRef>>> {
    let rows = sqlx::query!(
        r#"SELECT p.work_id, c.slug, c.name
           FROM club_picks p
           JOIN bc_member_clubs($1) AS mc ON mc.club_id = p.club_id
           JOIN clubs c ON c.id = p.club_id
           WHERE p.ended_at IS NULL AND p.work_id = ANY($2)
           ORDER BY mc.joined_at DESC, c.id"#,
        viewer,
        work_ids
    )
    .fetch_all(conn)
    .await?;
    let mut picks: HashMap<i64, Vec<ClubRef>> = HashMap::new();
    for row in rows {
        picks.entry(row.work_id).or_default().push(ClubRef {
            slug: row.slug,
            name: row.name,
        });
    }
    Ok(picks)
}

/// The `SearchHit`s of `refs`, in order, annotated for `viewer`. Works that no longer exist
/// are left out.
pub async fn search_hits(
    conn: &mut PgConnection,
    viewer: i64,
    refs: &[HitRef],
) -> sqlx::Result<Vec<SearchHit>> {
    let book_refs: Vec<BookRef> = refs.iter().map(|hit| BookRef::work(hit.work_id)).collect();
    let work_ids: Vec<i64> = refs.iter().map(|hit| hit.work_id).collect();
    let mut books = book_outs(conn, &book_refs).await?;
    let shelf = shelf_entries(conn, viewer, &work_ids).await?;
    let mut picks = club_picks(conn, viewer, &work_ids).await?;
    Ok(refs
        .iter()
        .filter_map(|hit| {
            let book = books.remove(&BookRef::work(hit.work_id))?;
            let shelf_id = shelf.get(&hit.work_id).copied();
            Some(SearchHit {
                id: book.id,
                ol_work_key: book.ol_work_key,
                title: book.title,
                authors: book.authors,
                year: book.year,
                cover_url: book.cover_url,
                cover_thumb_url: book.cover_thumb_url,
                cover_public_id: book.cover_public_id,
                isbn: hit.isbn.clone(),
                source: book.source,
                custom: book.custom,
                on_shelf: shelf_id.is_some(),
                shelf_id,
                club_picks: picks.remove(&hit.work_id).unwrap_or_default(),
                planned_publication_date: None,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(work_id: i64, isbn: Option<&str>) -> HitRef {
        HitRef {
            work_id,
            isbn: isbn.map(str::to_owned),
        }
    }

    #[test]
    fn dedupe_by_work_and_isbn_and_cap() {
        let refs = vec![
            hit(1, None),
            hit(2, Some("9780316556347")),
            hit(1, None),
            hit(3, Some("9780316556347")),
            hit(4, None),
            hit(5, None),
        ];
        assert_eq!(
            dedupe(refs.clone(), 10),
            vec![
                hit(1, None),
                hit(2, Some("9780316556347")),
                hit(4, None),
                hit(5, None)
            ]
        );
        assert_eq!(dedupe(refs, 2).len(), 2);
    }
}

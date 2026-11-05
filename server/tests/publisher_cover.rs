//! Publisher covers (docs/phase3/notes/publisher-covers.md): with `DNB_PORTAL_COVERS` the cover
//! search asks the DNB portal for the cover of a German-group ISBN before it searches Open
//! Library by title, stores the picture as a `dnb_portal` media row and makes it the edition's
//! and the work's cover. The FakeCatalog stands in for the portal (the setting lives in
//! `CatalogService`).

mod common;
#[path = "common_cover/mod.rs"]
mod cover;

use bookclub_server::catalog::shared::ingest::{self, EditionRecord, Mode, Source};
use bookclub_server::catalog::shared::isbn::parse_isbn;
use common::*;
use cover::*;
use sqlx::PgPool;

const ISBN: &str = "9783462050493";

/// A legacy edition with `isbn` of `work_id`, as the importer writes it.
async fn legacy_edition(app: &TestApp, work_id: i64, isbn: &str) -> i64 {
    let edition = EditionRecord {
        isbn13: parse_isbn(isbn),
        ..EditionRecord::new(Source::Legacy)
    };
    let mut conn = app.db.acquire().await.unwrap();
    ingest::upsert_edition(&mut conn, work_id, &edition, Mode::Refresh)
        .await
        .unwrap()
        .1
}

/// The media row, its source and the provenance of the work's and the edition's cover.
async fn covers(app: &TestApp, work_id: i64, edition_id: i64) -> Option<(String, String, String)> {
    sqlx::query_as(
        "SELECT m.source, w.field_sources ->> 'cover_media_id', e.field_sources ->> 'cover_media_id'
         FROM works w JOIN media m ON m.id = w.cover_media_id
         JOIN editions e ON e.id = $2 AND e.cover_media_id = m.id
         WHERE w.id = $1",
    )
    .bind(work_id)
    .bind(edition_id)
    .fetch_optional(&app.db)
    .await
    .unwrap()
}

/// An imported book with a German ISBN gets the publisher's cover: stored as WebP variants,
/// provenance `dnb`, outcome `isbn`, and no request to Open Library by title.
#[sqlx::test]
async fn the_portal_gives_an_imported_book_its_publisher_cover(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = legacy_work(&app, "Tyll", "Daniel Kehlmann").await;
    let edition = legacy_edition(&app, work, ISBN).await;
    app.catalog
        .on_publisher_cover(ISBN, Ok(Some(jpeg([120, 40, 30]))));

    let report = run_job(&app).await;
    assert_eq!(report.isbn, 1, "{report:?}");
    assert_eq!(
        covers(&app, work, edition).await,
        Some(("dnb_portal".into(), "dnb".into(), "dnb".into()))
    );
    assert_eq!(app.catalog.publisher_calls(), [ISBN]);
    assert!(
        app.catalog.cover_calls().is_empty(),
        "no title search after the publisher's cover: {:?}",
        app.catalog.cover_calls()
    );
    let public_id: String = sqlx::query_scalar(
        "SELECT m.public_id FROM works w JOIN media m ON m.id = w.cover_media_id WHERE w.id = $1",
    )
    .bind(work)
    .fetch_one(&app.db)
    .await
    .unwrap();
    let mut keys: Vec<String> = app
        .media
        .keys()
        .into_iter()
        .filter(|key| key.contains(&public_id))
        .collect();
    keys.sort();
    assert_eq!(keys.len(), 2, "the l and s variants: {keys:?}");
    let row = search_row(&app, work).await.unwrap();
    assert_eq!(row.outcome.as_deref(), Some("isbn"));
}

/// No cover at the portal, or no answer: the search goes on by title as before, and nothing is
/// stored.
#[sqlx::test]
async fn without_a_portal_cover_the_title_search_follows(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = legacy_work(&app, "Tyll", "Daniel Kehlmann").await;
    legacy_edition(&app, work, ISBN).await;

    run_job(&app).await;
    assert_eq!(app.catalog.publisher_calls(), [ISBN]);
    assert_eq!(app.catalog.cover_calls(), ["cover:Tyll|Kehlmann|de"]);

    app.catalog
        .on_publisher_cover(ISBN, Err(FakeError::Unavailable));
    sqlx::query("UPDATE cover_search SET next_search_at = now() WHERE work_id = $1")
        .bind(work)
        .execute(&app.db)
        .await
        .unwrap();
    run_job(&app).await;
    assert_eq!(app.catalog.publisher_calls(), [ISBN, ISBN]);
    assert_eq!(app.catalog.cover_calls().len(), 2);
    assert!(cover_of(&app, work).await.is_none());
    assert!(app.media.keys().is_empty(), "{:?}", app.media.keys());
}

/// Only German-group ISBNs are asked, and only by the job and an admin's search: a search
/// without a media service (the import, `rematch-legacy`) skips the portal.
#[sqlx::test]
async fn only_german_isbns_and_only_with_a_media_service(db: PgPool) {
    let app = TestApp::new(db).await;
    let english = legacy_work(&app, "Piranesi", "Susanna Clarke").await;
    legacy_edition(&app, english, "9781526622433").await;
    run_job(&app).await;
    assert!(app.catalog.publisher_calls().is_empty());

    let german = legacy_work(&app, "Tyll", "Daniel Kehlmann").await;
    legacy_edition(&app, german, ISBN).await;
    app.catalog
        .on_publisher_cover(ISBN, Ok(Some(jpeg([120, 40, 30]))));
    search_with(&app.db, app.catalog.as_ref(), german, true).await;
    assert!(app.catalog.publisher_calls().is_empty());
    assert!(cover_of(&app, german).await.is_none());
}

/// A picture that was taken down is never stored again, and a book whose cover a takedown
/// cleared gets no publisher cover without an admin.
#[sqlx::test]
async fn a_taken_down_picture_is_not_stored_again(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = legacy_work(&app, "Tyll", "Daniel Kehlmann").await;
    legacy_edition(&app, work, ISBN).await;
    let picture = jpeg([120, 40, 30]);
    let sha256 = sha2_of(&picture);
    sqlx::query(
        "INSERT INTO media (public_id, kind, source, sha256, removed_at)
         VALUES ('removedremovedremovedremov', 'cover', 'upload', $1, now())",
    )
    .bind(&sha256[..])
    .execute(&app.db)
    .await
    .unwrap();
    app.catalog.on_publisher_cover(ISBN, Ok(Some(picture)));

    run_job(&app).await;
    assert_eq!(app.catalog.publisher_calls(), [ISBN]);
    assert!(cover_of(&app, work).await.is_none());
    assert!(app.media.keys().is_empty(), "{:?}", app.media.keys());
    assert_eq!(app.catalog.cover_calls(), ["cover:Tyll|Kehlmann|de"]);
}

fn sha2_of(data: &[u8]) -> [u8; 32] {
    bookclub_server::media::pipeline::sha256(data)
}

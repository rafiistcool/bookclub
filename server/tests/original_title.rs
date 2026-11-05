//! Original titles (docs/phase3/notes/original-title.md): DNB ingest stores a translation's
//! original title (MARC 240) on its editions and in `works.search_extra`; the cover search
//! asks the DNB once for a German-group edition that no DNB record has told one, and asks Open
//! Library by the original title when the German title finds nothing.

mod common;
#[path = "common_cover/mod.rs"]
mod cover;

use bookclub_server::catalog::shared::ingest::{self, EditionRecord, Mode, Source, WorkGrouping};
use bookclub_server::catalog::shared::isbn::parse_isbn;
use common::*;
use cover::*;
use sqlx::PgPool;

const ISBN: &str = "9783608938159";

/// One DNB record of `ISBN` with this original title, through ingest as the harvest and the
/// SRU lookup write it.
async fn ingest_dnb(app: &TestApp, original_title: Option<&str>) -> (i64, i64) {
    let edition = EditionRecord {
        isbn13: parse_isbn(ISBN),
        original_title: original_title.map(str::to_owned),
        ..EditionRecord::new(Source::Dnb)
    };
    let work = ingest::WorkRecord {
        author_names: "Patrick Rothfuss".into(),
        ..ingest::WorkRecord::new(Source::Dnb, "Der Name des Windes")
    };
    let mut conn = app.db.acquire().await.unwrap();
    ingest::upsert_edition_with_work(
        &mut conn,
        &work,
        &edition,
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap()
}

/// A legacy edition `ISBN` of `work_id`, as the importer writes it (never checked).
async fn legacy_edition(app: &TestApp, work_id: i64) -> i64 {
    let edition = EditionRecord {
        isbn13: parse_isbn(ISBN),
        ..EditionRecord::new(Source::Legacy)
    };
    let mut conn = app.db.acquire().await.unwrap();
    ingest::upsert_edition(&mut conn, work_id, &edition, Mode::Refresh)
        .await
        .unwrap()
        .1
}

async fn original_of(app: &TestApp, edition_id: i64) -> (Option<String>, bool) {
    sqlx::query_as(
        "SELECT original_title, original_title_checked_at IS NOT NULL FROM editions WHERE id = $1",
    )
    .bind(edition_id)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

async fn search_extra(app: &TestApp, work_id: i64) -> String {
    sqlx::query_scalar("SELECT search_extra FROM works WHERE id = $1")
        .bind(work_id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// A DNB record stores the original title and makes the work findable by it; a later record
/// without 240 clears it (DNB records are complete); another source never touches it.
#[sqlx::test]
async fn dnb_ingest_stores_the_original_title_and_search_finds_it(db: PgPool) {
    let app = TestApp::new(db).await;
    let (work_id, edition_id) = ingest_dnb(&app, Some("The name of the wind")).await;
    assert_eq!(
        original_of(&app, edition_id).await,
        (Some("The name of the wind".into()), true)
    );
    assert_eq!(search_extra(&app, work_id).await, "The name of the wind");
    let found = app
        .scalar_i64(
            "SELECT count(*) FROM works
             WHERE bc_search_text(title, author_names, search_extra) LIKE '%name of the wind%'",
        )
        .await;
    assert_eq!(found, 1);

    let other = EditionRecord {
        isbn13: parse_isbn(ISBN),
        original_title: Some("Something else".into()),
        pages: Some(800),
        ..EditionRecord::new(Source::OpenLibrary)
    };
    let mut conn = app.db.acquire().await.unwrap();
    ingest::upsert_edition(&mut conn, work_id, &other, Mode::Refresh)
        .await
        .unwrap();
    drop(conn);
    assert_eq!(
        original_of(&app, edition_id).await.0.as_deref(),
        Some("The name of the wind"),
        "only the DNB tells original titles"
    );

    ingest_dnb(&app, None).await;
    assert_eq!(original_of(&app, edition_id).await, (None, true));
    assert_eq!(search_extra(&app, work_id).await, "");
}

/// An imported book with a German ISBN that Open Library does not know by its German title gets
/// the original's cover: the DNB check runs once, the second request asks by the original title
/// after the first found nothing, and the work becomes findable by it.
#[sqlx::test]
async fn the_cover_search_asks_by_the_original_title(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = legacy_work(&app, "Der Name des Windes", "Patrick Rothfuss").await;
    let edition = legacy_edition(&app, work).await;
    assert_eq!(original_of(&app, edition).await, (None, false));
    app.catalog
        .on_original_title(ISBN, Ok(Some("The name of the wind")));
    app.catalog.on_covers(
        "The name of the wind",
        Some("Rothfuss"),
        answer(vec![doc(
            31,
            "The Name of the Wind",
            "",
            "Patrick Rothfuss",
            Some(3100),
            40,
        )]),
    );

    let report = run_job(&app).await;
    assert_eq!(report.attached, 1, "{report:?}");
    assert_eq!(
        cover_of(&app, work).await,
        Some((3100, "openlibrary".into()))
    );
    assert_eq!(
        app.catalog.cover_calls(),
        [
            "cover:Der Name des Windes|Rothfuss|de",
            "cover:The name of the wind|Rothfuss|de"
        ]
    );
    assert_eq!(app.catalog.original_calls(), [ISBN]);
    assert_eq!(
        original_of(&app, edition).await,
        (Some("The name of the wind".into()), true)
    );
    assert_eq!(search_extra(&app, work).await, "The name of the wind");
}

/// The DNB check runs once per edition: a record without 240 is an answer too. Without an answer
/// (throttled or failing) it stays open, and that attempt goes on with the German title.
#[sqlx::test]
async fn the_dnb_check_runs_once_and_waits_without_an_answer(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = legacy_work(&app, "Der Name des Windes", "Patrick Rothfuss").await;
    let edition = legacy_edition(&app, work).await;

    app.catalog
        .on_original_title(ISBN, Err(FakeError::Unavailable));
    search_with(&app.db, app.catalog.as_ref(), work, true).await;
    assert_eq!(original_of(&app, edition).await, (None, false));
    assert_eq!(
        app.catalog.cover_calls(),
        ["cover:Der Name des Windes|Rothfuss|de"]
    );

    app.catalog.on_original_title(ISBN, Ok(None));
    search_with(&app.db, app.catalog.as_ref(), work, true).await;
    assert_eq!(original_of(&app, edition).await, (None, true));
    search_with(&app.db, app.catalog.as_ref(), work, true).await;
    assert_eq!(
        app.catalog.original_calls(),
        [ISBN, ISBN],
        "no third check after an answer"
    );
    assert_eq!(
        app.catalog.cover_calls().len(),
        3,
        "one request per attempt without an original title"
    );
}

/// A DNB edition ingested with this change was told its original title by its record: no check.
#[sqlx::test]
async fn a_dnb_edition_needs_no_check(db: PgPool) {
    let app = TestApp::new(db).await;
    let (work, _) = ingest_dnb(&app, None).await;
    use_work(&app, work).await;
    run_job(&app).await;
    assert!(app.catalog.original_calls().is_empty());
}

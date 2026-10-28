//! The frozen catalog interfaces against Postgres (docs/phase1/conventions.md §7): ingest with
//! field-level provenance, the BookOut/EditionOut loaders, WorkRef resolution and the local ISBN
//! step.

use bookclub_server::catalog::shared::books::{
    self, BookRef, book_outs, edition_outs, require_edition_of, require_work,
};
use bookclub_server::catalog::shared::dnb_marc::DnbRecord;
use bookclub_server::catalog::shared::ingest::{
    self, AuthorRecord, ContributorRole, CoverTarget, EditionRecord, Format, Mode, Source,
    WorkGrouping, WorkRecord,
};
use bookclub_server::catalog::shared::isbn::{Isbn13, parse_isbn};
use bookclub_server::catalog::shared::lookup::{Resolved, local_isbn};
use bookclub_server::catalog::shared::work_ref::{WorkRef, resolve_local};
use bookclub_server::dto::CatalogSource;
use bookclub_server::error::is_deadlock;
use bookclub_server::extract::Patch;
use serde_json::{Map, Value, json};
use sqlx::pool::PoolConnection;
use sqlx::postgres::Postgres;
use sqlx::{PgConnection, PgPool};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use time::macros::date;
use tokio::sync::Barrier;

const ISBN_A: &str = "9783895614194";
const ISBN_B: &str = "9783312015689";
const ISBN_C: &str = "9783648139776";

fn isbn(raw: &str) -> Isbn13 {
    parse_isbn(raw).expect("valid test ISBN")
}

async fn conn(db: &PgPool) -> PoolConnection<Postgres> {
    db.acquire().await.unwrap()
}

/// The whole row as JSON, for assertions.
async fn row(db: &PgPool, table: &str, id: i64) -> Value {
    let sql = match table {
        "works" => "SELECT to_jsonb(t) FROM works t WHERE id = $1",
        "editions" => "SELECT to_jsonb(t) FROM editions t WHERE id = $1",
        "media" => "SELECT to_jsonb(t) FROM media t WHERE id = $1",
        _ => panic!("no row helper for {table}"),
    };
    sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap()
}

async fn scalar_i64(db: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(db).await.unwrap()
}

/// An uploaded cover media row, taken down or not.
async fn upload_media(db: &PgPool, public_id: &str, removed: bool) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO media (public_id, kind, source, removed_at)
         VALUES ($1, 'cover', 'upload', CASE WHEN $2 THEN now() END) RETURNING id",
    )
    .bind(public_id)
    .bind(removed)
    .fetch_one(db)
    .await
    .unwrap()
}

fn dnb_work(title: &str, authors: &[(&str, &str, Option<&str>)]) -> WorkRecord {
    let authors: Vec<AuthorRecord> = authors
        .iter()
        .map(|(name, sort, gnd)| AuthorRecord {
            name: (*name).into(),
            sort_name: Some((*sort).into()),
            ol_author_id: None,
            gnd_id: gnd.map(Into::into),
        })
        .collect();
    WorkRecord {
        author_names: authors
            .iter()
            .map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join(", "),
        authors,
        ..WorkRecord::new(Source::Dnb, title)
    }
}

fn dnb_edition(isbn13: &str, idn: &str, title: &str) -> EditionRecord {
    EditionRecord {
        isbn13: Some(isbn(isbn13)),
        dnb_idn: Some(idn.into()),
        title: Some(title.into()),
        ..EditionRecord::new(Source::Dnb)
    }
}

fn ol_work(ol_id: &str, title: &str) -> WorkRecord {
    WorkRecord {
        ol_work_id: Some(ol_id.into()),
        ..WorkRecord::new(Source::OpenLibrary, title)
    }
}

fn fields(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

// ---------------------------------------------------------------------------------------------
// Provenance: one test per rank pair (owner of the field → writer of the new value).
// ---------------------------------------------------------------------------------------------

/// An edition whose `pages` (100) and `publisher` ("Owner") were written by `owner`; its
/// `language` is empty.
async fn edition_owned_by(db: &PgPool, owner: Source) -> (i64, i64) {
    let mut c = conn(db).await;
    let work = ingest::upsert_work(
        &mut c,
        &WorkRecord::new(Source::User, "Circe"),
        Mode::Refresh,
    )
    .await
    .unwrap();
    let base = EditionRecord {
        isbn13: Some(isbn(ISBN_A)),
        ..EditionRecord::new(if owner == Source::Admin {
            Source::User
        } else {
            owner
        })
    };
    if owner == Source::Admin {
        let (_, edition) = ingest::upsert_edition(&mut c, work, &base, Mode::Refresh)
            .await
            .unwrap();
        ingest::apply_admin_fields(
            &mut c,
            CoverTarget::Edition(edition),
            &fields(json!({"pages": 100, "publisher": "Owner"})),
        )
        .await
        .unwrap();
        return (work, edition);
    }
    let rec = EditionRecord {
        pages: Some(100),
        publisher: Some("Owner".into()),
        ..base
    };
    ingest::upsert_edition(&mut c, work, &rec, Mode::Refresh)
        .await
        .unwrap()
}

/// `writer` sends pages 200, publisher "Writer" and language "ger" for the edition `owner` wrote.
async fn rank_case(db: PgPool, owner: Source, writer: Source, overwrites: bool) {
    let (work, edition) = edition_owned_by(&db, owner).await;
    let rec = EditionRecord {
        isbn13: Some(isbn(ISBN_A)),
        pages: Some(200),
        publisher: Some("Writer".into()),
        language: Some("ger".into()),
        ..EditionRecord::new(writer)
    };
    let mut c = conn(&db).await;
    let found = ingest::upsert_edition(&mut c, work, &rec, Mode::Refresh)
        .await
        .unwrap();
    assert_eq!(found, (work, edition), "found by ISBN");

    let e = row(&db, "editions", edition).await;
    let (pages, publisher, by) = if overwrites {
        (200, "Writer", writer)
    } else {
        (100, "Owner", owner)
    };
    assert_eq!(e["pages"], pages, "{owner:?} → {writer:?}: {e}");
    assert_eq!(e["publisher"], publisher);
    assert_eq!(e["field_sources"]["pages"], by.as_str());
    assert_eq!(e["field_sources"]["publisher"], by.as_str());
    // An empty field is always filled, whoever owns the rest of the row.
    assert_eq!(e["language"], "ger");
    assert_eq!(e["field_sources"]["language"], writer.as_str());
}

#[sqlx::test]
async fn legacy_field_is_overwritten_by_legacy(db: PgPool) {
    rank_case(db, Source::Legacy, Source::Legacy, true).await;
}

#[sqlx::test]
async fn legacy_field_is_overwritten_by_open_library(db: PgPool) {
    rank_case(db, Source::Legacy, Source::OpenLibrary, true).await;
}

#[sqlx::test]
async fn legacy_field_is_overwritten_by_dnb(db: PgPool) {
    rank_case(db, Source::Legacy, Source::Dnb, true).await;
}

#[sqlx::test]
async fn legacy_field_is_overwritten_by_user(db: PgPool) {
    rank_case(db, Source::Legacy, Source::User, true).await;
}

#[sqlx::test]
async fn open_library_field_is_kept_against_legacy(db: PgPool) {
    rank_case(db, Source::OpenLibrary, Source::Legacy, false).await;
}

#[sqlx::test]
async fn open_library_field_is_overwritten_by_dnb_at_equal_rank(db: PgPool) {
    rank_case(db, Source::OpenLibrary, Source::Dnb, true).await;
}

#[sqlx::test]
async fn dnb_field_is_overwritten_by_open_library_at_equal_rank(db: PgPool) {
    rank_case(db, Source::Dnb, Source::OpenLibrary, true).await;
}

#[sqlx::test]
async fn dnb_field_is_overwritten_by_dnb(db: PgPool) {
    rank_case(db, Source::Dnb, Source::Dnb, true).await;
}

#[sqlx::test]
async fn dnb_field_is_overwritten_by_user(db: PgPool) {
    rank_case(db, Source::Dnb, Source::User, true).await;
}

#[sqlx::test]
async fn user_field_is_kept_against_dnb(db: PgPool) {
    rank_case(db, Source::User, Source::Dnb, false).await;
}

#[sqlx::test]
async fn user_field_is_kept_against_open_library(db: PgPool) {
    rank_case(db, Source::User, Source::OpenLibrary, false).await;
}

#[sqlx::test]
async fn user_field_is_kept_against_legacy(db: PgPool) {
    rank_case(db, Source::User, Source::Legacy, false).await;
}

#[sqlx::test]
async fn user_field_is_overwritten_by_user(db: PgPool) {
    rank_case(db, Source::User, Source::User, true).await;
}

#[sqlx::test]
async fn admin_field_is_kept_against_user(db: PgPool) {
    rank_case(db, Source::Admin, Source::User, false).await;
}

#[sqlx::test]
async fn admin_field_is_kept_against_dnb(db: PgPool) {
    rank_case(db, Source::Admin, Source::Dnb, false).await;
}

#[sqlx::test]
async fn admin_field_is_kept_against_open_library(db: PgPool) {
    rank_case(db, Source::Admin, Source::OpenLibrary, false).await;
}

#[sqlx::test]
async fn admin_field_is_kept_against_legacy(db: PgPool) {
    rank_case(db, Source::Admin, Source::Legacy, false).await;
}

#[sqlx::test]
async fn work_fields_follow_the_same_ranks(db: PgPool) {
    let mut c = conn(&db).await;
    // A reader added the work; Open Library later knows it by id.
    let user = WorkRecord {
        ol_work_id: Some("OL7W".into()),
        description: Some("Entered by a reader".into()),
        ..WorkRecord::new(Source::User, "Kirke")
    };
    let id = ingest::upsert_work(&mut c, &user, Mode::Refresh)
        .await
        .unwrap();
    let refresh = WorkRecord {
        description: Some("From Open Library".into()),
        pages: Some(320),
        full_details: true,
        ..ol_work("OL7W", "Circe")
    };
    assert_eq!(
        ingest::upsert_work(&mut c, &refresh, Mode::Refresh)
            .await
            .unwrap(),
        id
    );
    let w = row(&db, "works", id).await;
    assert_eq!(w["title"], "Kirke");
    assert_eq!(w["description"], "Entered by a reader");
    assert_eq!(w["pages"], 320, "an empty field is filled");
    assert_eq!(w["field_sources"]["pages"], "openlibrary");
    assert_eq!(w["source"], "user", "user rows keep their origin");
    assert!(!w["details_fetched_at"].is_null());
}

// ---------------------------------------------------------------------------------------------
// Modes
// ---------------------------------------------------------------------------------------------

#[sqlx::test]
async fn insert_only_returns_existing_rows_untouched(db: PgPool) {
    let mut c = conn(&db).await;
    let first = WorkRecord {
        pages: Some(300),
        ..ol_work("OL1W", "Circe")
    };
    let id = ingest::upsert_work(&mut c, &first, Mode::InsertOnly)
        .await
        .unwrap();
    let second = WorkRecord {
        pages: Some(999),
        description: Some("New".into()),
        ..ol_work("OL1W", "Circe (renamed)")
    };
    assert_eq!(
        ingest::upsert_work(&mut c, &second, Mode::InsertOnly)
            .await
            .unwrap(),
        id
    );
    let w = row(&db, "works", id).await;
    assert_eq!(
        (w["title"].as_str(), w["pages"].as_i64()),
        (Some("Circe"), Some(300))
    );
    assert_eq!(w["description"], "");

    let (work, edition) = ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work("Titel", &[]),
        &EditionRecord {
            pages: Some(10),
            ..dnb_edition(ISBN_A, "1", "Titel")
        },
        WorkGrouping::NewWork,
        Mode::InsertOnly,
    )
    .await
    .unwrap();
    let again = ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work("Anders", &[]),
        &EditionRecord {
            pages: Some(20),
            ..dnb_edition(ISBN_A, "1", "Anders")
        },
        WorkGrouping::NewWork,
        Mode::InsertOnly,
    )
    .await
    .unwrap();
    assert_eq!(again, (work, edition));
    assert_eq!(row(&db, "editions", edition).await["pages"], 10);
    assert_eq!(row(&db, "works", work).await["title"], "Titel");
}

#[sqlx::test]
async fn refresh_overwrites_with_values_and_keeps_missing_ones(db: PgPool) {
    let mut c = conn(&db).await;
    let full = WorkRecord {
        description: Some("A witch on an island.".into()),
        subjects: Some(vec!["Mythology".into()]),
        pages: Some(300),
        ..ol_work("OL1W", "Circe")
    };
    let id = ingest::upsert_work(&mut c, &full, Mode::Refresh)
        .await
        .unwrap();
    let partial = WorkRecord {
        pages: Some(320),
        ..ol_work("OL1W", "Circe: A Novel")
    };
    ingest::upsert_work(&mut c, &partial, Mode::Refresh)
        .await
        .unwrap();
    let w = row(&db, "works", id).await;
    assert_eq!(w["title"], "Circe: A Novel");
    assert_eq!(w["pages"], 320);
    assert_eq!(w["description"], "A witch on an island.");
    assert_eq!(w["subjects"], json!(["Mythology"]));
}

#[sqlx::test]
async fn replace_clears_only_the_sources_own_fields(db: PgPool) {
    let mut c = conn(&db).await;
    let work = dnb_work("Titel", &[]);
    let full = EditionRecord {
        pages: Some(300),
        publisher: Some("Reclam".into()),
        format: Some(Format::Paperback),
        planned_publication_date: Patch::Value(date!(2026 - 09 - 01)),
        ..dnb_edition(ISBN_A, "1", "Titel")
    };
    let (work_id, edition) = ingest::upsert_edition_with_work(
        &mut c,
        &work,
        &full,
        WorkGrouping::NewWork,
        Mode::Replace,
    )
    .await
    .unwrap();
    // A reader fills the empty language; Open Library (equal rank) fills the empty subtitle.
    let user = EditionRecord {
        isbn13: Some(isbn(ISBN_A)),
        language: Some("ger".into()),
        ..EditionRecord::new(Source::User)
    };
    ingest::upsert_edition(&mut c, work_id, &user, Mode::Refresh)
        .await
        .unwrap();
    let open_library = EditionRecord {
        isbn13: Some(isbn(ISBN_A)),
        subtitle: Some("Roman".into()),
        ..EditionRecord::new(Source::OpenLibrary)
    };
    ingest::upsert_edition(&mut c, work_id, &open_library, Mode::Refresh)
        .await
        .unwrap();

    // Refresh keeps what the record lacks.
    let bare = dnb_edition(ISBN_A, "1", "Titel");
    ingest::upsert_edition_with_work(&mut c, &work, &bare, WorkGrouping::NewWork, Mode::Refresh)
        .await
        .unwrap();
    assert_eq!(row(&db, "editions", edition).await["pages"], 300);

    // Replace clears DNB's own fields, keeps the reader's, Open Library's (same rank, another
    // source) and the planned date (Missing).
    ingest::upsert_edition_with_work(&mut c, &work, &bare, WorkGrouping::NewWork, Mode::Replace)
        .await
        .unwrap();
    let e = row(&db, "editions", edition).await;
    assert_eq!(e["pages"], Value::Null);
    assert_eq!(e["publisher"], "");
    assert_eq!(e["format"], Value::Null);
    assert_eq!(e["language"], "ger");
    assert_eq!(e["subtitle"], "Roman");
    assert_eq!(e["planned_publication_date"], "2026-09-01");
    assert_eq!(
        e["field_sources"],
        json!({"language": "user", "subtitle": "openlibrary",
               "planned_publication_date": "dnb"})
    );

    // Null clears the planned date.
    let cleared = EditionRecord {
        planned_publication_date: Patch::Null,
        ..bare
    };
    ingest::upsert_edition_with_work(
        &mut c,
        &work,
        &cleared,
        WorkGrouping::NewWork,
        Mode::Replace,
    )
    .await
    .unwrap();
    assert_eq!(
        row(&db, "editions", edition).await["planned_publication_date"],
        Value::Null
    );
}

// ---------------------------------------------------------------------------------------------
// Works from DNB and legacy facts
// ---------------------------------------------------------------------------------------------

#[sqlx::test]
async fn dnb_records_create_works_with_authors_and_contributors(db: PgPool) {
    let mut c = conn(&db).await;
    let work = dnb_work(
        "Ich kann nicht vergeben",
        &[("Rudolf Vrba", "Vrba, Rudolf", Some("119505584"))],
    );
    let contributor = |name: &str, sort: &str, gnd: Option<&str>| AuthorRecord {
        name: name.into(),
        sort_name: Some(sort.into()),
        ol_author_id: None,
        gnd_id: gnd.map(Into::into),
    };
    let hardcover = EditionRecord {
        format: Some(Format::Hardcover),
        pages: Some(496),
        publication_year: Some(2026),
        contributors: vec![
            (
                contributor("Sigrid Ruschmeier", "Ruschmeier, Sigrid", Some("133971783")),
                ContributorRole::Translator,
            ),
            (
                contributor("Jeanette Mey", "Mey, Jeanette", None),
                ContributorRole::Translator,
            ),
            (
                contributor("Dagi Knellessen", "Knellessen, Dagi", Some("1194960073")),
                ContributorRole::Editor,
            ),
        ],
        ..dnb_edition(ISBN_A, "1209150832", "Ich kann nicht vergeben")
    };
    let (work_id, edition) = ingest::upsert_edition_with_work(
        &mut c,
        &work,
        &hardcover,
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["source"], "dnb");
    assert_eq!(w["author_names"], "Rudolf Vrba");
    assert_eq!(w["match_key"], "ich kann nicht vergeben\u{1f}vrba");
    assert_eq!(w["first_publish_year"], 2026, "filled from the edition");
    assert_eq!(w["pages"], 496);
    assert_eq!(
        w["field_sources"],
        json!({"title": "dnb", "author_names": "dnb", "first_publish_year": "dnb", "pages": "dnb"})
    );
    let linked: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT a.name, a.sort_name, a.gnd_id FROM work_authors wa JOIN authors a ON a.id = wa.author_id
         WHERE wa.work_id = $1 ORDER BY wa.position",
    )
    .bind(work_id)
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(
        linked,
        [(
            "Rudolf Vrba".into(),
            Some("Vrba, Rudolf".into()),
            Some("119505584".into())
        )]
    );
    let out = edition_outs(&mut c, &[edition]).await.unwrap();
    let names: Vec<(&str, &str)> = out[&edition]
        .contributors
        .iter()
        .map(|c| (c.name.as_str(), c.role.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("Sigrid Ruschmeier", "translator"),
            ("Jeanette Mey", "translator"),
            ("Dagi Knellessen", "editor")
        ]
    );
    assert_eq!(out[&edition].title, None, "same as the work title");
    assert_eq!(out[&edition].format.as_deref(), Some("hardcover"));

    // The paperback is a second record with the same title and author: same work, same authors.
    let paperback = EditionRecord {
        format: Some(Format::Paperback),
        contributors: vec![(
            contributor("Jeanette Mey", "Mey, Jeanette", None),
            ContributorRole::Translator,
        )],
        ..dnb_edition(ISBN_B, "1209150840", "Ich kann  nicht vergeben")
    };
    let (same_work, other) = ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work(
            "ICH KANN NICHT VERGEBEN",
            &[("Rudolf Vrba", "Vrba, Rudolf", Some("119505584"))],
        ),
        &paperback,
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    assert_eq!(same_work, work_id);
    assert_ne!(other, edition);
    assert_eq!(scalar_i64(&db, "SELECT count(*) FROM authors").await, 4);
    assert_eq!(
        scalar_i64(
            &db,
            "SELECT count(*) FROM authors WHERE name_key = 'jeanette mey'"
        )
        .await,
        1,
        "an id-less name is deduplicated by name_key"
    );
}

#[sqlx::test]
async fn legacy_bare_facts_become_a_work_and_are_upgraded_by_a_source(db: PgPool) {
    let mut c = conn(&db).await;
    let legacy_work = WorkRecord {
        author_names: "Udo Paulitz".into(),
        authors: vec![AuthorRecord::named("Udo Paulitz")],
        ..WorkRecord::new(Source::Legacy, "Traktoren (alter Titel)")
    };
    let legacy_edition = EditionRecord {
        isbn13: Some(isbn(ISBN_C)),
        pages: Some(250),
        publication_year: Some(2019),
        ..EditionRecord::new(Source::Legacy)
    };
    let (work_id, edition) = ingest::upsert_edition_with_work(
        &mut c,
        &legacy_work,
        &legacy_edition,
        WorkGrouping::NewWork,
        Mode::Refresh,
    )
    .await
    .unwrap();
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["source"], "legacy");
    assert_eq!(w["first_publish_year"], 2019);
    assert_eq!(w["pages"], 250);
    assert_eq!(w["field_sources"]["pages"], "legacy");
    assert_eq!(
        scalar_i64(&db, "SELECT count(*) FROM work_authors").await,
        1
    );

    // rematch-legacy: a DNB record for the ISBN upgrades the bare facts.
    let dnb = EditionRecord {
        pages: Some(288),
        publisher: Some("GeraMond".into()),
        ..dnb_edition(ISBN_C, "1168979633", "Traktoren")
    };
    let found = ingest::upsert_edition_with_work(
        &mut c,
        &WorkRecord {
            subtitle: Some("Marktpreise".into()),
            ..dnb_work("Traktoren", &[("Udo Paulitz", "Paulitz, Udo", None)])
        },
        &dnb,
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    assert_eq!(
        found,
        (work_id, edition),
        "the bare-facts edition is found by ISBN"
    );
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["source"], "dnb");
    assert_eq!(w["title"], "Traktoren");
    assert_eq!(w["subtitle"], "Marktpreise");
    assert_eq!(w["field_sources"]["title"], "dnb");
    assert_eq!(
        w["pages"], 250,
        "work pages are not part of the parent update"
    );
    let e = row(&db, "editions", edition).await;
    assert_eq!(e["source"], "dnb");
    assert_eq!(e["pages"], 288);
    assert_eq!(
        e["publication_year"], 2019,
        "kept: Replace clears only DNB's own fields"
    );
    assert_eq!(e["field_sources"]["pages"], "dnb");
    assert_eq!(e["field_sources"]["publication_year"], "legacy");
    assert_eq!(e["dnb_idn"], "1168979633");
    let sort: Option<String> = sqlx::query_scalar(
        "SELECT a.sort_name FROM work_authors wa JOIN authors a ON a.id = wa.author_id WHERE wa.work_id = $1",
    )
    .bind(work_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(sort.as_deref(), Some("Paulitz, Udo"));
}

#[sqlx::test]
async fn a_final_dnb_title_replaces_a_provisional_one_on_single_edition_works(db: PgPool) {
    let mut c = conn(&db).await;
    let author = [("Mario Giordano", "Giordano, Mario", Some("119538687"))];
    let (work_id, edition) = ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work("Arbeitstitel", &author),
        &dnb_edition(ISBN_A, "1", "Arbeitstitel"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work("Tante Poldi und der Gesang der Sirenen", &author),
        &dnb_edition(ISBN_A, "1", "Tante Poldi und der Gesang der Sirenen"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["title"], "Tante Poldi und der Gesang der Sirenen");
    assert_eq!(
        w["match_key"],
        "tante poldi und der gesang der sirenen\u{1f}giordano"
    );
    assert_eq!(row(&db, "editions", edition).await["title"], Value::Null);

    // With a second edition, a retitled record no longer renames the work.
    ingest::upsert_edition(
        &mut c,
        work_id,
        &dnb_edition(ISBN_B, "2", "Tante Poldi und der Gesang der Sirenen"),
        Mode::Replace,
    )
    .await
    .unwrap();
    ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work("Tante Poldi (Sonderausgabe)", &author),
        &dnb_edition(ISBN_A, "1", "Tante Poldi (Sonderausgabe)"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["title"], "Tante Poldi und der Gesang der Sirenen");
    assert_eq!(w["search_extra"], "Tante Poldi (Sonderausgabe)");
    assert_eq!(
        row(&db, "editions", edition).await["title"],
        "Tante Poldi (Sonderausgabe)"
    );
}

#[sqlx::test]
async fn an_open_library_work_keeps_its_title_for_a_german_edition(db: PgPool) {
    let mut c = conn(&db).await;
    let hobbit = WorkRecord {
        author_names: "J. R. R. Tolkien".into(),
        ..ol_work("OL27482W", "The Hobbit")
    };
    let work_id = ingest::upsert_work(&mut c, &hobbit, Mode::Refresh)
        .await
        .unwrap();
    let ol_edition = EditionRecord {
        isbn13: Some(isbn(ISBN_A)),
        ..EditionRecord::new(Source::OpenLibrary)
    };
    ingest::upsert_edition(&mut c, work_id, &ol_edition, Mode::Refresh)
        .await
        .unwrap();
    let found = ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work(
            "Der Hobbit",
            &[("J. R. R. Tolkien", "Tolkien, J. R. R.", None)],
        ),
        &dnb_edition(ISBN_A, "9", "Der Hobbit"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    assert_eq!(found.0, work_id);
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["title"], "The Hobbit");
    assert_eq!(w["source"], "openlibrary");
    assert_eq!(w["search_extra"], "Der Hobbit");
    assert_eq!(row(&db, "editions", found.1).await["title"], "Der Hobbit");
}

// ---------------------------------------------------------------------------------------------
// Rules ingest maintains (§7.3)
// ---------------------------------------------------------------------------------------------

#[sqlx::test]
async fn null_work_fields_are_filled_from_editions(db: PgPool) {
    let mut c = conn(&db).await;
    let work_id = ingest::upsert_work(
        &mut c,
        &WorkRecord::new(Source::User, "Kirke"),
        Mode::Refresh,
    )
    .await
    .unwrap();
    let planned = EditionRecord {
        isbn13: Some(isbn(ISBN_A)),
        pages: Some(300),
        planned_publication_date: Patch::Value(date!(2027 - 03 - 01)),
        ..EditionRecord::new(Source::Dnb)
    };
    let (_, first) = ingest::upsert_edition(&mut c, work_id, &planned, Mode::Refresh)
        .await
        .unwrap();
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["pages"], 300);
    assert_eq!(w["first_publish_year"], 2027, "from the planned date");
    assert_eq!(w["field_sources"]["pages"], "dnb");
    assert_eq!(w["field_sources"]["first_publish_year"], "dnb");
    assert_eq!(w["cover_media_id"], Value::Null);

    // A user photo of the edition becomes the work cover too, with the user's provenance.
    let photo = upload_media(&db, "aaaaaaaaaaaaaaaaaaaaaaaaaa", false).await;
    assert!(
        ingest::set_cover(
            &mut c,
            CoverTarget::Edition(first),
            photo,
            Source::User,
            true
        )
        .await
        .unwrap()
    );
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["cover_media_id"], photo);
    assert_eq!(w["field_sources"]["cover_media_id"], "user");

    // Only NULL fields are filled.
    let second = EditionRecord {
        isbn13: Some(isbn(ISBN_B)),
        pages: Some(500),
        publication_year: Some(2020),
        ..EditionRecord::new(Source::Dnb)
    };
    ingest::upsert_edition(&mut c, work_id, &second, Mode::Refresh)
        .await
        .unwrap();
    let w = row(&db, "works", work_id).await;
    assert_eq!(
        (w["pages"].as_i64(), w["first_publish_year"].as_i64()),
        (Some(300), Some(2027))
    );

    // publication_year wins over the planned date when both exist.
    let other = ingest::upsert_work(
        &mut c,
        &WorkRecord::new(Source::User, "Other"),
        Mode::Refresh,
    )
    .await
    .unwrap();
    let both = EditionRecord {
        isbn13: Some(isbn(ISBN_C)),
        publication_year: Some(2025),
        planned_publication_date: Patch::Value(date!(2026 - 01 - 01)),
        ..EditionRecord::new(Source::Legacy)
    };
    ingest::upsert_edition(&mut c, other, &both, Mode::Refresh)
        .await
        .unwrap();
    let w = row(&db, "works", other).await;
    assert_eq!(w["first_publish_year"], 2025);
    assert_eq!(w["field_sources"]["first_publish_year"], "legacy");
}

#[sqlx::test]
async fn edition_titles_feed_search_extra_and_authors_the_match_key(db: PgPool) {
    let mut c = conn(&db).await;
    let hobbit = WorkRecord {
        author_names: "J. R. R. Tolkien".into(),
        authors: vec![AuthorRecord {
            ol_author_id: Some("OL26320A".into()),
            ..AuthorRecord::named("J. R. R. Tolkien")
        }],
        ..ol_work("OL27482W", "The Hobbit")
    };
    let work_id = ingest::upsert_work(&mut c, &hobbit, Mode::Refresh)
        .await
        .unwrap();
    assert_eq!(
        row(&db, "works", work_id).await["match_key"],
        "the hobbit\u{1f}tolkien"
    );
    for (raw, title) in [
        (ISBN_A, "Der Hobbit"),
        (ISBN_B, "the  HOBBIT"),
        (ISBN_C, "der hobbit"),
        ("9780316769488", "Le Hobbit"),
    ] {
        let rec = EditionRecord {
            isbn13: Some(isbn(raw)),
            title: Some(title.into()),
            ..EditionRecord::new(Source::OpenLibrary)
        };
        ingest::upsert_edition(&mut c, work_id, &rec, Mode::Refresh)
            .await
            .unwrap();
    }
    let w = row(&db, "works", work_id).await;
    // "der hobbit" folds to "Der Hobbit"; "the  HOBBIT" to the work title.
    assert_eq!(w["search_extra"], "Der Hobbit / Le Hobbit");
    let found = scalar_i64(
        &db,
        "SELECT id FROM works WHERE bc_search_text(title, author_names, search_extra) ILIKE '%le hobbit%'",
    )
    .await;
    assert_eq!(found, work_id);
    // An edition title equal to the work title is not stored.
    assert_eq!(
        scalar_i64(&db, "SELECT count(*) FROM editions WHERE title IS NULL").await,
        1
    );
}

#[sqlx::test]
async fn dnb_grouping_matches_title_and_surname_among_dnb_works_only(db: PgPool) {
    let mut c = conn(&db).await;
    // An Open Library work with the same match key is never a DNB grouping target.
    let ol = WorkRecord {
        author_names: "Udo Paulitz".into(),
        ..ol_work("OL5W", "Traktoren")
    };
    let ol_id = ingest::upsert_work(&mut c, &ol, Mode::Refresh)
        .await
        .unwrap();
    let paulitz = [("Udo Paulitz", "Paulitz, Udo", None)];
    let (first, _) = ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work("Traktoren", &paulitz),
        &dnb_edition(ISBN_A, "1", "Traktoren"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    assert_ne!(first, ol_id);
    let (second, _) = ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work(" traktoren ", &[("U. Paulitz", "Paulitz, U.", None)]),
        &dnb_edition(ISBN_B, "2", "traktoren"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    assert_eq!(second, first, "same title and surname");
    let (third, _) = ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work("Traktoren", &[("Ada Lovelace", "Lovelace, Ada", None)]),
        &dnb_edition(ISBN_C, "3", "Traktoren"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    assert_ne!(third, first, "another author");
}

#[sqlx::test]
async fn removed_open_library_covers_never_come_back(db: PgPool) {
    let mut c = conn(&db).await;
    let media = ingest::ol_cover_media(&mut c, 123).await.unwrap().unwrap();
    assert_eq!(
        ingest::ol_cover_media(&mut c, 123).await.unwrap(),
        Some(media)
    );
    assert_eq!(ingest::ol_cover_media(&mut c, 0).await.unwrap(), None);
    assert_eq!(ingest::ol_cover_media(&mut c, -1).await.unwrap(), None);
    let m = row(&db, "media", media).await;
    assert_eq!(
        (m["kind"].as_str(), m["source"].as_str()),
        (Some("cover"), Some("openlibrary"))
    );
    assert_eq!(m["storage_key"], Value::Null);

    sqlx::query("UPDATE media SET removed_at = now() WHERE id = $1")
        .bind(media)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(ingest::ol_cover_media(&mut c, 123).await.unwrap(), None);
    let rec = WorkRecord {
        ol_cover_id: Some(123),
        ..ol_work("OL1W", "Circe")
    };
    let work_id = ingest::upsert_work(&mut c, &rec, Mode::Refresh)
        .await
        .unwrap();
    assert_eq!(
        row(&db, "works", work_id).await["cover_media_id"],
        Value::Null
    );
    assert!(
        !ingest::set_cover(
            &mut c,
            CoverTarget::Work(work_id),
            media,
            Source::Admin,
            false
        )
        .await
        .unwrap()
    );
    assert_eq!(scalar_i64(&db, "SELECT count(*) FROM media").await, 1);
}

#[sqlx::test]
async fn set_cover_only_if_empty_and_by_rank(db: PgPool) {
    let mut c = conn(&db).await;
    let work_id = ingest::upsert_work(&mut c, &ol_work("OL1W", "Circe"), Mode::Refresh)
        .await
        .unwrap();
    let first = upload_media(&db, "aaaaaaaaaaaaaaaaaaaaaaaaaa", false).await;
    let second = upload_media(&db, "bbbbbbbbbbbbbbbbbbbbbbbbbb", false).await;
    let removed = upload_media(&db, "cccccccccccccccccccccccccc", true).await;
    let target = CoverTarget::Work(work_id);

    assert!(
        !ingest::set_cover(&mut c, target, removed, Source::User, true)
            .await
            .unwrap()
    );
    assert!(
        ingest::set_cover(&mut c, target, first, Source::User, true)
            .await
            .unwrap()
    );
    // Someone else was first.
    assert!(
        !ingest::set_cover(&mut c, target, second, Source::User, true)
            .await
            .unwrap()
    );
    // A source never replaces a user's cover; a refresh with a cover id does not either.
    assert!(
        !ingest::set_cover(&mut c, target, second, Source::OpenLibrary, false)
            .await
            .unwrap()
    );
    let refresh = WorkRecord {
        ol_cover_id: Some(55),
        ..ol_work("OL1W", "Circe")
    };
    ingest::upsert_work(&mut c, &refresh, Mode::Refresh)
        .await
        .unwrap();
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["cover_media_id"], first);
    assert_eq!(w["field_sources"]["cover_media_id"], "user");
    // An approved cover_replace does.
    assert!(
        ingest::set_cover(&mut c, target, second, Source::Admin, false)
            .await
            .unwrap()
    );
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["cover_media_id"], second);
    assert_eq!(w["field_sources"]["cover_media_id"], "admin");
    assert!(
        !ingest::set_cover(&mut c, target, second, Source::Admin, false)
            .await
            .unwrap(),
        "unchanged"
    );
    assert!(
        !ingest::set_cover(
            &mut c,
            CoverTarget::Work(work_id + 100),
            first,
            Source::Admin,
            false
        )
        .await
        .unwrap()
    );
}

#[sqlx::test]
async fn an_open_library_cover_replaces_an_older_one(db: PgPool) {
    let mut c = conn(&db).await;
    let first = WorkRecord {
        ol_cover_id: Some(1),
        ..ol_work("OL1W", "Circe")
    };
    let work_id = ingest::upsert_work(&mut c, &first, Mode::Refresh)
        .await
        .unwrap();
    let old = row(&db, "works", work_id).await["cover_media_id"].clone();
    let newer = WorkRecord {
        ol_cover_id: Some(2),
        ..ol_work("OL1W", "Circe")
    };
    ingest::upsert_work(&mut c, &newer, Mode::Refresh)
        .await
        .unwrap();
    let w = row(&db, "works", work_id).await;
    assert_ne!(w["cover_media_id"], old);
    assert_eq!(w["field_sources"]["cover_media_id"], "openlibrary");
    // A record without a cover never removes one, even under Replace.
    ingest::upsert_work(&mut c, &ol_work("OL1W", "Circe"), Mode::Replace)
        .await
        .unwrap();
    assert_eq!(
        row(&db, "works", work_id).await["cover_media_id"],
        w["cover_media_id"]
    );
}

#[sqlx::test]
async fn clear_media_removes_every_reference(db: PgPool) {
    let mut c = conn(&db).await;
    let work_id = ingest::upsert_work(
        &mut c,
        &WorkRecord::new(Source::User, "Kirke"),
        Mode::Refresh,
    )
    .await
    .unwrap();
    let edition = EditionRecord {
        isbn13: Some(isbn(ISBN_A)),
        ..EditionRecord::new(Source::User)
    };
    let (_, edition_id) = ingest::upsert_edition(&mut c, work_id, &edition, Mode::Refresh)
        .await
        .unwrap();
    let media = upload_media(&db, "aaaaaaaaaaaaaaaaaaaaaaaaaa", false).await;
    ingest::set_cover(
        &mut c,
        CoverTarget::Edition(edition_id),
        media,
        Source::User,
        true,
    )
    .await
    .unwrap();
    assert_eq!(row(&db, "works", work_id).await["cover_media_id"], media);

    assert_eq!(ingest::clear_media(&mut c, media).await.unwrap(), 2);
    for (table, id) in [("works", work_id), ("editions", edition_id)] {
        let r = row(&db, table, id).await;
        assert_eq!(r["cover_media_id"], Value::Null, "{table}");
        assert_eq!(r["field_sources"].get("cover_media_id"), None, "{table}");
    }
}

#[sqlx::test]
async fn admin_fields_win_over_every_source(db: PgPool) {
    let mut c = conn(&db).await;
    let rec = WorkRecord {
        pages: Some(300),
        first_publish_year: Some(2018),
        author_names: "Madeline Miller".into(),
        authors: vec![AuthorRecord::named("Madeline Miller")],
        ..ol_work("OL1W", "Circe")
    };
    let work_id = ingest::upsert_work(&mut c, &rec, Mode::Refresh)
        .await
        .unwrap();
    ingest::apply_admin_fields(
        &mut c,
        CoverTarget::Work(work_id),
        &fields(json!({"title": "Circe (Fixed)", "pages": 333, "year": null,
                       "subjects": ["Greek myth"], "authors": "M. Miller"})),
    )
    .await
    .unwrap();
    // Null clears the field and keeps it the admin's.
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["first_publish_year"], Value::Null);
    assert_eq!(w["field_sources"]["first_publish_year"], "admin");
    assert_eq!(w["field_sources"]["pages"], "admin");
    ingest::upsert_work(&mut c, &rec, Mode::Replace)
        .await
        .unwrap();
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["title"], "Circe (Fixed)");
    assert_eq!(w["pages"], 333);
    assert_eq!(w["author_names"], "M. Miller");
    assert_eq!(w["subjects"], json!(["Greek myth"]));
    assert_eq!(w["field_sources"]["title"], "admin");
    // No source fills a field an admin cleared.
    assert_eq!(w["first_publish_year"], Value::Null);
    assert_eq!(w["field_sources"]["first_publish_year"], "admin");
    assert_eq!(
        scalar_i64(&db, "SELECT count(*) FROM work_authors").await,
        0
    );
    assert_eq!(w["match_key"], "circe (fixed)\u{1f}miller");

    // Every key is checked before anything is written.
    let before = row(&db, "works", work_id).await;
    let bad = ingest::apply_admin_fields(
        &mut c,
        CoverTarget::Work(work_id),
        &fields(json!({"pages": 5, "foo": 1})),
    )
    .await
    .unwrap_err();
    assert_eq!(
        (bad.status.as_u16(), bad.detail.as_ref()),
        (400, "Unknown field: foo")
    );
    assert_eq!(row(&db, "works", work_id).await, before);
    let bad = ingest::apply_admin_fields(
        &mut c,
        CoverTarget::Work(work_id),
        &fields(json!({"pages": 0})),
    )
    .await
    .unwrap_err();
    assert_eq!(
        (bad.status.as_u16(), bad.detail.as_ref()),
        (400, "Invalid value for pages")
    );
    let bad = ingest::apply_admin_fields(
        &mut c,
        CoverTarget::Work(work_id),
        &fields(json!({"title": null})),
    )
    .await
    .unwrap_err();
    assert_eq!(bad.status.as_u16(), 400);
    let missing = ingest::apply_admin_fields(&mut c, CoverTarget::Edition(999), &fields(json!({})))
        .await
        .unwrap_err();
    assert_eq!(
        (missing.status.as_u16(), missing.detail.as_ref()),
        (404, "No such book.")
    );

    let (_, edition) = ingest::upsert_edition(
        &mut c,
        work_id,
        &EditionRecord {
            isbn13: Some(isbn(ISBN_A)),
            publisher: Some("Bloomsbury".into()),
            pages: Some(400),
            ..EditionRecord::new(Source::OpenLibrary)
        },
        Mode::Refresh,
    )
    .await
    .unwrap();
    ingest::apply_admin_fields(
        &mut c,
        CoverTarget::Edition(edition),
        &fields(json!({"format": "paperback", "language": "ENG",
                       "planned_publication_date": "2026-09-01", "title": "Kirke",
                       "publisher": null, "pages": null})),
    )
    .await
    .unwrap();
    let e = row(&db, "editions", edition).await;
    assert_eq!(e["format"], "paperback");
    assert_eq!(e["language"], "eng");
    assert_eq!(e["planned_publication_date"], "2026-09-01");
    assert_eq!(e["field_sources"]["format"], "admin");
    assert_eq!((&e["publisher"], &e["pages"]), (&json!(""), &Value::Null));
    assert_eq!(e["field_sources"]["publisher"], "admin");
    assert_eq!(e["field_sources"]["pages"], "admin");
    assert_eq!(row(&db, "works", work_id).await["search_extra"], "Kirke");
    // A refresh of the record the admin corrected leaves the cleared fields empty.
    ingest::upsert_edition(
        &mut c,
        work_id,
        &EditionRecord {
            isbn13: Some(isbn(ISBN_A)),
            publisher: Some("Bloomsbury".into()),
            pages: Some(400),
            ..EditionRecord::new(Source::OpenLibrary)
        },
        Mode::Refresh,
    )
    .await
    .unwrap();
    let e = row(&db, "editions", edition).await;
    assert_eq!((&e["publisher"], &e["pages"]), (&json!(""), &Value::Null));
    assert_eq!(e["field_sources"]["publisher"], "admin");
    let before = row(&db, "editions", edition).await;
    for (body, detail) in [
        (json!({"format": "scroll"}), "Invalid value for format"),
        (
            json!({"pages": 5, "planned_publication_date": "3500-01-01"}),
            "Invalid value for planned_publication_date",
        ),
    ] {
        let bad = ingest::apply_admin_fields(&mut c, CoverTarget::Edition(edition), &fields(body))
            .await
            .unwrap_err();
        assert_eq!((bad.status.as_u16(), bad.detail.as_ref()), (400, detail));
    }
    assert_eq!(row(&db, "editions", edition).await, before);
}

#[sqlx::test]
async fn withdrawn_editions_come_back_with_a_live_record(db: PgPool) {
    let mut c = conn(&db).await;
    let work = dnb_work("Titel", &[]);
    let record = dnb_edition(ISBN_A, "1168979633", "Titel");
    let (_, edition) = ingest::upsert_edition_with_work(
        &mut c,
        &work,
        &record,
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    ingest::withdraw_dnb_edition(&mut c, "1168979633")
        .await
        .unwrap();
    ingest::withdraw_dnb_edition(&mut c, "unknown")
        .await
        .unwrap();
    assert!(!row(&db, "editions", edition).await["withdrawn_at"].is_null());
    assert_eq!(
        local_isbn(&mut c, &isbn(ISBN_A))
            .await
            .unwrap()
            .map(|r| r.edition_id),
        Some(Some(edition)),
        "a withdrawn edition still resolves"
    );
    ingest::upsert_edition_with_work(
        &mut c,
        &work,
        &record,
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    assert!(row(&db, "editions", edition).await["withdrawn_at"].is_null());
}

#[sqlx::test]
async fn the_newest_record_owns_the_idn(db: PgPool) {
    let mut c = conn(&db).await;
    let work = dnb_work("Titel", &[]);
    let (_, a) = ingest::upsert_edition_with_work(
        &mut c,
        &work,
        &dnb_edition(ISBN_A, "100", "Titel"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    let (_, b) = ingest::upsert_edition_with_work(
        &mut c,
        &work,
        &dnb_edition(ISBN_B, "200", "Titel"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    // A newer record for ISBN A carries IDN 200: it moves from B to A.
    ingest::upsert_edition_with_work(
        &mut c,
        &work,
        &dnb_edition(ISBN_A, "200", "Titel"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    assert_eq!(row(&db, "editions", a).await["dnb_idn"], "200");
    assert_eq!(row(&db, "editions", b).await["dnb_idn"], Value::Null);
    // An edition without an ISBN is found by its IDN, and gets the ISBN when it is free.
    let (_, c_id) = ingest::upsert_edition_with_work(
        &mut c,
        &work,
        &EditionRecord {
            isbn13: None,
            ..dnb_edition(ISBN_C, "300", "Titel")
        },
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    let found = ingest::upsert_edition_with_work(
        &mut c,
        &work,
        &dnb_edition(ISBN_C, "300", "Titel"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    assert_eq!(found.1, c_id);
    assert_eq!(row(&db, "editions", c_id).await["isbn13"], ISBN_C);
}

/// A DNB record `idn` with one edition per ISBN, as `map_record` gives it: the first edition
/// carries the IDN as `dnb_idn`, every edition carries it as `dnb_record_idn`.
fn dnb_record(idn: &str, isbns: &[&str]) -> DnbRecord {
    DnbRecord {
        idn: idn.into(),
        work: dnb_work("Titel", &[("Ada Autorin", "Autorin, Ada", None)]),
        editions: isbns
            .iter()
            .enumerate()
            .map(|(i, raw)| EditionRecord {
                dnb_idn: (i == 0).then(|| idn.into()),
                dnb_record_idn: Some(idn.into()),
                ..dnb_edition(raw, idn, "Titel")
            })
            .collect(),
    }
}

/// `(dnb_idn, dnb_record_idn, withdrawn)` of an edition.
async fn dnb_ids(db: &PgPool, edition: i64) -> (Value, Value, bool) {
    let e = row(db, "editions", edition).await;
    (
        e["dnb_idn"].clone(),
        e["dnb_record_idn"].clone(),
        !e["withdrawn_at"].is_null(),
    )
}

/// A deleted header withdraws every edition whose newest delivering record it names
/// (`editions.dnb_record_idn`): all ISBNs of a multi-ISBN record, but not an edition a newer
/// record delivered again. Re-delivering a record brings its editions back, one per ISBN.
#[sqlx::test]
async fn a_deleted_record_withdraws_the_editions_it_delivered_last(db: PgPool) {
    let mut c = conn(&db).await;
    let first =
        ingest::upsert_dnb_record(&mut c, &dnb_record("100", &[ISBN_A, ISBN_B]), Mode::Replace)
            .await
            .unwrap();
    let (a, b) = (first[0].1, first[1].1);
    assert_eq!(first[1].0, first[0].0, "one work");
    assert_eq!(dnb_ids(&db, a).await, (json!("100"), json!("100"), false));
    assert_eq!(dnb_ids(&db, b).await, (Value::Null, json!("100"), false));

    // A newer record delivers B first and a new ISBN C second.
    let second =
        ingest::upsert_dnb_record(&mut c, &dnb_record("200", &[ISBN_B, ISBN_C]), Mode::Replace)
            .await
            .unwrap();
    assert_eq!(second[0], (first[0].0, b));
    let c_id = second[1].1;
    assert_eq!(dnb_ids(&db, b).await, (json!("200"), json!("200"), false));
    assert_eq!(dnb_ids(&db, c_id).await, (Value::Null, json!("200"), false));

    // Deleting the older record withdraws only what it delivered last.
    ingest::withdraw_dnb_edition(&mut c, "100").await.unwrap();
    assert_eq!(dnb_ids(&db, a).await, (json!("100"), json!("100"), true));
    assert!(!dnb_ids(&db, b).await.2);
    assert!(!dnb_ids(&db, c_id).await.2);
    // Deleting the newer one withdraws both of its editions.
    ingest::withdraw_dnb_edition(&mut c, "200").await.unwrap();
    assert!(dnb_ids(&db, b).await.2);
    assert!(dnb_ids(&db, c_id).await.2);

    // The first record again: A and B are live and its own again; C stays withdrawn.
    let again =
        ingest::upsert_dnb_record(&mut c, &dnb_record("100", &[ISBN_A, ISBN_B]), Mode::Replace)
            .await
            .unwrap();
    assert_eq!(again, first);
    assert_eq!(dnb_ids(&db, a).await, (json!("100"), json!("100"), false));
    assert_eq!(dnb_ids(&db, b).await, (json!("200"), json!("100"), false));
    assert_eq!(dnb_ids(&db, c_id).await, (Value::Null, json!("200"), true));
    assert_eq!(scalar_i64(&db, "SELECT count(*) FROM editions").await, 3);
}

/// An edition that a newer record lists as a later ISBN keeps the older record's `dnb_idn`
/// (only a record that names it first moves that), but its `dnb_record_idn` becomes the newer
/// record's, even when nothing else about it changes. So deleting the older record leaves it
/// live, and deleting the newer one withdraws it (int.md decision 1: the rule
/// `dnb_idn = $1 OR dnb_record_idn = $1` would withdraw it with the older record).
#[sqlx::test]
async fn an_edition_relisted_second_by_a_newer_record_follows_that_record(db: PgPool) {
    const ISBN_D: &str = "9783161484100";
    let mut c = conn(&db).await;
    let first =
        ingest::upsert_dnb_record(&mut c, &dnb_record("100", &[ISBN_A, ISBN_B]), Mode::Replace)
            .await
            .unwrap();
    let (a, b) = (first[0].1, first[1].1);

    // Record 300 lists D first and A second, with the data A already has.
    let newer =
        ingest::upsert_dnb_record(&mut c, &dnb_record("300", &[ISBN_D, ISBN_A]), Mode::Replace)
            .await
            .unwrap();
    assert_eq!(newer[1], (first[0].0, a), "A is found by its ISBN");
    let d = newer[0].1;
    assert_eq!(dnb_ids(&db, a).await, (json!("100"), json!("300"), false));
    assert_eq!(dnb_ids(&db, d).await, (json!("300"), json!("300"), false));

    // Deleting the older record withdraws only B, which it still delivered last.
    ingest::withdraw_dnb_edition(&mut c, "100").await.unwrap();
    assert_eq!(dnb_ids(&db, a).await, (json!("100"), json!("300"), false));
    assert!(dnb_ids(&db, b).await.2);
    assert!(!dnb_ids(&db, d).await.2);
    // Deleting the newer one withdraws A with D.
    ingest::withdraw_dnb_edition(&mut c, "300").await.unwrap();
    assert!(dnb_ids(&db, a).await.2);
    assert!(dnb_ids(&db, d).await.2);
    assert_eq!(scalar_i64(&db, "SELECT count(*) FROM editions").await, 3);
}

/// Applies, in order, the migrations whose version `pick` selects, without sqlx's bookkeeping,
/// so that a test can write rows in the shape an older schema left them.
async fn apply_migrations(db: &PgPool, pick: impl Fn(i64) -> bool) {
    for migration in sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| pick(m.version))
    {
        sqlx::raw_sql(migration.sql.clone())
            .execute(db)
            .await
            .unwrap_or_else(|error| panic!("migration {}: {error}", migration.version));
    }
}

/// Migration 1200 on rows written before it, when only the first ISBN's edition of a DNB record
/// carried the IDN. The other editions of the record (no IDN, the record's 263 date) get the IDN
/// of the one edition of their work that carries an IDN and has their date, and are withdrawn
/// when an edition of their work with that date is withdrawn, as new-releases hid them before
/// (HAR decision 15). An edition whose date is not the DNB's, or whose date two records share,
/// keeps no record IDN.
#[sqlx::test(migrations = false)]
async fn migration_1200_backfills_the_other_isbns_of_a_record(db: PgPool) {
    apply_migrations(&db, |version| version < 1200).await;
    sqlx::raw_sql(
        r#"INSERT INTO works (id, title, source) VALUES (1, 'Titel', 'dnb'), (2, 'Zwei', 'dnb');
           INSERT INTO editions (id, work_id, dnb_idn, planned_publication_date, field_sources,
                                 source, withdrawn_at)
           VALUES (1, 1, '100', '2026-11-01', '{"planned_publication_date": "dnb"}', 'dnb', NULL),
                  (2, 1, NULL, '2026-11-01', '{"planned_publication_date": "dnb"}', 'dnb', NULL),
                  (3, 1, '200', '2026-12-01', '{"planned_publication_date": "dnb"}', 'dnb', now()),
                  (4, 1, NULL, '2026-12-01', '{"planned_publication_date": "dnb"}', 'dnb', NULL),
                  (5, 1, NULL, '2026-12-01', '{"planned_publication_date": "user"}', 'user', NULL),
                  (6, 1, NULL, NULL, '{}', 'dnb', NULL),
                  (7, 2, '300', '2027-01-01', '{"planned_publication_date": "dnb"}', 'dnb', NULL),
                  (8, 2, '301', '2027-01-01', '{"planned_publication_date": "dnb"}', 'dnb', NULL),
                  (9, 2, NULL, '2027-01-01', '{"planned_publication_date": "dnb"}', 'dnb', NULL);"#,
    )
    .execute(&db)
    .await
    .unwrap();
    apply_migrations(&db, |version| version >= 1200).await;

    let expected = [
        (1, (json!("100"), json!("100"), false)),
        (2, (Value::Null, json!("100"), false)),
        (3, (json!("200"), json!("200"), true)),
        (4, (Value::Null, json!("200"), true)),
        (5, (Value::Null, Value::Null, false)),
        (6, (Value::Null, Value::Null, false)),
        (7, (json!("300"), json!("300"), false)),
        (8, (json!("301"), json!("301"), false)),
        (9, (Value::Null, Value::Null, false)),
    ];
    for (edition, ids) in expected {
        assert_eq!(dnb_ids(&db, edition).await, ids, "edition {edition}");
    }
    // From now on a deleted header takes the backfilled editions with the first one.
    let mut c = conn(&db).await;
    ingest::withdraw_dnb_edition(&mut c, "100").await.unwrap();
    assert!(dnb_ids(&db, 1).await.2);
    assert!(dnb_ids(&db, 2).await.2);
}

#[sqlx::test]
async fn work_stubs_are_inserted_in_one_batch_in_input_order(db: PgPool) {
    let mut c = conn(&db).await;
    let existing = ingest::upsert_work(&mut c, &ol_work("OL2W", "Old title"), Mode::Refresh)
        .await
        .unwrap();
    let stubs = vec![
        WorkRecord {
            ol_cover_id: Some(11),
            author_names: "Madeline Miller".into(),
            first_publish_year: Some(2018),
            ..ol_work("OL1W", "Circe")
        },
        ol_work("OL2W", "New title"),
        WorkRecord::new(Source::User, "No Open Library id"),
        ol_work("OL1W", "Circe again"),
    ];
    let ids = ingest::upsert_work_stubs(&mut c, &stubs).await.unwrap();
    assert_eq!(ids.len(), 4);
    assert_eq!(ids[1], existing);
    assert_eq!(ids[3], ids[0], "duplicates in one page share a row");
    assert_eq!(row(&db, "works", existing).await["title"], "Old title");
    let circe = row(&db, "works", ids[0]).await;
    assert_eq!(circe["title"], "Circe");
    assert_eq!(circe["match_key"], "circe\u{1f}miller");
    assert_eq!(
        circe["field_sources"],
        json!({"title": "openlibrary", "author_names": "openlibrary",
               "first_publish_year": "openlibrary", "cover_media_id": "openlibrary"})
    );
    let cover: i64 = sqlx::query_scalar("SELECT ol_cover_id FROM media WHERE id = $1")
        .bind(circe["cover_media_id"].as_i64().unwrap())
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(cover, 11);
    assert_eq!(row(&db, "works", ids[2]).await["source"], "user");
    assert_eq!(
        ingest::upsert_work_stubs(&mut c, &stubs[..2])
            .await
            .unwrap(),
        ids[..2]
    );
}

#[sqlx::test]
async fn an_open_library_record_needs_its_id(db: PgPool) {
    let mut c = conn(&db).await;
    let rec = WorkRecord::new(Source::OpenLibrary, "Circe");
    let result = std::panic::AssertUnwindSafe(ingest::upsert_work(&mut c, &rec, Mode::Refresh));
    let outcome = futures::FutureExt::catch_unwind(result).await;
    // debug_assert in test builds; Err(Protocol) in release builds.
    assert!(outcome.is_err() || outcome.unwrap().is_err());
}

// ---------------------------------------------------------------------------------------------
// Loaders, references, the local ISBN step
// ---------------------------------------------------------------------------------------------

#[sqlx::test]
async fn book_outs_prefer_the_edition_context(db: PgPool) {
    let mut c = conn(&db).await;
    let hobbit = WorkRecord {
        author_names: "J. R. R. Tolkien".into(),
        pages: Some(310),
        first_publish_year: Some(1937),
        ..ol_work("OL27482W", "The Hobbit")
    };
    let work_id = ingest::upsert_work(&mut c, &hobbit, Mode::Refresh)
        .await
        .unwrap();
    let work_cover = upload_media(&db, "wwwwwwwwwwwwwwwwwwwwwwwwww", false).await;
    let edition_cover = upload_media(&db, "eeeeeeeeeeeeeeeeeeeeeeeeee", false).await;
    sqlx::query("UPDATE media SET dominant_color = CASE WHEN id = $1 THEN '#112233' ELSE '' END")
        .bind(work_cover)
        .execute(&db)
        .await
        .unwrap();
    ingest::set_cover(
        &mut c,
        CoverTarget::Work(work_id),
        work_cover,
        Source::User,
        true,
    )
    .await
    .unwrap();
    let german = EditionRecord {
        isbn13: Some(isbn(ISBN_A)),
        title: Some("Der Hobbit".into()),
        pages: Some(400),
        ..EditionRecord::new(Source::Dnb)
    };
    let (_, edition) = ingest::upsert_edition(&mut c, work_id, &german, Mode::Refresh)
        .await
        .unwrap();
    ingest::set_cover(
        &mut c,
        CoverTarget::Edition(edition),
        edition_cover,
        Source::User,
        true,
    )
    .await
    .unwrap();
    let other_work = ingest::upsert_work(
        &mut c,
        &WorkRecord::new(Source::User, "Other"),
        Mode::Refresh,
    )
    .await
    .unwrap();
    let (_, foreign) = ingest::upsert_edition(
        &mut c,
        other_work,
        &EditionRecord {
            isbn13: Some(isbn(ISBN_B)),
            pages: Some(1),
            ..EditionRecord::new(Source::User)
        },
        Mode::Refresh,
    )
    .await
    .unwrap();

    let plain = BookRef::work(work_id);
    let in_edition = BookRef {
        work_id,
        edition_id: Some(edition),
    };
    let wrong_edition = BookRef {
        work_id,
        edition_id: Some(foreign),
    };
    let missing = BookRef::work(other_work + 100);
    let outs = book_outs(&mut c, &[plain, in_edition, wrong_edition, missing])
        .await
        .unwrap();
    assert_eq!(outs.len(), 3);
    let p = &outs[&plain];
    assert_eq!(p.title, "The Hobbit");
    assert_eq!(p.ol_work_key.as_deref(), Some("/works/OL27482W"));
    assert_eq!((p.pages, p.year), (Some(310), Some(1937)));
    assert_eq!(
        p.cover_public_id.as_deref(),
        Some("wwwwwwwwwwwwwwwwwwwwwwwwww")
    );
    assert_eq!(
        p.cover_url.as_deref(),
        Some("/media/wwwwwwwwwwwwwwwwwwwwwwwwww/l")
    );
    assert_eq!(
        p.cover_thumb_url.as_deref(),
        Some("/media/wwwwwwwwwwwwwwwwwwwwwwwwww/s")
    );
    assert_eq!(p.spine_color.as_deref(), Some("#112233"));
    assert_eq!((p.source, p.custom), (CatalogSource::Openlibrary, false));
    let e = &outs[&in_edition];
    assert_eq!(e.title, "Der Hobbit");
    assert_eq!(e.pages, Some(400));
    assert_eq!(
        e.cover_public_id.as_deref(),
        Some("eeeeeeeeeeeeeeeeeeeeeeeeee")
    );
    assert_eq!(e.spine_color, None, "'' means no usable colour");
    assert_eq!(outs[&wrong_edition].title, "The Hobbit");
    assert_eq!(outs[&wrong_edition].pages, Some(310));

    // A taken-down cover is never served, even before references are cleared.
    sqlx::query("UPDATE media SET removed_at = now() WHERE id = $1")
        .bind(work_cover)
        .execute(&db)
        .await
        .unwrap();
    let p = books::book_out(&mut c, plain).await.unwrap().unwrap();
    assert_eq!((p.cover_url, p.cover_public_id), (None, None));
    assert!(books::book_out(&mut c, missing).await.unwrap().is_none());
    let custom = books::book_out(&mut c, BookRef::work(other_work))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((custom.source, custom.custom), (CatalogSource::User, true));
}

#[sqlx::test]
async fn edition_outs_carry_contributors_and_a_derived_isbn10(db: PgPool) {
    let mut c = conn(&db).await;
    let work_id = ingest::upsert_work(
        &mut c,
        &WorkRecord::new(Source::User, "Kirke"),
        Mode::Refresh,
    )
    .await
    .unwrap();
    let rec = EditionRecord {
        isbn13: Some(isbn("9791090636071")),
        publisher: Some("Éditions".into()),
        language: Some("fre".into()),
        format: Some(Format::Ebook),
        publication_year: Some(2020),
        contributors: vec![(
            AuthorRecord::named("Ada Translator"),
            ContributorRole::Translator,
        )],
        ..EditionRecord::new(Source::User)
    };
    let (_, french) = ingest::upsert_edition(&mut c, work_id, &rec, Mode::Refresh)
        .await
        .unwrap();
    let (_, german) = ingest::upsert_edition(
        &mut c,
        work_id,
        &EditionRecord {
            isbn13: Some(isbn("3-89561-419-X")),
            ..EditionRecord::new(Source::User)
        },
        Mode::Refresh,
    )
    .await
    .unwrap();
    let outs = edition_outs(&mut c, &[french, german, german + 100])
        .await
        .unwrap();
    assert_eq!(outs.len(), 2);
    let f = &outs[&french];
    assert_eq!(f.isbn13.as_deref(), Some("9791090636071"));
    assert_eq!(f.isbn10, None, "979 has no ISBN-10");
    assert_eq!(f.publisher, "Éditions");
    assert_eq!(f.format.as_deref(), Some("ebook"));
    assert_eq!(f.contributors.len(), 1);
    assert_eq!(f.contributors[0].role, "translator");
    assert_eq!(f.source, CatalogSource::User);
    let g = &outs[&german];
    assert_eq!(g.isbn10.as_deref(), Some("389561419X"));
    assert_eq!(g.publisher, "");
    assert!(g.contributors.is_empty());
}

#[sqlx::test]
async fn references_resolve_to_the_current_work(db: PgPool) {
    let mut c = conn(&db).await;
    let loser = ingest::upsert_work(
        &mut c,
        &WorkRecord::new(Source::User, "Kirke"),
        Mode::Refresh,
    )
    .await
    .unwrap();
    let winner = ingest::upsert_work(&mut c, &ol_work("OL1W", "Circe"), Mode::Refresh)
        .await
        .unwrap();
    let (_, edition) = ingest::upsert_edition(
        &mut c,
        loser,
        &EditionRecord {
            isbn13: Some(isbn(ISBN_A)),
            ..EditionRecord::new(Source::User)
        },
        Mode::Refresh,
    )
    .await
    .unwrap();
    for (key, book_id) in [
        ("BCdeadbeef01", 1),
        ("isbn080442957x", 2),
        ("GBzyTCAlFPjgYC", 3),
    ] {
        let Some(WorkRef::Legacy(legacy_key)) = WorkRef::parse(key) else {
            panic!("{key}")
        };
        sqlx::query(
            "INSERT INTO legacy_work_keys (legacy_key, legacy_book_id, work_id, match)
             VALUES ($1, $2, $3, 'user')",
        )
        .bind(legacy_key)
        .bind(book_id)
        .bind(loser)
        .execute(&db)
        .await
        .unwrap();
    }
    let resolve = |raw: &str| WorkRef::parse(raw).unwrap();
    assert_eq!(
        resolve_local(&mut c, &resolve(&loser.to_string()))
            .await
            .unwrap(),
        Some(loser)
    );
    assert_eq!(
        resolve_local(&mut c, &resolve("OL1W")).await.unwrap(),
        Some(winner)
    );
    assert_eq!(resolve_local(&mut c, &resolve("OL2W")).await.unwrap(), None);
    assert_eq!(
        resolve_local(&mut c, &resolve("ISBN080442957X"))
            .await
            .unwrap(),
        Some(loser)
    );
    assert_eq!(
        resolve_local(&mut c, &resolve("GBzyTCAlFPjgYC"))
            .await
            .unwrap(),
        Some(loser)
    );
    assert_eq!(
        resolve_local(&mut c, &resolve("GBZYTCALFPJGYC"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        resolve_local(&mut c, &resolve("999999")).await.unwrap(),
        None
    );

    sqlx::query("UPDATE works SET merged_into_id = $2 WHERE id = $1")
        .bind(loser)
        .bind(winner)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        resolve_local(&mut c, &resolve("BCdeadbeef01"))
            .await
            .unwrap(),
        Some(winner)
    );
    assert_eq!(require_work(&mut c, loser).await.unwrap(), winner);
    let missing = require_work(&mut c, winner + 100).await.unwrap_err();
    assert_eq!(
        (missing.status.as_u16(), missing.detail.as_ref()),
        (404, "No such book.")
    );
    assert_eq!(
        local_isbn(&mut c, &isbn(ISBN_A)).await.unwrap(),
        Some(Resolved {
            work_id: winner,
            edition_id: Some(edition)
        })
    );
    assert_eq!(local_isbn(&mut c, &isbn(ISBN_B)).await.unwrap(), None);

    require_edition_of(&mut c, loser, edition).await.unwrap();
    let wrong = require_edition_of(&mut c, winner, edition)
        .await
        .unwrap_err();
    assert_eq!(
        (wrong.status.as_u16(), wrong.detail.as_ref()),
        (400, "That edition belongs to another book")
    );
    assert_eq!(
        require_edition_of(&mut c, loser, edition + 100)
            .await
            .unwrap_err()
            .status
            .as_u16(),
        400
    );
}

// ---------------------------------------------------------------------------------------------
// Upgrades, merges and grouping edge cases
// ---------------------------------------------------------------------------------------------

/// A legacy bare-facts work with one edition of `raw`.
async fn legacy_book(c: &mut PgConnection, title: &str, raw: &str) -> (i64, i64) {
    ingest::upsert_edition_with_work(
        c,
        &WorkRecord {
            author_names: "Udo Paulitz".into(),
            ..WorkRecord::new(Source::Legacy, title)
        },
        &EditionRecord {
            isbn13: Some(isbn(raw)),
            ..EditionRecord::new(Source::Legacy)
        },
        WorkGrouping::NewWork,
        Mode::Refresh,
    )
    .await
    .unwrap()
}

/// What CAT persists for an Open Library ISBN hit: the work and the edition record.
fn open_library_hit(ol_id: &str, title: &str, raw: &str) -> (WorkRecord, EditionRecord) {
    (
        WorkRecord {
            author_names: "Udo Paulitz".into(),
            ..ol_work(ol_id, title)
        },
        EditionRecord {
            isbn13: Some(isbn(raw)),
            publisher: Some("GeraMond".into()),
            ..EditionRecord::new(Source::OpenLibrary)
        },
    )
}

#[sqlx::test]
async fn a_legacy_work_upgraded_by_open_library_takes_its_work_id(db: PgPool) {
    let mut c = conn(&db).await;
    let (work_id, edition) = legacy_book(&mut c, "Traktoren (alt)", ISBN_C).await;

    // rematch-legacy: an Open Library hit for the ISBN.
    let (work, ed) = open_library_hit("OL9W", "Traktoren", ISBN_C);
    let found =
        ingest::upsert_edition_with_work(&mut c, &work, &ed, WorkGrouping::NewWork, Mode::Refresh)
            .await
            .unwrap();
    assert_eq!(found, (work_id, edition));
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["source"], "openlibrary");
    assert_eq!(w["ol_work_id"], "OL9W");
    assert_eq!(w["title"], "Traktoren");
    assert_eq!(
        w["details_fetched_at"],
        Value::Null,
        "the first detail view fetches the full record"
    );
    assert_eq!(row(&db, "editions", edition).await["source"], "openlibrary");
    let out = books::book_out(&mut c, BookRef::work(work_id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(out.ol_work_key.as_deref(), Some("/works/OL9W"));
    // A search hit for that id finds the upgraded work instead of creating a second one.
    assert_eq!(
        ingest::upsert_work_stubs(&mut c, &[ol_work("OL9W", "Traktoren")])
            .await
            .unwrap(),
        [work_id]
    );
    assert_eq!(scalar_i64(&db, "SELECT count(*) FROM works").await, 1);
}

#[sqlx::test]
async fn a_legacy_work_stays_legacy_when_its_open_library_id_is_taken(db: PgPool) {
    let mut c = conn(&db).await;
    let existing = ingest::upsert_work(&mut c, &ol_work("OL9W", "Traktoren"), Mode::Refresh)
        .await
        .unwrap();
    let (work_id, edition) = legacy_book(&mut c, "Traktoren (alt)", ISBN_C).await;
    let (work, ed) = open_library_hit("OL9W", "Traktoren", ISBN_C);
    let found =
        ingest::upsert_edition_with_work(&mut c, &work, &ed, WorkGrouping::NewWork, Mode::Refresh)
            .await
            .unwrap();
    assert_eq!(found, (work_id, edition), "the bare-facts edition is found");
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["source"], "legacy", "a visible merge candidate");
    assert_eq!(w["ol_work_id"], Value::Null);
    assert_eq!(
        w["title"], "Traktoren",
        "its legacy-ranked fields are upgraded"
    );
    assert_eq!(w["field_sources"]["title"], "openlibrary");
    assert_eq!(row(&db, "works", existing).await["ol_work_id"], "OL9W");
}

#[sqlx::test]
async fn an_open_library_record_never_retitles_a_dnb_work(db: PgPool) {
    let mut c = conn(&db).await;
    let tolkien = [("J. R. R. Tolkien", "Tolkien, J. R. R.", None)];
    let (work_id, edition) = ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work("Der Hobbit", &tolkien),
        &dnb_edition(ISBN_A, "1", "Der Hobbit"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    // CAT persists an Open Library hit for the ISBN while DNB cools down.
    let found = ingest::upsert_edition_with_work(
        &mut c,
        &WorkRecord {
            author_names: "J.R.R. Tolkien".into(),
            ..ol_work("OL27482W", "The Hobbit")
        },
        &EditionRecord {
            isbn13: Some(isbn(ISBN_A)),
            ..EditionRecord::new(Source::OpenLibrary)
        },
        WorkGrouping::NewWork,
        Mode::Refresh,
    )
    .await
    .unwrap();
    assert_eq!(found, (work_id, edition));
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["title"], "Der Hobbit");
    assert_eq!(w["author_names"], "J. R. R. Tolkien");
    assert_eq!(w["field_sources"]["title"], "dnb");
    assert_eq!(w["source"], "dnb");
    assert_eq!(w["ol_work_id"], Value::Null);
    assert_eq!(scalar_i64(&db, "SELECT count(*) FROM works").await, 1);
}

/// Marks `loser` as merged into `winner`, as `catalog::merge_works` does when both keep their
/// Open Library ids.
async fn merge(db: &PgPool, loser: i64, winner: i64) {
    sqlx::query("UPDATE works SET merged_into_id = $2 WHERE id = $1")
        .bind(loser)
        .bind(winner)
        .execute(db)
        .await
        .unwrap();
}

#[sqlx::test]
async fn a_merged_away_open_library_id_resolves_to_the_surviving_work(db: PgPool) {
    let mut c = conn(&db).await;
    let winner = ingest::upsert_work(&mut c, &ol_work("OL1W", "Circe"), Mode::Refresh)
        .await
        .unwrap();
    let loser = ingest::upsert_work(&mut c, &ol_work("OL2W", "Circe (dup)"), Mode::Refresh)
        .await
        .unwrap();
    merge(&db, loser, winner).await;
    let loser_before = row(&db, "works", loser).await;

    let refresh = WorkRecord {
        description: Some("From the duplicate".into()),
        full_details: true,
        ..ol_work("OL2W", "Circe, again")
    };
    assert_eq!(
        ingest::upsert_work(&mut c, &refresh, Mode::Refresh)
            .await
            .unwrap(),
        winner
    );
    // An ISBN hit whose Open Library edition names the merged-away work.
    let (work_id, edition) = ingest::upsert_edition_with_work(
        &mut c,
        &refresh,
        &EditionRecord {
            isbn13: Some(isbn(ISBN_A)),
            ..EditionRecord::new(Source::OpenLibrary)
        },
        WorkGrouping::NewWork,
        Mode::Refresh,
    )
    .await
    .unwrap();
    assert_eq!(work_id, winner);
    // POST /api/shelf {work_id: loser, edition_id} resolves the work, then accepts the edition.
    let current = require_work(&mut c, loser).await.unwrap();
    require_edition_of(&mut c, current, edition).await.unwrap();
    assert_eq!(
        ingest::upsert_work_stubs(
            &mut c,
            &[ol_work("OL2W", "Circe"), ol_work("OL1W", "Circe")]
        )
        .await
        .unwrap(),
        [winner, winner]
    );
    // Neither row took the duplicate's record.
    assert_eq!(row(&db, "works", loser).await, loser_before);
    let w = row(&db, "works", winner).await;
    assert_eq!(
        (&w["title"], &w["description"]),
        (&json!("Circe"), &json!(""))
    );

    // A second merge makes a chain; every path follows it to the end.
    let last = ingest::upsert_work(&mut c, &ol_work("OL3W", "Circe: A Novel"), Mode::Refresh)
        .await
        .unwrap();
    merge(&db, winner, last).await;
    assert_eq!(require_work(&mut c, loser).await.unwrap(), last);
    assert_eq!(
        resolve_local(&mut c, &WorkRef::parse(&loser.to_string()).unwrap())
            .await
            .unwrap(),
        Some(last)
    );
    assert_eq!(
        resolve_local(&mut c, &WorkRef::parse("OL2W").unwrap())
            .await
            .unwrap(),
        Some(last)
    );
    assert_eq!(
        ingest::upsert_work_stubs(&mut c, &[ol_work("OL2W", "Circe")])
            .await
            .unwrap(),
        [last]
    );
    let (into, _) = ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work("Kirke", &[]),
        &dnb_edition(ISBN_B, "2", "Kirke"),
        WorkGrouping::Into(loser),
        Mode::Replace,
    )
    .await
    .unwrap();
    assert_eq!(into, last, "Into follows merges too");
    let missing = ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work("Kirke", &[]),
        &dnb_edition(ISBN_C, "3", "Kirke"),
        WorkGrouping::Into(last + 100),
        Mode::Replace,
    )
    .await;
    assert!(missing.is_err(), "Into a missing work");
    assert_eq!(scalar_i64(&db, "SELECT count(*) FROM editions").await, 2);
}

#[sqlx::test]
async fn dnb_grouping_skips_merged_away_works(db: PgPool) {
    let mut c = conn(&db).await;
    let paulitz = [("Udo Paulitz", "Paulitz, Udo", None)];
    let (first, _) = ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work("Traktoren", &paulitz),
        &dnb_edition(ISBN_A, "1", "Traktoren"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    let winner = ingest::upsert_work(
        &mut c,
        &WorkRecord::new(Source::User, "Traktoren"),
        Mode::Refresh,
    )
    .await
    .unwrap();
    merge(&db, first, winner).await;
    let (second, _) = ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work("Traktoren", &paulitz),
        &dnb_edition(ISBN_B, "2", "Traktoren"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    assert_ne!(second, first, "the merged-away work is no grouping target");
    assert_ne!(second, winner, "neither is a user work");
}

#[sqlx::test]
async fn authorless_dnb_records_are_never_grouped_by_title_alone(db: PgPool) {
    let mut c = conn(&db).await;
    let mut works = Vec::new();
    for (raw, idn) in [(ISBN_A, "1"), (ISBN_B, "2")] {
        let (work, _) = ingest::upsert_edition_with_work(
            &mut c,
            &dnb_work("Gedichte", &[]),
            &dnb_edition(raw, idn, "Gedichte"),
            WorkGrouping::DnbTitleAuthor,
            Mode::Replace,
        )
        .await
        .unwrap();
        works.push(work);
    }
    assert_ne!(works[0], works[1], "two anthologies with one title");
    // The same record again finds its edition, and so its work.
    let (again, _) = ingest::upsert_edition_with_work(
        &mut c,
        &dnb_work("Gedichte", &[]),
        &dnb_edition(ISBN_A, "1", "Gedichte"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    assert_eq!(again, works[0]);
}

#[sqlx::test]
async fn an_edition_of_another_work_keeps_its_work(db: PgPool) {
    let mut c = conn(&db).await;
    let (work, edition) = edition_owned_by(&db, Source::OpenLibrary).await;
    let other = ingest::upsert_work(
        &mut c,
        &WorkRecord::new(Source::User, "Other"),
        Mode::Refresh,
    )
    .await
    .unwrap();
    let rec = EditionRecord {
        isbn13: Some(isbn(ISBN_A)),
        ..EditionRecord::new(Source::User)
    };
    assert_eq!(
        ingest::upsert_edition(&mut c, other, &rec, Mode::Refresh)
            .await
            .unwrap(),
        (work, edition),
        "CAT answers 409"
    );
    assert_eq!(row(&db, "editions", edition).await["work_id"], work);
}

#[sqlx::test]
async fn planned_years_beyond_the_schema_never_reach_the_work(db: PgPool) {
    let mut c = conn(&db).await;
    let work_id = ingest::upsert_work(
        &mut c,
        &WorkRecord::new(Source::User, "Kirke"),
        Mode::Refresh,
    )
    .await
    .unwrap();
    // A record's out-of-range planned date is dropped.
    let rec = EditionRecord {
        isbn13: Some(isbn(ISBN_A)),
        planned_publication_date: Patch::Value(date!(3001 - 01 - 01)),
        ..EditionRecord::new(Source::Dnb)
    };
    let (_, edition) = ingest::upsert_edition(&mut c, work_id, &rec, Mode::Replace)
        .await
        .unwrap();
    assert_eq!(
        row(&db, "editions", edition).await["planned_publication_date"],
        Value::Null
    );
    // A stored one (editions.planned_publication_date has no CHECK) is not used for the year.
    sqlx::query("UPDATE editions SET planned_publication_date = '9999-12-01' WHERE id = $1")
        .bind(edition)
        .execute(&db)
        .await
        .unwrap();
    let pages = EditionRecord {
        pages: Some(100),
        ..rec
    };
    ingest::upsert_edition(&mut c, work_id, &pages, Mode::Refresh)
        .await
        .unwrap();
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["pages"], 100);
    assert_eq!(w["first_publish_year"], Value::Null);
    assert_eq!(w["field_sources"].get("first_publish_year"), None);
}

#[sqlx::test]
async fn nul_characters_never_reach_postgres(db: PgPool) {
    let mut c = conn(&db).await;
    let stub = WorkRecord {
        author_names: "Madeline\u{0} Miller".into(),
        ..ol_work("OL1W", "Cir\u{0}ce")
    };
    let ids = ingest::upsert_work_stubs(&mut c, &[stub]).await.unwrap();
    let w = row(&db, "works", ids[0]).await;
    assert_eq!(
        (&w["title"], &w["author_names"]),
        (&json!("Circe"), &json!("Madeline Miller"))
    );
    let full = WorkRecord {
        description: Some("A witch\u{0}.".into()),
        authors: vec![AuthorRecord::named("Madeline Miller\u{0}")],
        ..ol_work("OL1W", "Circe")
    };
    ingest::upsert_work(&mut c, &full, Mode::Refresh)
        .await
        .unwrap();
    assert_eq!(row(&db, "works", ids[0]).await["description"], "A witch.");
}

#[sqlx::test]
async fn set_cover_refuses_a_media_row_that_is_not_a_cover(db: PgPool) {
    let mut c = conn(&db).await;
    let work_id = ingest::upsert_work(&mut c, &ol_work("OL1W", "Circe"), Mode::Refresh)
        .await
        .unwrap();
    let avatar: i64 = sqlx::query_scalar(
        "INSERT INTO media (public_id, kind, source) VALUES ('aaaaaaaaaaaaaaaaaaaaaaaaaa', 'avatar', 'upload')
         RETURNING id",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(
        !ingest::set_cover(
            &mut c,
            CoverTarget::Work(work_id),
            avatar,
            Source::Admin,
            false
        )
        .await
        .unwrap()
    );
    assert_eq!(
        row(&db, "works", work_id).await["cover_media_id"],
        Value::Null
    );
}

// ---------------------------------------------------------------------------------------------
// Concurrency: callers run ingest in transactions (§7.2)
// ---------------------------------------------------------------------------------------------

async fn backend_pid(conn: &mut PgConnection) -> i32 {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(conn)
        .await
        .unwrap()
}

/// Polls `pg_stat_activity` until backend `pid` waits for a lock (advisory or row lock).
async fn wait_until_blocked(db: &PgPool, pid: i32) {
    for _ in 0..1000 {
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                            WHERE pid = $1 AND wait_event_type = 'Lock')",
        )
        .bind(pid)
        .fetch_one(db)
        .await
        .unwrap();
        if blocked {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("backend {pid} never waited for a lock");
}

#[sqlx::test]
async fn concurrent_dnb_ingests_of_one_title_share_one_work(db: PgPool) {
    let author = [("Mario Giordano", "Giordano, Mario", None)];
    let work = dnb_work("Tante Poldi", &author);
    // HAR's page transaction holds the hardcover's locks...
    let mut harvest = db.begin().await.unwrap();
    let (first_work, _) = ingest::upsert_edition_with_work(
        &mut harvest,
        &work,
        &dnb_edition(ISBN_A, "1", "Tante Poldi"),
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap();
    // ...while CAT's SRU lookup ingests the paperback.
    let mut lookup = db.begin().await.unwrap();
    let pid = backend_pid(&mut lookup).await;
    let paperback = dnb_edition(ISBN_B, "2", "Tante Poldi");
    let task = tokio::spawn(async move {
        let ids = ingest::upsert_edition_with_work(
            &mut lookup,
            &work,
            &paperback,
            WorkGrouping::DnbTitleAuthor,
            Mode::Replace,
        )
        .await
        .unwrap();
        lookup.commit().await.unwrap();
        ids
    });
    // The lookup waits for the match-key lock, then sees the committed work.
    wait_until_blocked(&db, pid).await;
    harvest.commit().await.unwrap();
    let (second_work, _) = task.await.unwrap();
    assert_eq!(second_work, first_work);
    assert_eq!(scalar_i64(&db, "SELECT count(*) FROM works").await, 1);
    assert_eq!(
        scalar_i64(&db, "SELECT count(DISTINCT work_id) FROM editions").await,
        1
    );
    assert_eq!(scalar_i64(&db, "SELECT count(*) FROM editions").await, 2);
}

#[sqlx::test]
async fn concurrent_ingests_of_one_isbn_make_one_edition(db: PgPool) {
    let work_id = {
        let mut c = conn(&db).await;
        ingest::upsert_work(
            &mut c,
            &WorkRecord::new(Source::User, "Kirke"),
            Mode::Refresh,
        )
        .await
        .unwrap()
    };
    // Without a transaction the advisory locks end with each statement, so the inserts race
    // and the losers merge into the winner's row (ON CONFLICT DO NOTHING).
    let start = Arc::new(Barrier::new(4));
    let tasks: Vec<_> = (0..4)
        .map(|i| {
            let (db, start) = (db.clone(), start.clone());
            tokio::spawn(async move {
                let mut c = db.acquire().await.unwrap();
                let rec = EditionRecord {
                    isbn13: Some(isbn(ISBN_A)),
                    pages: Some(100 + i),
                    ..EditionRecord::new(Source::OpenLibrary)
                };
                start.wait().await;
                ingest::upsert_edition(&mut c, work_id, &rec, Mode::Refresh).await
            })
        })
        .collect();
    let mut editions = HashSet::new();
    for task in tasks {
        let (work, edition) = task.await.unwrap().unwrap();
        assert_eq!(work, work_id);
        editions.insert(edition);
    }
    assert_eq!(editions.len(), 1);
    assert_eq!(scalar_i64(&db, "SELECT count(*) FROM editions").await, 1);
}

#[sqlx::test]
async fn transactions_over_several_records_can_deadlock_and_retry(db: PgPool) {
    let (first_work, second_work) = {
        let mut c = conn(&db).await;
        let first = ingest::upsert_work(&mut c, &WorkRecord::new(Source::User, "A"), Mode::Refresh)
            .await
            .unwrap();
        let second =
            ingest::upsert_work(&mut c, &WorkRecord::new(Source::User, "B"), Mode::Refresh)
                .await
                .unwrap();
        (first, second)
    };
    let rec = |raw: &str| EditionRecord {
        isbn13: Some(isbn(raw)),
        ..EditionRecord::new(Source::User)
    };
    // Two page transactions take the same two ISBN locks in opposite order.
    let mut one = db.begin().await.unwrap();
    ingest::upsert_edition(&mut one, first_work, &rec(ISBN_A), Mode::Refresh)
        .await
        .unwrap();
    let mut two = db.begin().await.unwrap();
    ingest::upsert_edition(&mut two, second_work, &rec(ISBN_B), Mode::Refresh)
        .await
        .unwrap();
    let pid = backend_pid(&mut one).await;
    let b = rec(ISBN_B);
    let task = tokio::spawn(async move {
        let result = ingest::upsert_edition(&mut one, first_work, &b, Mode::Refresh).await;
        (one, result)
    });
    wait_until_blocked(&db, pid).await;
    let second = ingest::upsert_edition(&mut two, second_work, &rec(ISBN_A), Mode::Refresh).await;
    let (one, first) = task.await.unwrap();
    // Postgres aborts exactly one of them; the caller sees is_deadlock and retries it.
    let failed = match (first, second) {
        (Err(e), Ok(_)) => {
            two.commit().await.unwrap();
            drop(one);
            e
        }
        (Ok(_), Err(e)) => {
            one.commit().await.unwrap();
            drop(two);
            e
        }
        other => panic!("expected one deadlock abort, got {other:?}"),
    };
    assert!(is_deadlock(&failed), "{failed}");
    assert!(!is_deadlock(&sqlx::Error::RowNotFound));
    assert_eq!(scalar_i64(&db, "SELECT count(*) FROM editions").await, 2);
}

#[sqlx::test]
async fn a_takedown_waits_for_the_ingest_that_attaches_its_cover(db: PgPool) {
    let media = {
        let mut c = conn(&db).await;
        ingest::ol_cover_media(&mut c, 77).await.unwrap().unwrap()
    };
    let photo = upload_media(&db, "pppppppppppppppppppppppppp", false).await;
    let photo_work = {
        let mut c = conn(&db).await;
        ingest::upsert_work(&mut c, &ol_work("OL2W", "Kirke"), Mode::Refresh)
            .await
            .unwrap()
    };
    // First an Open Library cover attached by ingest, then a first upload through set_cover.
    for (target, media_id) in [(None, media), (Some(photo_work), photo)] {
        // The ingest attaches the media row in an open transaction...
        let mut writer = db.begin().await.unwrap();
        let work_id = match target {
            None => ingest::upsert_work(
                &mut writer,
                &WorkRecord {
                    ol_cover_id: Some(77),
                    ..ol_work("OL1W", "Circe")
                },
                Mode::Refresh,
            )
            .await
            .unwrap(),
            Some(work_id) => {
                assert!(
                    ingest::set_cover(
                        &mut writer,
                        CoverTarget::Work(work_id),
                        media_id,
                        Source::User,
                        true
                    )
                    .await
                    .unwrap()
                );
                work_id
            }
        };
        // ...and the takedown's UPDATE waits for it, so clear_media sees the reference.
        let mut takedown = db.begin().await.unwrap();
        let pid = backend_pid(&mut takedown).await;
        let task = tokio::spawn(async move {
            sqlx::query("UPDATE media SET removed_at = now() WHERE id = $1")
                .bind(media_id)
                .execute(&mut *takedown)
                .await
                .unwrap();
            let cleared = ingest::clear_media(&mut takedown, media_id).await.unwrap();
            takedown.commit().await.unwrap();
            cleared
        });
        wait_until_blocked(&db, pid).await;
        writer.commit().await.unwrap();
        assert_eq!(task.await.unwrap(), 1);
        assert_eq!(
            row(&db, "works", work_id).await["cover_media_id"],
            Value::Null
        );
    }
}

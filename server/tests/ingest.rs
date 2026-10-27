//! The frozen catalog interfaces against Postgres (docs/phase1/conventions.md §7): ingest with
//! field-level provenance, the BookOut/EditionOut loaders, WorkRef resolution and the local ISBN
//! step.

use bookclub_server::catalog::shared::books::{
    self, BookRef, book_outs, edition_outs, require_edition_of, require_work,
};
use bookclub_server::catalog::shared::ingest::{
    self, AuthorRecord, ContributorRole, CoverTarget, EditionRecord, Format, Mode, Source,
    WorkGrouping, WorkRecord,
};
use bookclub_server::catalog::shared::isbn::{Isbn13, parse_isbn};
use bookclub_server::catalog::shared::lookup::{Resolved, local_isbn};
use bookclub_server::catalog::shared::work_ref::{WorkRef, resolve_local};
use bookclub_server::dto::CatalogSource;
use bookclub_server::extract::Patch;
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use sqlx::pool::PoolConnection;
use sqlx::postgres::Postgres;
use time::macros::date;

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
    // A reader fills the empty language.
    let user = EditionRecord {
        isbn13: Some(isbn(ISBN_A)),
        language: Some("ger".into()),
        ..EditionRecord::new(Source::User)
    };
    ingest::upsert_edition(&mut c, work_id, &user, Mode::Refresh)
        .await
        .unwrap();

    // Refresh keeps what the record lacks.
    let bare = dnb_edition(ISBN_A, "1", "Titel");
    ingest::upsert_edition_with_work(&mut c, &work, &bare, WorkGrouping::NewWork, Mode::Refresh)
        .await
        .unwrap();
    assert_eq!(row(&db, "editions", edition).await["pages"], 300);

    // Replace clears DNB's own fields, keeps the reader's and the planned date (Missing).
    ingest::upsert_edition_with_work(&mut c, &work, &bare, WorkGrouping::NewWork, Mode::Replace)
        .await
        .unwrap();
    let e = row(&db, "editions", edition).await;
    assert_eq!(e["pages"], Value::Null);
    assert_eq!(e["publisher"], "");
    assert_eq!(e["format"], Value::Null);
    assert_eq!(e["language"], "ger");
    assert_eq!(e["planned_publication_date"], "2026-09-01");
    assert_eq!(
        e["field_sources"],
        json!({"language": "user", "planned_publication_date": "dnb"})
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
    ingest::upsert_work(&mut c, &rec, Mode::Replace)
        .await
        .unwrap();
    let w = row(&db, "works", work_id).await;
    assert_eq!(w["title"], "Circe (Fixed)");
    assert_eq!(w["pages"], 333);
    assert_eq!(w["author_names"], "M. Miller");
    assert_eq!(w["subjects"], json!(["Greek myth"]));
    assert_eq!(w["field_sources"]["title"], "admin");
    // A cleared field may be filled again by any source.
    assert_eq!(w["first_publish_year"], 2018);
    assert_eq!(w["field_sources"]["first_publish_year"], "openlibrary");
    assert_eq!(
        scalar_i64(&db, "SELECT count(*) FROM work_authors").await,
        0
    );
    assert_eq!(w["match_key"], "circe (fixed)\u{1f}miller");

    let bad = ingest::apply_admin_fields(
        &mut c,
        CoverTarget::Work(work_id),
        &fields(json!({"foo": 1})),
    )
    .await
    .unwrap_err();
    assert_eq!(
        (bad.status.as_u16(), bad.detail.as_ref()),
        (400, "Unknown field: foo")
    );
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
                       "planned_publication_date": "2026-09-01", "title": "Kirke"})),
    )
    .await
    .unwrap();
    let e = row(&db, "editions", edition).await;
    assert_eq!(e["format"], "paperback");
    assert_eq!(e["language"], "eng");
    assert_eq!(e["planned_publication_date"], "2026-09-01");
    assert_eq!(e["field_sources"]["format"], "admin");
    assert_eq!(row(&db, "works", work_id).await["search_extra"], "Kirke");
    let bad = ingest::apply_admin_fields(
        &mut c,
        CoverTarget::Edition(edition),
        &fields(json!({"format": "scroll"})),
    )
    .await
    .unwrap_err();
    assert_eq!(bad.detail.as_ref(), "Invalid value for format");
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

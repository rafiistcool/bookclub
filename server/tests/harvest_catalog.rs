//! What the harvest does to the catalog (#42 acceptance) and `GET /api/books/new-releases`
//! (endpoint-map §5): harvested records become DNB editions, a deleted header withdraws its
//! edition, a re-delivered record retitles its work, an announced title is searchable and
//! resolvable locally, and new-releases lists the date window. Pages come from
//! `tests/fixtures/dnb_oai/` through wiremock.

mod common;

use std::time::Duration;

use bookclub_server::catalog::merge::merge_works;
use bookclub_server::catalog::shared::ingest::{self, CoverTarget};
use bookclub_server::catalog::shared::isbn::parse_isbn;
use bookclub_server::catalog::shared::lookup::{Resolved, local_isbn};
use bookclub_server::harvest::harvest_set;
use bookclub_server::harvest::new_releases::LANGUAGE_PATTERN;
use bookclub_server::harvest::oai::{self, Answer};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::Date;
use time::macros::date;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

const PAGE_1: &str = include_str!("fixtures/dnb_oai/reiheN_page1.xml");
const PAGE_2: &str = include_str!("fixtures/dnb_oai/reiheN_page2.xml");
const REIHE_A: &str = include_str!("fixtures/dnb_oai/reiheA_complete.xml");
const RETITLED: &str = include_str!("fixtures/dnb_oai/retitled.xml");
const DELETED: &str = include_str!("fixtures/dnb_oai/deleted.xml");

const SET_N: &str = "dnb-all:reiheN";
const SET_A: &str = "dnb:reiheA";

/// "Bondbreaker" by Mila Finch: announced in reiheN for March 2028 (263 `202803`).
const BONDBREAKER_IDN: &str = "1423074580";
const BONDBREAKER_ISBN: &str = "9783690513029";
const FINAL_TITLE: &str = "Bondbreaker – Bund der Schatten";
/// "Die kleine Glitzerblume" of `reiheA_complete.xml`: one record with two ISBNs, a German and
/// an Austrian co-publisher edition.
const GLITZERBLUME_IDN: &str = "1399063251";
const GLITZERBLUME_ISBNS: [&str; 2] = ["9783693370056", "9783903300989"];

fn xml(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.as_bytes().to_vec(), "text/xml;charset=UTF-8")
}

fn token_of(page: &str) -> String {
    match oai::parse_list_records(page).unwrap() {
        Answer::Page(page) => page.resumption_token.expect("the page has a token"),
        other => panic!("not a page: {other:?}"),
    }
}

/// Serves `pages` as one list of `set`: the first for the list request, each next one for the
/// token of the page before. Earlier mocks are dropped.
async fn serve(app: &TestApp, set: &str, pages: &[&str]) {
    app.upstream.reset().await;
    Mock::given(method("GET"))
        .and(path("/oai/repository"))
        .and(query_param("set", set))
        .respond_with(xml(pages[0]))
        .mount(&app.upstream)
        .await;
    for pair in pages.windows(2) {
        Mock::given(method("GET"))
            .and(path("/oai/repository"))
            .and(query_param("resumptionToken", token_of(pair[0])))
            .respond_with(xml(pair[1]))
            .mount(&app.upstream)
            .await;
    }
}

/// Harvests `pages` of `set` to the end of the list.
async fn harvest(app: &TestApp, set: &str, pages: &[&str]) {
    serve(app, set, pages).await;
    let mut conn = app.db.acquire().await.unwrap();
    let report = harvest_set(&app.state, &mut conn, set, &CancellationToken::new())
        .await
        .unwrap();
    assert!(report.completed);
}

#[derive(Debug)]
struct Edition {
    id: i64,
    work_id: i64,
    isbn13: Option<String>,
    language: Option<String>,
    publication_year: Option<i32>,
    planned_publication_date: Option<Date>,
    source: String,
    withdrawn: bool,
}

async fn edition_by_idn(app: &TestApp, idn: &str) -> Option<Edition> {
    sqlx::query_as!(
        Edition,
        r#"SELECT id, work_id, isbn13, language, publication_year, planned_publication_date,
                  source, withdrawn_at IS NOT NULL AS "withdrawn!"
           FROM editions WHERE dnb_idn = $1"#,
        idn
    )
    .fetch_optional(&app.db)
    .await
    .unwrap()
}

#[derive(Debug)]
struct Work {
    title: String,
    author_names: String,
    source: String,
    description: String,
}

async fn work(app: &TestApp, id: i64) -> Work {
    sqlx::query_as!(
        Work,
        "SELECT title, author_names, source, description FROM works WHERE id = $1",
        id
    )
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// Moves an edition's planned date to today (UTC, the test users' zone) + `days`.
async fn plan_in(app: &TestApp, edition_id: i64, days: i32) {
    sqlx::query!(
        "UPDATE editions SET planned_publication_date = (now() AT TIME ZONE 'UTC')::date + $2::int
         WHERE id = $1",
        edition_id,
        days
    )
    .execute(&app.db)
    .await
    .unwrap();
}

fn ids(page: &Value) -> Vec<i64> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["id"].as_i64().unwrap())
        .collect()
}

/// Harvested records become DNB editions and works with their CC0 facts; a placeholder year
/// (2099) counts as missing; the deleted header is never mapped, so its title is nowhere.
#[sqlx::test]
async fn harvested_records_become_dnb_editions(db: PgPool) {
    let app = TestApp::new(db).await;
    harvest(&app, SET_N, &[PAGE_1, PAGE_2]).await;

    let bond = edition_by_idn(&app, BONDBREAKER_IDN).await.unwrap();
    assert_eq!(bond.isbn13.as_deref(), Some(BONDBREAKER_ISBN));
    assert_eq!(bond.language.as_deref(), Some("ger"));
    assert_eq!(bond.source, "dnb");
    assert_eq!(
        bond.planned_publication_date,
        Some(time::macros::date!(2028 - 03 - 01))
    );
    assert!(!bond.withdrawn);
    let bond_work = work(&app, bond.work_id).await;
    assert_eq!(bond_work.title, "Bondbreaker");
    assert_eq!(bond_work.author_names, "Mila Finch");
    assert_eq!(bond_work.source, "dnb");
    assert_eq!(bond_work.description, "", "no VLB blurb");

    // Meister Eckhart: 264 $c 2099 is a placeholder; 263 announces November 2027.
    let eckhart = edition_by_idn(&app, "1289274576").await.unwrap();
    assert_eq!(eckhart.publication_year, None);
    assert_eq!(
        eckhart.planned_publication_date,
        Some(time::macros::date!(2027 - 11 - 01))
    );
    // An English-language title from reiheN.
    let yachting = edition_by_idn(&app, "1423078357").await.unwrap();
    assert_eq!(yachting.language.as_deref(), Some("eng"));

    // The deleted header (Traktor Oldtimer Katalog Nr. 8) inserted nothing.
    assert!(edition_by_idn(&app, "1168979633").await.is_none());
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM editions WHERE isbn13 = '9783868527926'")
            .await,
        0
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM works WHERE title LIKE 'Traktor%'")
            .await,
        0
    );
}

/// A record with two ISBNs of one registration group gives two editions of one work.
#[sqlx::test]
async fn a_multi_isbn_record_stays_one_work(db: PgPool) {
    let app = TestApp::new(db).await;
    harvest(&app, SET_A, &[REIHE_A]).await;
    let works: Vec<i64> = sqlx::query_scalar!(
        "SELECT DISTINCT work_id FROM editions WHERE isbn13 IN ('9783693370056', '9783903300989')"
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(works.len(), 1);
    assert_eq!(
        app.scalar_i64(
            "SELECT count(*) FROM editions WHERE isbn13 IN ('9783693370056', '9783903300989')"
        )
        .await,
        2
    );
}

/// No VLB text or URL is stored: the recorded records carry 856 links to blurbs, which the
/// mapping never reads.
#[sqlx::test]
async fn no_vlb_text_is_stored(db: PgPool) {
    let app = TestApp::new(db).await;
    assert!(PAGE_1.contains("services.dnb.de/plus/idn/1289274576/blurb/"));
    harvest(&app, SET_N, &[PAGE_1, PAGE_2]).await;
    harvest(&app, SET_A, &[REIHE_A]).await;
    assert!(app.scalar_i64("SELECT count(*) FROM works").await >= 10);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM works WHERE description <> ''")
            .await,
        0
    );
    for table in ["works", "editions", "authors"] {
        let sql = format!(
            "SELECT count(*) FROM {table} t WHERE t::text ILIKE '%http%' OR t::text ILIKE '%blurb%'"
        );
        assert_eq!(app.scalar_i64(&sql).await, 0, "{table}");
    }
}

/// Acceptance (#42): a deleted record withdraws its edition and drops it from new-releases; the
/// edition still resolves by ISBN, and a later live record for the IDN brings it back.
#[sqlx::test]
async fn a_deleted_record_withdraws_its_edition(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    harvest(&app, SET_N, &[PAGE_1, PAGE_2]).await;
    let bond = edition_by_idn(&app, BONDBREAKER_IDN).await.unwrap();
    plan_in(&app, bond.id, 10).await;
    let page = ada.get("/api/books/new-releases").await;
    page.assert_status(200);
    assert!(ids(&page.json()).contains(&bond.work_id));

    harvest(&app, SET_N, &[DELETED]).await;
    let withdrawn = edition_by_idn(&app, BONDBREAKER_IDN).await.unwrap();
    assert!(withdrawn.withdrawn);
    assert_eq!(withdrawn.id, bond.id);
    let page = ada.get("/api/books/new-releases").await;
    page.assert_status(200);
    assert!(!ids(&page.json()).contains(&bond.work_id));
    // The row stays: shelves may point at it, and it still resolves by ISBN.
    let mut conn = app.db.acquire().await.unwrap();
    let isbn = parse_isbn(BONDBREAKER_ISBN).unwrap();
    assert_eq!(
        local_isbn(&mut conn, &isbn).await.unwrap(),
        Some(Resolved {
            work_id: bond.work_id,
            edition_id: Some(bond.id)
        })
    );

    // The published title arrives in reiheA under the same IDN: the edition is live again.
    harvest(&app, SET_A, &[RETITLED]).await;
    assert!(
        !edition_by_idn(&app, BONDBREAKER_IDN)
            .await
            .unwrap()
            .withdrawn
    );
}

/// `reiheA_complete.xml` with the "Glitzerblume" record changed by `edit`, which gets that
/// record's text from its `<record>` to its closing `</metadata>`.
fn edit_glitzerblume(edit: impl FnOnce(&str) -> String) -> String {
    let start = REIHE_A
        .find(&format!(
            "<record><header><identifier>oai:dnb.de/dnb:reiheA/{GLITZERBLUME_IDN}<"
        ))
        .expect("the record is in the fixture");
    let end = start + REIHE_A[start..].find("</metadata></record>").unwrap();
    let edited = edit(&REIHE_A[start..end]);
    assert_ne!(edited, REIHE_A[start..end], "the edit changed the record");
    format!("{}{edited}{}", &REIHE_A[..start], &REIHE_A[end..])
}

/// The "Glitzerblume" record as an announcement (with 263 = March 2027, as in reiheN).
fn announced_glitzerblume() -> String {
    edit_glitzerblume(|record| {
        record.replacen(
            r#"<datafield tag="264""#,
            r#"<datafield tag="263" ind1=" " ind2=" ">
      <subfield code="a">202703</subfield>
    </datafield>
    <datafield tag="264""#,
            1,
        )
    })
}

#[derive(Debug, PartialEq)]
struct GlitzerEdition {
    id: i64,
    work_id: i64,
    isbn13: String,
    dnb_idn: Option<String>,
    dnb_record_idn: Option<String>,
    withdrawn: bool,
}

/// Both "Glitzerblume" editions, in ISBN order.
async fn glitzer_editions(app: &TestApp) -> Vec<GlitzerEdition> {
    let isbns: Vec<String> = GLITZERBLUME_ISBNS.map(str::to_owned).to_vec();
    sqlx::query_as!(
        GlitzerEdition,
        r#"SELECT id, work_id, isbn13 AS "isbn13!", dnb_idn, dnb_record_idn,
                  withdrawn_at IS NOT NULL AS "withdrawn!"
           FROM editions WHERE isbn13 = ANY($1) ORDER BY isbn13"#,
        &isbns
    )
    .fetch_all(&app.db)
    .await
    .unwrap()
}

/// The new-releases hits as `(work id, isbn)`.
async fn listed(user: &TestClient) -> Vec<(i64, Option<String>)> {
    let page = user.get("/api/books/new-releases").await;
    page.assert_status(200);
    page.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| {
            (
                hit["id"].as_i64().unwrap(),
                hit["isbn"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

/// Acceptance (#42) for a record with two ISBNs: only the first ISBN's edition carries the IDN
/// as `dnb_idn` (it is unique), but both carry the record's IDN as `dnb_record_idn` (conventions
/// §7.5), so the deleted header withdraws both editions. Each one alone keeps the work in
/// new-releases while it is live, and neither does once withdrawn; other titles stay. The record
/// harvested again keeps one edition per ISBN and brings the work back.
#[sqlx::test]
async fn a_deleted_multi_isbn_record_leaves_new_releases(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let (other, _) = release(&app, 1, 10, "ger").await;
    let announced = announced_glitzerblume();
    harvest(&app, SET_A, &[&announced]).await;
    let editions = glitzer_editions(&app).await;
    assert_eq!(editions.len(), 2);
    let work_id = editions[0].work_id;
    assert_eq!(editions[1].work_id, work_id, "one work");
    assert_eq!(editions[0].dnb_idn.as_deref(), Some(GLITZERBLUME_IDN));
    assert_eq!(editions[1].dnb_idn, None);
    for edition in &editions {
        assert_eq!(edition.dnb_record_idn.as_deref(), Some(GLITZERBLUME_IDN));
    }
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM editions WHERE work_id = {work_id}
               AND planned_publication_date = '2027-03-01'
               AND field_sources->>'planned_publication_date' = 'dnb'"
        ))
        .await,
        2
    );
    // Only the second ISBN's edition in the window: it alone lists the work.
    plan_in(&app, editions[0].id, 200).await;
    plan_in(&app, editions[1].id, 10).await;
    let second = (work_id, Some(editions[1].isbn13.clone()));
    assert!(listed(&ada).await.contains(&second));

    let deleted =
        edit_glitzerblume(|record| record.replacen("<header>", r#"<header status="deleted">"#, 1));
    harvest(&app, SET_A, &[&deleted]).await;
    let withdrawn: Vec<bool> = glitzer_editions(&app)
        .await
        .iter()
        .map(|edition| edition.withdrawn)
        .collect();
    assert_eq!(withdrawn, [true, true], "the record's editions, both ISBNs");
    // Neither edition lists the work any more, wherever its date lies.
    for days in [10, 200] {
        plan_in(&app, editions[0].id, days).await;
        let hits = listed(&ada).await;
        assert!(hits.iter().all(|(id, _)| *id != work_id), "{hits:?}");
        assert!(hits.iter().any(|(id, _)| *id == other.id), "{hits:?}");
    }

    // The announcement is delivered again, twice: the same two editions, live again.
    harvest(&app, SET_A, &[&announced]).await;
    harvest(&app, SET_A, &[&announced]).await;
    let again = glitzer_editions(&app).await;
    let edition_ids =
        |editions: &[GlitzerEdition]| editions.iter().map(|e| e.id).collect::<Vec<_>>();
    assert_eq!(
        edition_ids(&again),
        edition_ids(&editions),
        "one edition per ISBN"
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM editions WHERE work_id = {work_id}"
        ))
        .await,
        2
    );
    assert!(again.iter().all(|edition| !edition.withdrawn));
    assert!(
        again
            .iter()
            .all(|edition| edition.dnb_record_idn.as_deref() == Some(GLITZERBLUME_IDN))
    );
    for edition in &again {
        plan_in(&app, edition.id, 10).await;
    }
    let hits = listed(&ada).await;
    assert!(hits.iter().any(|(id, _)| *id == work_id), "{hits:?}");
}

/// Waits until a session of this test's database waits for a lock.
async fn until_a_lock_wait(db: &PgPool) {
    for _ in 0..500 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(db)
        .await
        .unwrap();
        if waiting > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("nothing waited for a lock");
}

/// An admin merge that is already moving the loser's editions cannot see an edition the
/// harvest adds to the loser at the same moment (CAT's rule for writes that store a
/// `work_id`). The record waits for the merge on the work's row, sees the work merged away and
/// is ingested again, into the surviving work; nothing stays on the merged-away work.
#[sqlx::test]
async fn a_record_ingested_during_a_merge_lands_on_the_surviving_work(db: PgPool) {
    let app = TestApp::new(db).await;
    // Two DNB works with the record's title, author and match key, as before an admin merges
    // duplicates (ingest would group the second into the first, so it is renamed). The
    // harvest groups by the older one.
    let (loser, _) = app.dnb_work("Bondbreaker", "Mila Finch", &isbn(1)).await;
    let (survivor, _) = app
        .dnb_work("Bondbreaker (Doublette)", "Mila Finch", &isbn(2))
        .await;
    sqlx::query!(
        "UPDATE works SET (title, match_key) = (SELECT title, match_key FROM works WHERE id = $1)
         WHERE id = $2",
        loser.id,
        survivor.id
    )
    .execute(&app.db)
    .await
    .unwrap();

    let mut merge = app.db.begin().await.unwrap();
    merge_works(&mut merge, loser.id, survivor.id)
        .await
        .unwrap();
    let finish_merge = async {
        until_a_lock_wait(&app.db).await;
        merge.commit().await.unwrap();
    };
    tokio::join!(harvest(&app, SET_N, &[PAGE_1, PAGE_2]), finish_merge);

    let bond = edition_by_idn(&app, BONDBREAKER_IDN).await.unwrap();
    assert_eq!(bond.work_id, survivor.id);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM editions WHERE work_id = {}",
            loser.id
        ))
        .await,
        0
    );
}

/// Acceptance (#42): the same IDN re-delivered (here from reiheA, once published) with the
/// final title finds its edition by IDN and ISBN and replaces the provisional title of the
/// single-edition DNB work. Without 263 the planned date stays.
#[sqlx::test]
async fn a_retitled_record_replaces_the_provisional_title(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    harvest(&app, SET_N, &[PAGE_1, PAGE_2]).await;
    let bond = edition_by_idn(&app, BONDBREAKER_IDN).await.unwrap();
    let works = app.scalar_i64("SELECT count(*) FROM works").await;
    let editions = app.scalar_i64("SELECT count(*) FROM editions").await;

    harvest(&app, SET_A, &[RETITLED]).await;
    let after = edition_by_idn(&app, BONDBREAKER_IDN).await.unwrap();
    assert_eq!(after.id, bond.id);
    assert_eq!(after.work_id, bond.work_id);
    assert_eq!(
        after.planned_publication_date,
        Some(time::macros::date!(2028 - 03 - 01))
    );
    assert_eq!(work(&app, bond.work_id).await.title, FINAL_TITLE);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM works").await, works);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM editions").await,
        editions
    );

    plan_in(&app, bond.id, 30).await;
    let page = ada.get("/api/books/new-releases").await.json();
    let hit = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|hit| hit["id"] == bond.work_id)
        .expect("the retitled work is listed")
        .clone();
    assert_eq!(hit["title"], FINAL_TITLE);
}

/// An approved admin edit outranks the DNB: a re-delivered record does not retitle the work.
#[sqlx::test]
async fn an_admin_title_survives_a_retitled_record(db: PgPool) {
    let app = TestApp::new(db).await;
    harvest(&app, SET_N, &[PAGE_1, PAGE_2]).await;
    let bond = edition_by_idn(&app, BONDBREAKER_IDN).await.unwrap();
    let mut fields = serde_json::Map::new();
    fields.insert("title".into(), json!("Bondbreaker (Sonderausgabe)"));
    let mut conn = app.db.acquire().await.unwrap();
    ingest::apply_admin_fields(&mut conn, CoverTarget::Work(bond.work_id), &fields)
        .await
        .unwrap();

    harvest(&app, SET_A, &[RETITLED]).await;
    assert_eq!(
        work(&app, bond.work_id).await.title,
        "Bondbreaker (Sonderausgabe)"
    );
}

/// Acceptance (#42): a title that is not published yet is searchable through the local
/// catalog and resolves through `CatalogLookup::resolve_isbn`'s local step, without asking
/// the DNB or Open Library.
#[sqlx::test]
async fn an_announced_title_is_searchable_and_resolvable(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    let ada = app.user("ada").await;
    harvest(&app, SET_N, &[PAGE_1, PAGE_2]).await;
    let bond = edition_by_idn(&app, BONDBREAKER_IDN).await.unwrap();
    // Announced for March 2028, after the recording (2026-10-27).
    assert_eq!(bond.planned_publication_date, Some(date!(2028 - 03 - 01)));

    app.upstream.reset().await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"numFound": 0, "docs": []})))
        .mount(&app.upstream)
        .await;
    let search = ada.get("/api/books/search?q=Bondbreaker%20Finch").await;
    search.assert_status(200);
    let first = &search.json()["items"][0];
    assert_eq!(first["id"], bond.work_id);
    assert_eq!(first["source"], "dnb");

    app.upstream.reset().await;
    let mut conn = app.db.acquire().await.unwrap();
    let isbn = parse_isbn(BONDBREAKER_ISBN).unwrap();
    let resolved = app
        .state
        .lookup
        .resolve_isbn(&mut conn, &isbn)
        .await
        .unwrap();
    assert_eq!(
        resolved,
        Some(Resolved {
            work_id: bond.work_id,
            edition_id: Some(bond.id)
        })
    );
    let hit = ada
        .get(&format!("/api/books/isbn/{BONDBREAKER_ISBN}"))
        .await;
    hit.assert_status(200);
    assert_eq!(hit.json()["work_id"], bond.work_id);
    assert!(app.upstream_requests().await.is_empty());
}

// --- GET /api/books/new-releases -------------------------------------------------------------

/// A valid ISBN-13 in the German group (978-3) for test number `n`.
fn isbn(n: u32) -> String {
    let body = format!("97830000{n:04}");
    let sum: u32 = body
        .bytes()
        .enumerate()
        .map(|(i, b)| u32::from(b - b'0') * if i % 2 == 0 { 1 } else { 3 })
        .sum();
    format!("{body}{}", (10 - sum % 10) % 10)
}

/// A DNB work with one edition planned `days` from today (UTC) in `language`.
async fn release(app: &TestApp, n: u32, days: i32, language: &str) -> (TestWork, TestEdition) {
    let (work, edition) = app
        .dnb_work(&format!("Titel {n}"), &format!("Autorin {n}"), &isbn(n))
        .await;
    plan_in(app, edition.id, days).await;
    sqlx::query!(
        "UPDATE editions SET language = $2 WHERE id = $1",
        edition.id,
        language
    )
    .execute(&app.db)
    .await
    .unwrap();
    (work, edition)
}

/// The window is today − 30 to today + 120 days, both ends included; withdrawn editions,
/// other languages and editions without a date are left out; a work with several editions in
/// the window is listed once, with its earliest edition (its ISBN, date and own title); the
/// list is ordered by date, then work id. No remote calls.
#[sqlx::test]
async fn new_releases_lists_the_window(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let (soon, _) = release(&app, 1, 10, "ger").await;
    let (oldest, _) = release(&app, 2, -30, "ger").await;
    release(&app, 3, -31, "ger").await;
    release(&app, 4, 121, "ger").await;
    let (last, _) = release(&app, 5, 120, "ger").await;
    let (_, withdrawn) = release(&app, 6, 60, "ger").await;
    app.exec(&format!(
        "UPDATE editions SET withdrawn_at = now() WHERE id = {}",
        withdrawn.id
    ))
    .await;
    release(&app, 7, 5, "eng").await;
    app.dnb_work("Ohne Termin", "Autor", &isbn(8)).await;
    // Two editions of one work in the window: the earlier one is the hit.
    let (two, later) = release(&app, 9, 50, "ger").await;
    let earlier = app.edition(&two, &isbn(10)).await;
    app.exec(&format!(
        "UPDATE editions SET language = 'ger', title = 'Titel 9 (Taschenbuch)',
                planned_publication_date = (now() AT TIME ZONE 'UTC')::date + 20
         WHERE id = {}",
        earlier.id
    ))
    .await;
    assert_ne!(later.id, earlier.id);
    // Same date as `soon`: the lower work id first.
    let (tie, _) = release(&app, 11, 10, "ger").await;

    let res = ada.get("/api/books/new-releases").await;
    res.assert_status(200);
    let page = res.json();
    assert_eq!(
        ids(&page),
        [oldest.id, soon.id, tie.id, two.id, last.id],
        "{page:#}"
    );
    assert_eq!(page["page"], 1);
    assert_eq!(page["has_more"], false);

    let today = time::OffsetDateTime::now_utc().date();
    let hit = &page["items"][3];
    assert_eq!(hit["title"], "Titel 9 (Taschenbuch)");
    assert_eq!(hit["isbn"], isbn(10));
    assert_eq!(
        hit["planned_publication_date"],
        (today + time::Duration::days(20)).to_string()
    );
    let first = &page["items"][0];
    assert_eq!(first["title"], "Titel 2");
    assert_eq!(first["authors"], "Autorin 2");
    assert_eq!(first["isbn"], isbn(2));
    assert_eq!(first["source"], "dnb");
    assert_eq!(first["custom"], false);
    assert_eq!(first["on_shelf"], false);
    assert_eq!(first["shelf_id"], Value::Null);
    assert_eq!(first["club_picks"], json!([]));
    assert_eq!(first["ol_work_key"], Value::Null);
    assert_eq!(
        first["planned_publication_date"],
        (today - time::Duration::days(30)).to_string()
    );
    assert!(app.upstream_requests().await.is_empty());
}

/// `language` picks the editions' language (default `ger`).
#[sqlx::test]
async fn new_releases_by_language(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let (german, _) = release(&app, 1, 3, "ger").await;
    let (english, _) = release(&app, 2, 3, "eng").await;
    assert_eq!(
        ids(&ada.get("/api/books/new-releases").await.json()),
        [german.id]
    );
    assert_eq!(
        ids(&ada.get("/api/books/new-releases?language=ger").await.json()),
        [german.id]
    );
    assert_eq!(
        ids(&ada.get("/api/books/new-releases?language=eng").await.json()),
        [english.id]
    );
    assert!(ids(&ada.get("/api/books/new-releases?language=fre").await.json()).is_empty());
}

/// `page` and `limit` page through the list; `has_more` says whether another page follows.
#[sqlx::test]
async fn new_releases_pages(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let mut works = Vec::new();
    for n in 1..=5 {
        works.push(release(&app, n, n as i32, "ger").await.0.id);
    }
    let page = |n: u32| {
        let ada = ada.clone();
        async move {
            ada.get(&format!("/api/books/new-releases?page={n}&limit=2"))
                .await
                .json()
        }
    };
    let first = page(1).await;
    assert_eq!(ids(&first), works[0..2]);
    assert_eq!(first["has_more"], true);
    let second = page(2).await;
    assert_eq!(ids(&second), works[2..4]);
    assert_eq!(second["page"], 2);
    assert_eq!(second["has_more"], true);
    let third = page(3).await;
    assert_eq!(ids(&third), works[4..5]);
    assert_eq!(third["has_more"], false);
    let fourth = page(4).await;
    assert!(ids(&fourth).is_empty());
    assert_eq!(fourth["has_more"], false);
    // The default page holds 24.
    assert_eq!(
        ids(&ada.get("/api/books/new-releases").await.json()).len(),
        5
    );
}

/// Parameters are checked like search's: 400 with pydantic's wording.
#[sqlx::test]
async fn new_releases_checks_its_parameters(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for (query, detail) in [
        ("language=de", LANGUAGE_PATTERN),
        ("language=GER", LANGUAGE_PATTERN),
        ("language=", LANGUAGE_PATTERN),
        ("page=0", "Input should be greater than or equal to 1"),
        ("limit=0", "Input should be greater than or equal to 1"),
        ("limit=41", "Input should be less than or equal to 40"),
    ] {
        ada.get(&format!("/api/books/new-releases?{query}"))
            .await
            .assert_detail(400, detail);
    }
    ada.get("/api/books/new-releases?page=x")
        .await
        .assert_status(400);
    ada.get("/api/books/new-releases?limit=40")
        .await
        .assert_status(200);
    app.anon()
        .get("/api/books/new-releases")
        .await
        .assert_detail(401, "Not signed in");
}

/// Hits carry the viewer's shelf entry and the viewer's clubs whose current pick the work is,
/// like search hits.
#[sqlx::test]
async fn new_releases_are_annotated_for_the_viewer(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let (book, _) = release(&app, 1, 7, "ger").await;
    let club = app.club(&ada, "Leserunde").await;
    let entry: i64 = sqlx::query_scalar!(
        "INSERT INTO shelf_entries (user_id, work_id, status) VALUES ($1, $2, 'want_to_read')
         RETURNING id",
        ada.id(),
        book.id
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    sqlx::query!(
        "INSERT INTO club_picks (club_id, work_id, set_by) VALUES ($1, $2, $3)",
        club.id,
        book.id,
        ada.id()
    )
    .execute(&app.db)
    .await
    .unwrap();

    let hit = ada.get("/api/books/new-releases").await.json()["items"][0].clone();
    assert_eq!(hit["on_shelf"], true);
    assert_eq!(hit["shelf_id"], entry);
    assert_eq!(
        hit["club_picks"],
        json!([{"slug": club.slug, "name": "Leserunde"}])
    );
    let theirs = bob.get("/api/books/new-releases").await.json()["items"][0].clone();
    assert_eq!(theirs["id"], book.id);
    assert_eq!(theirs["on_shelf"], false);
    assert_eq!(theirs["club_picks"], json!([]));
}

/// "Today" is the viewer's: Kiritimati (UTC+14) is always one or two dates ahead of Pago Pago
/// (UTC−11), so each sees an edge date the other does not.
#[sqlx::test]
async fn the_window_follows_the_viewer_time_zone(db: PgPool) {
    let app = TestApp::new(db).await;
    let east = app.user("east").await;
    let west = app.user("west").await;
    app.exec(
        "UPDATE users SET timezone = 'Pacific/Kiritimati' WHERE username = 'east';
         UPDATE users SET timezone = 'Pacific/Pago_Pago' WHERE username = 'west';",
    )
    .await;
    let (late, late_edition) = release(&app, 1, 0, "ger").await;
    let (early, early_edition) = release(&app, 2, 0, "ger").await;
    app.exec(&format!(
        "UPDATE editions SET planned_publication_date =
             (now() AT TIME ZONE 'Pacific/Kiritimati')::date + 120 WHERE id = {};
         UPDATE editions SET planned_publication_date =
             (now() AT TIME ZONE 'Pacific/Pago_Pago')::date - 30 WHERE id = {};",
        late_edition.id, early_edition.id
    ))
    .await;
    assert_eq!(
        ids(&east.get("/api/books/new-releases").await.json()),
        [late.id]
    );
    assert_eq!(
        ids(&west.get("/api/books/new-releases").await.json()),
        [early.id]
    );
}

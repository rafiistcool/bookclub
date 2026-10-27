//! Legacy shelves, layouts, favourites, quotes, search history, picks, milestones and votes
//! (docs/phase1/legacy-mapping.md §4–§8, §10, §11, §13; §17 AC 6, 11 and 12), on
//! tests/fixtures/legacy/fresh.sql.

mod common;
#[path = "fixtures/legacy/harness.rs"]
mod legacy;

use bookclub_server::catalog::shared::books::{BookRef, book_out};
use bookclub_server::catalog::shared::ingest::Source;
use bookclub_server::importer::{MatchKind, Report};
use common::*;
use legacy::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;

/// The fixture's ISBN row (book 10) resolves to the Open Library work of book 9 (`OL45883W`):
/// the merge fixture of AC 6.
fn program_merge(app: &TestApp) {
    app.catalog.on_isbn(
        ISBN_MATCH,
        FakeHit::New {
            title: "Middlemarch".into(),
            authors: "George Eliot".into(),
            ol_work_id: Some("OL45883W".into()),
            source: Source::OpenLibrary,
        },
    );
}

async fn merged_import(app: &TestApp, legacy: &Legacy) -> Report {
    program_merge(app);
    import_ok(app, legacy.args("demo")).await
}

/// id, user_id, edition_id, body, page, visibility, updated_at IS NULL.
type QuoteRow = (i64, i64, Option<i64>, String, Option<i32>, String, bool);
/// id, club_id, work_id, set_by, ended_at, reminder_sent_at IS NOT NULL.
type PickRow = (i64, i64, i64, Option<i64>, Option<OffsetDateTime>, bool);
/// id, club_id, pick_id, unit, range_from, range_to, position.
type MilestoneRow = (i64, i64, i64, Option<String>, Option<i32>, Option<i32>, i32);

/// A shelf entry's columns.
#[derive(Debug, sqlx::FromRow, PartialEq)]
struct Entry {
    id: i64,
    user_id: i64,
    work_id: i64,
    edition_id: Option<i64>,
    status: String,
    position: i32,
    shelf_position: Option<i32>,
    rating: Option<i32>,
    rating_visibility: String,
    take: String,
    dnf_reason: String,
    progress: Option<i32>,
    started_at: Option<OffsetDateTime>,
    finished_at: Option<OffsetDateTime>,
}

async fn entries(app: &TestApp) -> Vec<Entry> {
    sqlx::query_as(
        "SELECT id, user_id, work_id, edition_id, status, position, shelf_position, rating,
                rating_visibility, take, dnf_reason, progress, started_at, finished_at
         FROM shelf_entries ORDER BY id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap()
}

fn entry(all: &[Entry], id: i64) -> &Entry {
    all.iter()
        .find(|e| e.id == id)
        .unwrap_or_else(|| panic!("no shelf entry {id}"))
}

/// The timestamp a query on the legacy fixture returns (one row, one column).
fn legacy_time(legacy: &Legacy, sql: &str) -> OffsetDateTime {
    let conn = rusqlite::Connection::open(&legacy.path).unwrap();
    let text: String = conn.query_row(sql, [], |r| r.get(0)).unwrap();
    bookclub_server::importer::parse_timestamp(&text).unwrap()
}

/// §4: entries keep their ids; values their status does not allow are dropped (counted);
/// ISBN-keyed books carry their edition; positions are dense per (user, status); the sequence
/// continues after the highest id.
#[sqlx::test]
async fn shelf_entries_keep_their_ids_and_follow_their_status(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let report = merged_import(&app, &legacy).await;
    let all = entries(&app).await;
    assert_eq!(all.len(), 26);
    assert!(all.iter().all(|e| e.rating_visibility == "audience"));

    // Finished seed entries carried progress 100: dropped.
    let e = entry(&all, 3);
    assert_eq!(
        (e.status.as_str(), e.rating, e.progress, e.take.as_str()),
        (
            "finished",
            Some(5),
            None,
            "Eine Geschichte, die noch lange nachklingt."
        )
    );
    // want_to_read with a rating and a take: both dropped.
    let e = entry(&all, 26);
    assert_eq!(
        (e.status.as_str(), e.rating, e.take.as_str()),
        ("want_to_read", None, "")
    );
    // currently_reading with progress 150: NULL; the only entry of its column gets position 0.
    let e = entry(&all, 24);
    assert_eq!(
        (e.progress, e.position, e.shelf_position),
        (None, 0, Some(3))
    );
    let e = entry(&all, 23);
    assert_eq!(
        (e.status.as_str(), e.dnf_reason.as_str()),
        ("did_not_finish", "Zu lang.")
    );
    let e = entry(&all, 22);
    assert_eq!((e.rating, e.take.as_str()), (Some(5), "Wunderbar."));

    // Editions: only ISBN keys identify one.
    let lena = book_id(&legacy, &format!("/works/ISBN{ISBN_UNMATCHED}"));
    assert_eq!(entry(&all, 21).edition_id, report.books[&lena].edition_id);
    assert!(entry(&all, 21).edition_id.is_some());
    let isbn10 = book_id(&legacy, &format!("/works/ISBN{ISBN10_KEY}"));
    assert_eq!(entry(&all, 25).edition_id, report.books[&isbn10].edition_id);
    assert_eq!(
        entry(&all, 26).edition_id,
        None,
        "an invalid ISBN has no edition"
    );
    assert_eq!(entry(&all, 1).edition_id, None, "a BC book has no edition");

    // Dense positions per (user, status) in the legacy (position, id) order: demo's finished
    // column is 3 (2), 19 (3), 5 (4), 8 (7).
    let mut finished: Vec<(i32, i64)> = all
        .iter()
        .filter(|e| e.user_id == 1 && e.status == "finished")
        .map(|e| (e.position, e.id))
        .collect();
    finished.sort();
    assert_eq!(finished, vec![(0, 3), (1, 19), (2, 5), (3, 8)]);
    let otto: Vec<(i64, i32)> = all
        .iter()
        .filter(|e| e.user_id == 5 && e.status == "want_to_read")
        .map(|e| (e.id, e.position))
        .collect();
    assert_eq!(otto, vec![(26, 0), (27, 1)]);

    assert!(report.normalised("shelf.progress") >= 8);
    assert_eq!(report.normalised("shelf.rating"), 1);
    assert_eq!(report.normalised("shelf.take"), 1);
    assert_eq!(
        app.scalar_i64("SELECT nextval(pg_get_serial_sequence('shelf_entries', 'id'))")
            .await,
        28
    );
}

/// #38 AC 6 (shelf): demo's OL entry and ISBN entry of one work merge into the finished one,
/// which takes the edition the ISBN key identified; the report lists the merge.
#[sqlx::test]
async fn two_entries_of_one_work_merge_into_the_winner(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let report = merged_import(&app, &legacy).await;
    let all = entries(&app).await;
    assert!(all.iter().all(|e| e.id != 20), "the loser is gone");
    let ol = book_id(&legacy, "/works/OL45883W");
    let isbn = book_id(&legacy, &format!("/works/ISBN{ISBN_MATCH}"));
    let winner = entry(&all, 19);
    assert_eq!(winner.work_id, report.books[&ol].work_id);
    assert_eq!(report.books[&ol].work_id, report.books[&isbn].work_id);
    assert_eq!(
        (
            winner.status.as_str(),
            winner.rating,
            winner.take.as_str(),
            winner.progress
        ),
        ("finished", Some(4), "Klug und lang.", None)
    );
    // The winner had no edition: the loser's ISBN edition is copied.
    assert_eq!(winner.edition_id, report.books[&isbn].edition_id);
    assert!(winner.edition_id.is_some());
    // Its own dates win.
    assert_eq!(
        winner.started_at,
        Some(legacy_time(
            &legacy,
            "SELECT started_at FROM shelf WHERE id = 19"
        ))
    );
    assert!(
        report
            .merges
            .contains(&"shelf 20 → shelf 19 (demo): books 10 and 9 are one work".to_owned()),
        "{:?}",
        report.merges
    );
    let line = report.entity("shelf").unwrap();
    assert_eq!(
        (line.source, line.target, line.detail.as_str()),
        (Some(27), Some(26), "(1 merged)")
    );
}

/// #38 AC 12 and AC 6 (layout): demo's layout with a decoration, two spines and a deleted
/// entry id imports pruned and valid, the merged entry re-mapped onto the placed winner;
/// otto's invalid JSON becomes the default layout, counted and listed.
#[sqlx::test]
async fn layouts_import_pruned_remapped_and_valid(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let report = merged_import(&app, &legacy).await;
    let layouts: Vec<(i64, Value)> =
        sqlx::query_as("SELECT user_id, layout FROM physical_shelf_layouts ORDER BY user_id")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(layouts.len(), 2);
    assert_eq!(
        layouts[0],
        (
            1,
            json!({
                "rows": [[1, 2, 3, 4, "decor:plant1"], [19], []],
                "material": "walnut",
                "lights": false,
                "spines": {"19": {"color": "#a1b2c3", "width": 30, "display": "spine"}},
                "decorations": {"decor:plant1": {"kind": "plant", "color": "#2e7d32"}},
            })
        )
    );
    assert_eq!(
        layouts[1],
        (
            5,
            json!({"rows": [[], [], []], "material": "oak", "lights": false, "spines": {},
                   "decorations": {}})
        )
    );
    assert_eq!(report.normalised("physical_shelf_layouts.layout_json"), 1);
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.starts_with("physical_shelf_layouts 5: not JSON")),
        "{:?}",
        report.notes
    );
    let line = report.entity("physical_shelf_layouts").unwrap();
    assert_eq!(
        (line.source, line.target, line.detail.as_str()),
        (
            Some(2),
            Some(2),
            "(1 imported as the default, 2 entry ids pruned)"
        )
    );
}

/// A stored layout the current schema refuses (an unknown material) becomes the default
/// rather than aborting the import; one whose winner is not placed takes the loser's place.
#[sqlx::test]
async fn refused_layouts_become_the_default(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    legacy.exec(
        r##"INSERT INTO physical_shelf_layouts VALUES (4, '{"rows": [[21]], "material": "pine"}');
            UPDATE physical_shelf_layouts
               SET layout_json = '{"rows": [[20, 1]], "spines": {"20": {"display": "flat"}}}'
             WHERE user_id = 1;"##,
    );
    let report = merged_import(&app, &legacy).await;
    let lena: Value =
        sqlx::query_scalar("SELECT layout FROM physical_shelf_layouts WHERE user_id = 4")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(lena["material"], "oak");
    assert_eq!(lena["rows"], json!([[], [], []]));
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.starts_with("physical_shelf_layouts 4: material \"pine\"")),
        "{:?}",
        report.notes
    );
    let demo: Value =
        sqlx::query_scalar("SELECT layout FROM physical_shelf_layouts WHERE user_id = 1")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(demo["rows"], json!([[19, 1]]));
    assert_eq!(
        demo["spines"],
        json!({"19": {"color": null, "width": null, "display": "flat"}})
    );
}

/// §6: favourites map onto works; two of one work keep the lower position; positions are
/// compacted to 1..n (the legacy gap repaired).
#[sqlx::test]
async fn favourites_keep_the_lower_position_and_are_compacted(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let report = merged_import(&app, &legacy).await;
    let favorites: Vec<(i64, i64, i32)> = sqlx::query_as(
        "SELECT user_id, work_id, position FROM user_favorites ORDER BY user_id, position",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    let work = |key: &str| report.books[&book_id(&legacy, key)].work_id;
    let middlemarch = work("/works/OL45883W");
    assert_eq!(
        favorites,
        vec![
            (1, work("/works/BC0000000003"), 1),
            (1, work("/works/BC0000000005"), 2),
            (1, middlemarch, 3),
            // anna had the ISBN row at 1 and the OL row at 3: one favourite, at 1.
            (2, middlemarch, 1),
            (2, work("/works/BC0000000002"), 2),
            // lena had 1 and 3.
            (4, work("/works/GBsynthVol42_x-Y"), 1),
            (4, work("/works/MISC-legacy-1"), 2),
        ]
    );
    assert!(
        report
            .merges
            .contains(&"user_favorites 6 → user_favorites 4: books 9 and 10 are one work".into()),
        "{:?}",
        report.merges
    );
    assert_eq!(report.normalised("user_favorites.position"), 1);
}

/// §10: quotes keep their ids, get the edition of an ISBN key, visibility audience and no
/// updated_at; a page outside 0..20000 becomes NULL.
#[sqlx::test]
async fn quotes_keep_their_ids_and_get_their_edition(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let isbn = book_id(&legacy, &format!("/works/ISBN{ISBN_UNMATCHED}"));
    legacy.exec(&format!(
        "INSERT INTO quotes VALUES (9, 4, {isbn}, 'Ein Satz aus der Ausgabe.', 7,
                                    '2026-08-20 10:00:00.000000');"
    ));
    let report = merged_import(&app, &legacy).await;
    let quotes: Vec<QuoteRow> = sqlx::query_as(
        "SELECT id, user_id, edition_id, body, page, visibility, updated_at IS NULL
             FROM quotes ORDER BY id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(quotes.len(), 4);
    assert_eq!(
        quotes[1],
        (
            2,
            4,
            None,
            "Geänderter Satz nach dem Speichern.".to_owned(),
            Some(12),
            "audience".to_owned(),
            true
        )
    );
    assert_eq!(quotes[2].4, None, "page 25000");
    assert_eq!(quotes[3].0, 9);
    assert_eq!(quotes[3].2, report.books[&isbn].edition_id);
    assert!(quotes[3].2.is_some());
    assert_eq!(report.normalised("quotes.page"), 1);
    assert_eq!(
        app.scalar_i64("SELECT nextval(pg_get_serial_sequence('quotes', 'id'))")
            .await,
        10
    );
}

/// §13: search history keeps its ids and normalised queries; an over-long query is cut to 300
/// characters and normalised again.
#[sqlx::test]
async fn search_history_keeps_its_ids(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let report = merged_import(&app, &legacy).await;
    let rows: Vec<(i64, i64, String, String)> = sqlx::query_as(
        "SELECT id, user_id, query, normalized_query FROM search_history ORDER BY id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows[2],
        (
            3,
            4,
            "Straße der Bücher".to_owned(),
            "strasse der bücher".to_owned()
        )
    );
    assert_eq!(rows[3].2, "x".repeat(300));
    assert_eq!(rows[3].3, "x".repeat(300));
    assert_eq!(report.normalised("search_history.query"), 1);
    assert_eq!(
        app.scalar_i64("SELECT nextval(pg_get_serial_sequence('search_history', 'id'))")
            .await,
        5
    );
}

/// §7, §8: picks and milestones keep their ids in the imported club; a second open pick is
/// ended when the next one started; chapters become a `chapter` range (a backwards one
/// swapped), a title-only checkpoint has no unit, positions are dense per pick.
#[sqlx::test]
async fn picks_and_milestones_join_the_club(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    legacy.exec(
        "UPDATE club_picks SET ended_at = NULL, reminder_sent_at = '2026-07-30 08:00:00'
          WHERE id = 1;
         UPDATE pick_milestones SET chapter_from = 16, chapter_to = 9, position = 7 WHERE id = 2;
         UPDATE pick_milestones SET position = 12 WHERE id = 3;",
    );
    let report = merged_import(&app, &legacy).await;
    let club = app.scalar_i64("SELECT id FROM clubs").await;
    let picks: Vec<PickRow> = sqlx::query_as(
        "SELECT id, club_id, work_id, set_by, ended_at, reminder_sent_at IS NOT NULL
         FROM club_picks ORDER BY id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    let work = |key: &str| report.books[&book_id(&legacy, key)].work_id;
    assert_eq!(picks.len(), 2);
    let second_started = legacy_time(&legacy, "SELECT started_at FROM club_picks WHERE id = 2");
    assert_eq!(
        picks[0],
        (
            1,
            club,
            work("/works/BC0000000003"),
            Some(2),
            Some(second_started),
            true
        )
    );
    assert_eq!(
        (picks[1].0, picks[1].2, picks[1].4),
        (2, work("/works/BC0000000001"), None)
    );
    assert_eq!(report.normalised("club_picks.ended_at"), 1);

    let milestones: Vec<MilestoneRow> = sqlx::query_as(
        "SELECT id, club_id, pick_id, unit, range_from, range_to, position
             FROM milestones ORDER BY id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        milestones,
        vec![
            (1, club, 2, Some("chapter".into()), Some(1), Some(8), 0),
            (2, club, 2, Some("chapter".into()), Some(9), Some(16), 1),
            (3, club, 2, Some("chapter".into()), Some(17), Some(24), 2),
            (4, club, 1, None, None, None, 0),
        ]
    );
    assert_eq!(report.normalised("pick_milestones.chapter_range"), 1);
    assert_eq!(
        app.scalar_i64("SELECT nextval(pg_get_serial_sequence('milestones', 'id'))")
            .await,
        5
    );
}

/// #38 AC 6 (nominations) and §11: the two nominations of one work in vote 2 merge into the
/// earliest; their ballots move to it, and the vote keeps its winner, re-pointed in the second
/// pass. A second open vote is ended.
#[sqlx::test]
async fn a_merged_nomination_keeps_its_vote_winner(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let report = merged_import(&app, &legacy).await;
    let votes: Vec<(i64, Option<i64>, bool)> = sqlx::query_as(
        "SELECT id, winner_nomination_id, ended_at IS NULL FROM next_up_votes ORDER BY id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(votes, vec![(1, None, true), (2, Some(4), false)]);
    let nominations: Vec<(i64, i64)> =
        sqlx::query_as("SELECT id, vote_id FROM next_up_nominations ORDER BY id")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(nominations, vec![(1, 1), (2, 1), (3, 1), (4, 2), (6, 2)]);
    let ballots: Vec<(i64, i64, i64)> = sqlx::query_as(
        "SELECT vote_id, user_id, nomination_id FROM next_up_ballots ORDER BY vote_id, user_id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        ballots,
        vec![(1, 2, 1), (1, 3, 2), (2, 2, 4), (2, 3, 4), (2, 4, 4)]
    );
    assert!(
        report.merges.contains(
            &"next_up_nominations 5 → next_up_nominations 4 (vote 2): books 10 and 9 are one work"
                .to_owned()
        ),
        "{:?}",
        report.merges
    );
    let line = report.entity("next_up_nominations").unwrap();
    assert_eq!((line.source, line.target), (Some(6), Some(5)));
}

/// A second open vote is ended when the next vote started; a winner that is no nomination of
/// its vote is dropped (counted).
#[sqlx::test]
async fn extra_open_votes_end_and_foreign_winners_are_dropped(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    legacy.exec(
        "INSERT INTO next_up_votes VALUES (3, '2026-09-02 12:00:00', NULL, NULL, NULL);
         UPDATE next_up_votes SET winner_nomination_id = 1 WHERE id = 2;",
    );
    let report = merged_import(&app, &legacy).await;
    let votes: Vec<(i64, Option<i64>, Option<OffsetDateTime>)> =
        sqlx::query_as("SELECT id, winner_nomination_id, ended_at FROM next_up_votes ORDER BY id")
            .fetch_all(&app.db)
            .await
            .unwrap();
    let ended = legacy_time(&legacy, "SELECT started_at FROM next_up_votes WHERE id = 2");
    assert_eq!(votes[0], (1, None, Some(ended)));
    assert_eq!(votes[1].1, None, "nomination 1 belongs to vote 1");
    assert_eq!(votes[2], (3, None, None));
    assert_eq!(report.normalised("next_up_votes.ended_at"), 1);
    assert_eq!(report.normalised("next_up_votes.winner_nomination_id"), 1);
}

/// #38 AC 11 (through the shelf): for every legacy shelf row, the year and pages the legacy
/// API showed for its book equal the imported entry's `BookOut` (with the entry's edition),
/// except for books listed in the report's differences; every key class is covered.
#[sqlx::test]
async fn shelf_entries_show_the_legacy_year_and_pages(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    // anna also shelves the matched ISBN row, so that class is seen through an entry of its own.
    legacy.exec(
        "INSERT INTO shelf VALUES (40, 2, 10, 'want_to_read', 9, NULL, NULL, '', '', NULL,
                                   '2026-08-01 00:00:00', NULL, NULL);",
    );
    let report = merged_import(&app, &legacy).await;
    let conn = rusqlite::Connection::open(&legacy.path).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT s.id, b.id, b.ol_work_key, b.year, b.pages
             FROM shelf s JOIN books b ON b.id = s.book_id ORDER BY s.id",
        )
        .unwrap();
    type Row = (i64, i64, String, Option<i64>, Option<i64>);
    let rows: Vec<Row> = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let all = entries(&app).await;
    let mut pg = app.db.acquire().await.unwrap();
    let mut kinds = std::collections::HashSet::new();
    for (shelf_id, book, key, year, pages) in rows {
        let Some(e) = all.iter().find(|e| e.id == shelf_id) else {
            assert!(
                report
                    .merges
                    .iter()
                    .any(|m| m.starts_with(&format!("shelf {shelf_id} → "))),
                "shelf {shelf_id} vanished unlisted"
            );
            continue;
        };
        kinds.insert(report.books[&book].kind);
        let out = book_out(
            &mut pg,
            BookRef {
                work_id: e.work_id,
                edition_id: e.edition_id,
            },
        )
        .await
        .unwrap()
        .unwrap();
        let year = year
            .filter(|y| (-5000..=3000).contains(y))
            .map(|y| y as i32);
        let pages = pages
            .filter(|p| (1..=100_000).contains(p))
            .map(|p| p as i32);
        if (out.year, out.pages) != (year, pages) {
            assert!(
                report
                    .differences
                    .iter()
                    .any(|d| d.starts_with(&format!("book {book} ({key})"))),
                "shelf {shelf_id} ({key}): {:?} became {:?}, unlisted",
                (year, pages),
                (out.year, out.pages)
            );
        }
    }
    for kind in [
        MatchKind::OpenLibrary,
        MatchKind::User,
        MatchKind::IsbnMatched,
        MatchKind::IsbnUnmatched,
        MatchKind::IsbnInvalid,
        MatchKind::GoogleVolume,
        MatchKind::Other,
    ] {
        assert!(kinds.contains(&kind), "{kind:?} not covered");
    }
}

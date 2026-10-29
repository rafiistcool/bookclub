//! The cover search of `import-legacy` and `rematch-legacy` (#74 AC 6, docs/phase2/notes/cover.md
//! §4.2, §4.3, §6) on the legacy fixture: which books are searched, the report's line and
//! section, the dry run, `--offline`, an outage, a database error, the time budget, and the
//! rematch pass with its pace.

mod common;
#[path = "common_cover/mod.rs"]
mod cover;
#[path = "fixtures/legacy/harness.rs"]
mod legacy;

use std::time::{Duration, Instant};

use bookclub_server::importer::{self, CoverSearchLine, RematchArgs, Report};
use common::*;
use cover::*;
use legacy::*;
use sqlx::PgPool;

/// The imported catalog books without a cover in `fresh.sql` (legacy book ids): the unmatched
/// ISBN rows 10 and 12, the invalid ISBN 13, the Google row 14 and the loose key 15.
const CANDIDATES: u64 = 5;

fn program(app: &TestApp) {
    // Book 12: a clear match. Book 14: another volume, a suggestion. The rest: no match.
    app.catalog.on_covers(
        "Zehnstellig",
        Some("Mustermann"),
        answer(vec![doc(
            51,
            "Zehnstellig",
            "",
            "Max Mustermann",
            Some(5100),
            4,
        )]),
    );
    app.catalog.on_covers(
        "Synthetic Volume",
        Some("Nonymous"),
        answer(vec![doc(
            52,
            "Synthetic Volume 2",
            "",
            "A. Nonymous",
            Some(5200),
            4,
        )]),
    );
}

fn cover_line(report: &Report) -> &str {
    report
        .lines
        .iter()
        .find(|line| line.starts_with("cover search "))
        .expect("a cover search line")
}

fn section(report: &Report) -> Vec<String> {
    let start = report
        .lines
        .iter()
        .position(|line| line == "without a cover:")
        .expect("a without a cover section");
    report.lines[start + 1..]
        .iter()
        .take_while(|line| line.starts_with("  "))
        .map(|line| line.trim().to_owned())
        .collect()
}

async fn readers_books_without_cover(app: &TestApp) -> u64 {
    app.scalar_i64("SELECT count(*) FROM works WHERE source = 'user' AND cover_media_id IS NULL")
        .await as u64
}

#[sqlx::test]
async fn the_import_searches_its_books_without_a_cover(db: PgPool) {
    let app = TestApp::new(db).await;
    program(&app);
    let legacy = load("fresh.sql");
    let report = import_ok(&app, legacy.args("demo")).await;
    let readers = readers_books_without_cover(&app).await;
    assert!(readers >= 8, "the BC rows");
    assert_eq!(
        report.covers,
        Some(CoverSearchLine {
            offline: false,
            candidates: CANDIDATES,
            without: 4,
            suggested: 1,
            no_match: 3,
            not_searched: 0,
            readers,
            // Middlemarch (12345), the edition cover of book 11 (777), and the cover the
            // search attached to book 12.
            unfetched: 3,
        })
    );
    assert_eq!(
        cover_line(&report),
        format!(
            "cover search           5 searched: 1 attached, 4 still without a cover \
             (1 suggested, 3 no match); {readers} books readers added left as they are; \
             3 imported cover ids not fetched yet (checked once the server runs)"
        )
    );
    assert_eq!(
        section(&report),
        [
            "book 10 \"Middlemarch (Synthetic Edition)\" by George Eliot: no match on Open Library",
            "book 13 \"Kaputte Nummer\" by Niemand: no match on Open Library",
            "book 14 \"Synthetic Volume\" by A. Nonymous: suggested, review it on the admin page",
            "book 15 \"Loose Key Book\" by Unbekannt: no match on Open Library",
        ]
    );
    let zehnstellig = report.books[&12].work_id;
    assert_eq!(
        cover_of(&app, zehnstellig).await,
        Some((5100, "openlibrary".into()))
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM cover_search").await,
        CANDIDATES as i64
    );
    // Readers' books were never sent to Open Library.
    let calls = app.catalog.cover_calls();
    assert_eq!(calls.len(), CANDIDATES as usize, "{calls:?}");
    assert!(!calls.iter().any(|call| call.contains("Bibliothek am Meer")));
}

#[sqlx::test]
async fn the_dry_run_reports_what_the_real_run_does(db: PgPool) {
    let app = TestApp::new(db).await;
    program(&app);
    let legacy = load("fresh.sql");
    let mut args = legacy.args("demo");
    args.dry_run = true;
    let dry = import_ok(&app, args).await;
    assert_eq!(app.scalar_i64("SELECT count(*) FROM cover_search").await, 0);
    let real = import_ok(&app, legacy.args("demo")).await;
    assert_eq!(dry.lines, real.lines);
    assert!(dry.covers.is_some());
}

#[sqlx::test]
async fn offline_skips_the_search(db: PgPool) {
    let app = TestApp::new(db).await;
    program(&app);
    let legacy = load("fresh.sql");
    let mut args = legacy.args("demo");
    args.offline = true;
    let report = import_ok(&app, args).await;
    assert_eq!(
        cover_line(&report),
        "cover search           skipped (--offline): 5 without a cover; rematch-legacy searches \
         them"
    );
    assert!(app.catalog.cover_calls().is_empty());
    assert!(
        section(&report)
            .iter()
            .all(|line| line.ends_with("not searched (--offline)"))
    );
}

/// An outage stops the search after the first book; the import goes on and commits.
#[sqlx::test]
async fn an_outage_stops_the_search_not_the_import(db: PgPool) {
    let app = TestApp::new(db).await;
    program(&app);
    app.catalog.fail_covers(FakeError::Unavailable, 1);
    let legacy = load("fresh.sql");
    let report = import_ok(&app, legacy.args("demo")).await;
    assert_eq!(app.catalog.cover_calls().len(), 1);
    let covers = report.covers.clone().unwrap();
    assert_eq!(
        (covers.without, covers.not_searched),
        (CANDIDATES, CANDIDATES)
    );
    let lines = section(&report);
    assert!(
        lines[0].ends_with("not searched (Open Library unavailable)"),
        "{lines:?}"
    );
    assert!(
        lines[1..]
            .iter()
            .all(|line| line.ends_with("not searched (stopped after an outage)"))
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM cover_search").await,
        0,
        "not searched: no row, the job takes them first"
    );
    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 5);
}

/// A database error in one book's step rolls back only that step.
#[sqlx::test]
async fn a_database_error_leaves_one_book_not_searched(db: PgPool) {
    let app = TestApp::new(db).await;
    program(&app);
    app.exec(
        "CREATE FUNCTION test_fail_cover() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
             IF (SELECT title FROM works WHERE id = NEW.work_id) = 'Kaputte Nummer' THEN
                 RAISE EXCEPTION 'boom';
             END IF;
             RETURN NEW;
         END $$;
         CREATE TRIGGER test_fail_cover BEFORE INSERT OR UPDATE ON cover_search
             FOR EACH ROW EXECUTE FUNCTION test_fail_cover();",
    )
    .await;
    let legacy = load("fresh.sql");
    let report = import_ok(&app, legacy.args("demo")).await;
    let lines = section(&report);
    assert!(
        lines.contains(
            &"book 13 \"Kaputte Nummer\" by Niemand: not searched (database error)".to_owned()
        ),
        "{lines:?}"
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM cover_search").await,
        CANDIDATES as i64 - 1
    );
    assert_eq!(
        cover_of(&app, report.books[&12].work_id).await.unwrap().0,
        5100
    );
}

#[sqlx::test]
async fn the_time_budget_stops_the_search(db: PgPool) {
    let app = TestApp::new(db).await;
    program(&app);
    let legacy = load("fresh.sql");
    let mut args = legacy.args("demo");
    args.cover_budget_ms = Some(0);
    let report = import_ok(&app, args).await;
    assert!(app.catalog.cover_calls().is_empty());
    assert!(
        section(&report)
            .iter()
            .all(|line| line.ends_with("not searched (the time budget ran out)"))
    );
}

async fn rematch(app: &TestApp, dry_run: bool, pace_ms: u64) -> Report {
    importer::rematch(
        app.db.clone(),
        app.state.lookup.clone(),
        RematchArgs {
            dry_run,
            retry_wait_ms: Some(0),
            cover_pace_ms: Some(pace_ms),
        },
    )
    .await
    .unwrap()
}

/// `rematch-legacy` searches the imported books after its ISBN pass, paced, and reports like
/// the import; its dry run writes nothing.
#[sqlx::test]
async fn rematch_searches_paces_and_reports(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let mut args = legacy.args("demo");
    args.offline = true;
    import_ok(&app, args).await;
    program(&app);

    let dry = rematch(&app, true, 0).await;
    assert_eq!(app.scalar_i64("SELECT count(*) FROM cover_search").await, 0);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM cover_suggestions")
            .await,
        0
    );

    let started = Instant::now();
    let real = rematch(&app, false, 150).await;
    assert!(
        started.elapsed() >= Duration::from_millis(150 * (CANDIDATES - 1)),
        "paced"
    );
    assert_eq!(dry.covers, real.covers);
    let covers = real.covers.clone().unwrap();
    assert_eq!(
        (covers.candidates, covers.without, covers.suggested),
        (CANDIDATES, 4, 1)
    );
    assert!(cover_line(&real).contains("1 attached"));
    assert_eq!(section(&real).len(), 4);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM cover_suggestions")
            .await,
        1
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM cover_search").await,
        CANDIDATES as i64
    );
}

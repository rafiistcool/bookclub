//! The independent count check of the cutover (#52, #38): deploy/cutover/legacy-counts.sql and
//! target-counts.sql run around the import of the legacy fixtures, so their queries cannot go
//! stale against either schema, and every count they print must agree with the importer's
//! report, and every `same` line with the other side, as `deploy/cutover/compare.sh --report`
//! checks them.

mod common;
#[path = "fixtures/legacy/harness.rs"]
mod legacy;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use bookclub_server::catalog::shared::ingest::Source;
use bookclub_server::importer::Report;
use common::*;
use legacy::*;
use rusqlite::fallible_iterator::FallibleIterator;
use serde_json::json;
use sqlx::{AssertSqlSafe, PgPool, Row};

/// A file of deploy/cutover/ without the lines that only configure its command-line client
/// (sqlite3's `.mode`, psql's `\pset`, each starting with `meta`): the queries compare.sh runs.
fn queries(name: &str, meta: char) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../deploy/cutover")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|line| !line.starts_with(meta))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What legacy-counts.sql prints for the legacy file.
fn legacy_lines(legacy: &Legacy) -> Vec<String> {
    let sql = queries("legacy-counts.sql", '.');
    let conn = rusqlite::Connection::open_with_flags(
        &legacy.path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut batch = rusqlite::Batch::new(&conn, &sql);
    let mut lines = Vec::new();
    while let Some(mut statement) = batch.next().unwrap() {
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            lines.push(row.get::<_, String>(0).unwrap());
        }
    }
    lines
}

/// What target-counts.sql prints for the test's database.
async fn target_lines(app: &TestApp) -> Vec<String> {
    let sql = queries("target-counts.sql", '\\');
    sqlx::raw_sql(AssertSqlSafe(sql))
        .fetch_all(&app.db)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<String, _>(0))
        .collect()
}

/// The `<kind>|<name>|<rows>` lines of one kind.
fn numbers(lines: &[String], kind: &str) -> BTreeMap<String, u64> {
    let prefix = format!("{kind}|");
    lines
        .iter()
        .filter_map(|line| line.strip_prefix(&prefix))
        .map(|rest| {
            let (name, rows) = rest.rsplit_once('|').unwrap();
            (name.to_owned(), rows.parse().unwrap())
        })
        .collect()
}

/// The `count|<entity>|<rows>` lines.
fn counts(lines: &[String]) -> BTreeMap<String, u64> {
    numbers(lines, "count")
}

/// The `same|<name>|<rows>` lines: rows the importer creates without a report line of their
/// own, which compare.sh requires to be equal on both sides.
fn same(lines: &[String]) -> BTreeMap<String, u64> {
    numbers(lines, "same")
}

/// The `user|<id>|<username>|<what>|<rows>` lines: (id, username, what) → rows.
fn per_user(lines: &[String]) -> BTreeMap<(i64, String, String), u64> {
    lines
        .iter()
        .filter_map(|line| line.strip_prefix("user|"))
        .map(|rest| {
            let parts: Vec<&str> = rest.split('|').collect();
            let [id, username, what, rows] = parts[..] else {
                panic!("not a user line: {rest}");
            };
            (
                (id.parse().unwrap(), username.to_owned(), what.to_owned()),
                rows.parse().unwrap(),
            )
        })
        .collect()
}

/// The `hash|<id>|<username>|<password_hash>` lines, in id order.
fn accounts(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| line.starts_with("hash|"))
        .cloned()
        .collect()
}

/// Every count against the report, as compare.sh checks them: each legacy count is the source
/// column of the report line with its name, and each target count is the target column (the sum
/// for an `a+b` line). Every report line is covered, except the target column of `covers`.
fn assert_counts_match(
    legacy: &BTreeMap<String, u64>,
    target: &BTreeMap<String, u64>,
    report: &Report,
) {
    let sources: BTreeMap<String, u64> = report
        .entities
        .iter()
        .filter_map(|e| e.source.map(|n| (e.name.clone(), n)))
        .collect();
    assert_eq!(legacy, &sources, "legacy-counts.sql against the report");

    let mut covered = BTreeSet::new();
    for (key, rows) in target {
        let mut sum = 0;
        for name in key.split('+') {
            let line = report
                .entity(name)
                .unwrap_or_else(|| panic!("target-counts.sql: no report line {name}"));
            sum += line
                .target
                .unwrap_or_else(|| panic!("the report has no target for {name}"));
            assert!(covered.insert(name.to_owned()), "{name} counted twice");
        }
        assert_eq!(*rows, sum, "target-counts.sql: {key}");
    }
    let targets: BTreeSet<String> = report
        .entities
        .iter()
        .filter(|e| e.target.is_some() && e.name != "covers")
        .map(|e| e.name.clone())
        .collect();
    assert_eq!(covered, targets, "target-counts.sql against the report");
}

/// The Open Library work the fixture's matched ISBN row resolves to, so the import merges book
/// 10 into book 9 (as in importer_run.rs).
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

/// fresh.sql: every count agrees with the report; per user, the only differences are the
/// report's merges and skips; the accounts are copied byte for byte, until a login re-hashes
/// one.
#[sqlx::test]
async fn the_counts_agree_with_the_report_of_the_fresh_fixture(db: PgPool) {
    let app = TestApp::new(db).await;
    program_merge(&app);
    let legacy = load("fresh.sql");
    let before = legacy_lines(&legacy);
    let report = import_ok(&app, legacy.args("demo")).await;
    let after = target_lines(&app).await;

    let (legacy_counts, target_counts) = (counts(&before), counts(&after));
    assert_counts_match(&legacy_counts, &target_counts, &report);
    assert_eq!((legacy_counts["users"], target_counts["users"]), (5, 5));
    assert_eq!((legacy_counts["books"], target_counts["books"]), (17, 16));
    assert_eq!(target_counts["book_posts+club_pick_posts"], 13);

    // Every user is a member of the club, and the three redeemed invites keep their redemption.
    let legacy_same = same(&before);
    assert_eq!(
        legacy_same,
        BTreeMap::from([
            ("club_invite_uses".to_owned(), 3),
            ("club_memberships".to_owned(), 5)
        ])
    );
    assert_eq!(same(&after), legacy_same);

    let (legacy_users, target_users) = (per_user(&before), per_user(&after));
    assert_eq!(
        legacy_users.keys().collect::<Vec<_>>(),
        target_users.keys().collect::<Vec<_>>()
    );
    assert_eq!(legacy_users.len(), 5 * 11);
    let differences: Vec<String> = legacy_users
        .iter()
        .filter(|(key, rows)| target_users[*key] != **rows)
        .map(|(key, rows)| format!("{} {} {rows} → {}", key.1, key.2, target_users[key]))
        .collect();
    assert_eq!(
        differences,
        [
            // club_pick_posts 2 is skipped: its diary copy was deleted.
            "demo posts 4 → 3",
            // Merge: shelf 20 → shelf 19 (demo), books 10 and 9 are one work.
            "demo shelf 10 → 9",
            // Merge: user_favorites 6 → user_favorites 4.
            "anna favorites 3 → 2",
            // Max's avatar is a PNG: "1 not a JPEG or WebP image".
            "max avatar 1 → 0",
        ],
        "{differences:#?}"
    );

    assert_eq!(accounts(&before).len(), 5);
    assert_eq!(accounts(&before), accounts(&after));

    // A login re-hashes the legacy hash: the account comparison is valid only before it.
    app.anon()
        .post(
            "/api/auth/login",
            json!({"username": "lena", "password": KNOWN_PASSWORD}),
        )
        .await
        .assert_status(204);
    let relogged = accounts(&target_lines(&app).await);
    let changed: Vec<&String> = relogged
        .iter()
        .filter(|line| !accounts(&before).contains(line))
        .collect();
    assert_eq!(changed.len(), 1);
    assert!(changed[0].starts_with("hash|4|lena|"), "{changed:?}");

    // A user the import left without a membership can sign in but not see the club: both the
    // totals and the user's own line show it.
    app.exec("DELETE FROM club_memberships WHERE user_id = 4")
        .await;
    let broken = target_lines(&app).await;
    assert_eq!(same(&broken)["club_memberships"], 4);
    assert_ne!(same(&broken), legacy_same);
    let lena = (4, "lena".to_owned(), "membership".to_owned());
    assert_eq!((legacy_users[&lena], per_user(&broken)[&lena]), (1, 0));
}

/// migrated.sql (the ALTER TABLE shape): both files run on it, and every count agrees with the
/// report.
#[sqlx::test]
async fn the_counts_agree_with_the_report_of_a_migrated_database(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("migrated.sql");
    let before = legacy_lines(&legacy);
    let report = import_ok(&app, legacy.args("ada")).await;
    let after = target_lines(&app).await;

    assert_counts_match(&counts(&before), &counts(&after), &report);
    assert_eq!(counts(&after)["users"], 2);
    assert_eq!(same(&before), same(&after));
    assert_eq!(same(&after)["club_memberships"], 2);
    assert_eq!(accounts(&before), accounts(&after));
    assert_eq!(per_user(&before).len(), per_user(&after).len());
}

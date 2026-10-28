//! Helpers the `tests/importer_*.rs` files share (included with `#[path]`): the legacy fixtures
//! loaded into a temporary SQLite file, the importer flags, and a run against a `TestApp`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use bookclub_server::importer::{self, Args, ImportError, Report};

use crate::common::TestApp;

/// The password every `fresh.sql` user has (the known argon2-cffi hash in the fixture).
pub const KNOWN_PASSWORD: &str = "DemoBuecher2026!";
/// The fixture's ISBN keys (see generate.py).
pub const ISBN_MATCH: &str = "9780000000019";
pub const ISBN_UNMATCHED: &str = "9783161484100";
pub const ISBN10_KEY: &str = "316100003X";
pub const ISBN_INVALID: &str = "9780000000000";
/// The club slug the tests pass, so a dry run and a real run report the same.
pub const SLUG: &str = "thursday-readers-test";

/// A legacy database in a temporary directory.
pub struct Legacy {
    _dir: tempfile::TempDir,
    pub path: PathBuf,
}

impl Legacy {
    /// Runs SQL against the legacy file (to add edge cases before an import).
    pub fn exec(&self, sql: &str) {
        let conn = rusqlite::Connection::open(&self.path).unwrap();
        conn.execute_batch(sql).unwrap();
    }

    /// The importer flags for this database: owner `owner`, club "Thursday Readers" in
    /// Europe/Berlin with the slug [`SLUG`], lookups on, throttling not waited out.
    pub fn args(&self, owner: &str) -> Args {
        Args {
            source: self.path.clone(),
            owner: owner.to_owned(),
            club_name: "Thursday Readers".to_owned(),
            timezone: "Europe/Berlin".to_owned(),
            club_slug: Some(SLUG.to_owned()),
            dry_run: false,
            offline: false,
            allow_unmatched: false,
            retry_wait_ms: Some(0),
        }
    }
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/legacy")
        .join(name)
}

/// `fresh.sql`, `migrated.sql` or `legacy_schema.sql` loaded into a new SQLite file.
pub fn load(fixture: &str) -> Legacy {
    let sql = std::fs::read_to_string(fixture_path(fixture)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bookclub.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    // The dump lists tables alphabetically, so a row can arrive before the table its foreign
    // key names (rusqlite's bundled SQLite enforces foreign keys by default).
    conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    conn.execute_batch(&sql).unwrap();
    drop(conn);
    Legacy { _dir: dir, path }
}

/// Runs the importer with the app's lookup (the FakeCatalog) and media store.
pub async fn import(app: &TestApp, args: Args) -> Result<Report, ImportError> {
    importer::run(
        app.db.clone(),
        app.state.lookup.clone(),
        app.state.media.clone(),
        args,
    )
    .await
}

/// Runs the importer and panics on an error.
pub async fn import_ok(app: &TestApp, args: Args) -> Report {
    import(app, args)
        .await
        .unwrap_or_else(|e| panic!("the import failed: {e}"))
}

/// The legacy book ids of fresh.sql by key, read from the fixture itself.
pub fn book_id(legacy: &Legacy, key: &str) -> i64 {
    let conn = rusqlite::Connection::open(&legacy.path).unwrap();
    conn.query_row(
        "SELECT id FROM books WHERE ol_work_key = ?1",
        [key],
        |row| row.get(0),
    )
    .unwrap_or_else(|e| panic!("no legacy book {key}: {e}"))
}

/// The `DATABASE_URL` of the test's database (for running the binary against it).
pub fn database_url(app: &TestApp) -> String {
    let base = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let mut url = url::Url::parse(&base).unwrap();
    let name = app
        .db
        .connect_options()
        .get_database()
        .expect("a database name")
        .to_owned();
    url.set_path(&format!("/{name}"));
    url.to_string()
}

//! The legacy SQLite database, read once into memory (docs/phase1/legacy-mapping.md §0).
//!
//! SQLite opens a private copy of the file (and its `-wal`) read-only, never the file itself
//! (see [`read`]). Both legacy schema shapes are accepted: a missing table reads as empty, and a
//! column `COLUMN_MIGRATIONS` added later reads as its DDL default (`PRAGMA table_info`
//! decides). Values are coerced leniently (SQLite has no column types); a value that cannot be
//! read aborts with its table, id and column.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

use super::ImportError;

/// `users`.
#[derive(Debug, Clone)]
pub struct LegacyUser {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub theme: String,
    pub color_mode: String,
    pub locale: String,
    pub created_at: OffsetDateTime,
    pub notify_meeting: bool,
    pub notify_pick: bool,
    pub notify_note: bool,
    pub avatar: Option<Vec<u8>>,
    pub avatar_mime: Option<String>,
    pub avatar_updated_at: Option<OffsetDateTime>,
}

/// `invites`.
#[derive(Debug, Clone)]
pub struct LegacyInvite {
    pub id: i64,
    pub code: String,
    pub created_by_id: Option<i64>,
    pub used_by_id: Option<i64>,
    pub created_at: OffsetDateTime,
    pub used_at: Option<OffsetDateTime>,
}

/// `books`.
#[derive(Debug, Clone)]
pub struct LegacyBook {
    pub id: i64,
    pub ol_work_key: String,
    pub title: String,
    pub authors: String,
    pub cover_id: Option<i64>,
    pub cover_image_url: Option<String>,
    pub spine_color: Option<String>,
    pub year: Option<i64>,
    pub description: String,
    pub pages: Option<i64>,
    /// The raw JSON text.
    pub subjects: String,
    pub ol_rating: Option<f64>,
    pub details_fetched_at: Option<OffsetDateTime>,
}

/// `shelf`.
#[derive(Debug, Clone)]
pub struct LegacyShelf {
    pub id: i64,
    pub user_id: i64,
    pub book_id: i64,
    pub status: String,
    pub position: i64,
    pub shelf_position: Option<i64>,
    pub rating: Option<i64>,
    /// NULL (a migrated database) reads as `''`.
    pub take: String,
    pub dnf_reason: String,
    pub progress: Option<i64>,
    pub updated_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
}

/// `physical_shelf_layouts` (keyed by `user_id`).
#[derive(Debug, Clone)]
pub struct LegacyLayout {
    pub user_id: i64,
    /// The raw JSON text.
    pub layout_json: String,
}

/// `user_favorites`.
#[derive(Debug, Clone)]
pub struct LegacyFavorite {
    pub id: i64,
    pub user_id: i64,
    pub book_id: i64,
    pub position: i64,
}

/// `club_picks`.
#[derive(Debug, Clone)]
pub struct LegacyPick {
    pub id: i64,
    pub book_id: i64,
    pub set_by_id: Option<i64>,
    pub note: String,
    pub started_at: OffsetDateTime,
    pub ended_at: Option<OffsetDateTime>,
    pub meeting_at: Option<OffsetDateTime>,
    pub reminder_sent_at: Option<OffsetDateTime>,
}

/// `pick_milestones`.
#[derive(Debug, Clone)]
pub struct LegacyMilestone {
    pub id: i64,
    pub pick_id: i64,
    pub title: String,
    pub chapter_from: Option<i64>,
    pub chapter_to: Option<i64>,
    pub due_at: Option<OffsetDateTime>,
    pub position: i64,
    pub created_by_id: Option<i64>,
    pub created_at: OffsetDateTime,
}

/// `club_pick_posts`: the legacy pick notes.
#[derive(Debug, Clone)]
pub struct LegacyPickPost {
    pub id: i64,
    pub pick_id: i64,
    pub author_id: i64,
    pub body: String,
    pub spoiler_upto: Option<i64>,
    pub milestone_id: Option<i64>,
    pub created_at: OffsetDateTime,
    pub updated_at: Option<OffsetDateTime>,
}

/// `book_posts`: diary entries and the mirror copies of pick notes.
#[derive(Debug, Clone)]
pub struct LegacyPost {
    pub id: i64,
    pub book_id: i64,
    pub author_id: i64,
    pub parent_id: Option<i64>,
    pub body: String,
    pub spoiler_upto: Option<i64>,
    pub progress_at: Option<i64>,
    pub status_at: Option<String>,
    pub pick_id: Option<i64>,
    pub legacy_post_id: Option<i64>,
    pub deleted_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: Option<OffsetDateTime>,
}

/// `post_reactions` (on `club_pick_posts`) and `book_post_reactions` (on `book_posts`).
#[derive(Debug, Clone)]
pub struct LegacyReaction {
    pub id: i64,
    pub post_id: i64,
    pub user_id: i64,
    pub emoji: String,
    pub created_at: OffsetDateTime,
}

/// `quotes`.
#[derive(Debug, Clone)]
pub struct LegacyQuote {
    pub id: i64,
    pub user_id: i64,
    pub book_id: i64,
    pub body: String,
    pub page: Option<i64>,
    pub created_at: OffsetDateTime,
}

/// `next_up_votes`.
#[derive(Debug, Clone)]
pub struct LegacyVote {
    pub id: i64,
    pub started_at: OffsetDateTime,
    pub ended_at: Option<OffsetDateTime>,
    pub closes_at: Option<OffsetDateTime>,
    pub winner_nomination_id: Option<i64>,
}

/// `next_up_nominations`.
#[derive(Debug, Clone)]
pub struct LegacyNomination {
    pub id: i64,
    pub vote_id: i64,
    pub book_id: i64,
    pub nominated_by_id: Option<i64>,
    pub created_at: OffsetDateTime,
}

/// `next_up_ballots`.
#[derive(Debug, Clone)]
pub struct LegacyBallot {
    pub id: i64,
    pub vote_id: i64,
    pub nomination_id: i64,
    pub user_id: i64,
    pub created_at: OffsetDateTime,
}

/// `events`.
#[derive(Debug, Clone)]
pub struct LegacyEvent {
    pub id: i64,
    pub actor_id: Option<i64>,
    pub kind: String,
    pub book_id: Option<i64>,
    pub pick_id: Option<i64>,
    /// The raw JSON text.
    pub payload: String,
    pub created_at: OffsetDateTime,
}

/// `search_history`.
#[derive(Debug, Clone)]
pub struct LegacySearch {
    pub id: i64,
    pub user_id: i64,
    pub query: String,
    pub normalized_query: String,
    pub last_used_at: OffsetDateTime,
}

/// Everything the importer reads from the legacy database.
#[derive(Debug, Clone, Default)]
pub struct LegacyDb {
    pub users: Vec<LegacyUser>,
    pub invites: Vec<LegacyInvite>,
    pub books: Vec<LegacyBook>,
    pub shelf: Vec<LegacyShelf>,
    pub layouts: Vec<LegacyLayout>,
    pub favorites: Vec<LegacyFavorite>,
    pub picks: Vec<LegacyPick>,
    pub milestones: Vec<LegacyMilestone>,
    pub pick_posts: Vec<LegacyPickPost>,
    pub pick_post_reactions: Vec<LegacyReaction>,
    pub posts: Vec<LegacyPost>,
    pub post_reactions: Vec<LegacyReaction>,
    pub quotes: Vec<LegacyQuote>,
    pub votes: Vec<LegacyVote>,
    pub nominations: Vec<LegacyNomination>,
    pub ballots: Vec<LegacyBallot>,
    pub events: Vec<LegacyEvent>,
    pub search_history: Vec<LegacySearch>,
    /// Rows of `push_subscriptions` (dropped, only counted).
    pub push_subscriptions: u64,
}

/// Copies `path` (with its `-wal` and `-journal`, found next to the file a symlink points to)
/// into a private temporary directory, opens the copy read-only and reads every table the
/// importer maps.
///
/// The legacy app runs SQLite in WAL mode, and a copy made with `app.backup` keeps WAL mode in
/// its header. SQLite opens such a file only where it can create the `-wal` and `-shm` files
/// next to it, so a read-only open in a directory the server cannot write (the `:ro` mount of
/// `import-legacy`) fails with "attempt to write a readonly database". In the private copy it
/// can. The `-wal` holds committed rows that no checkpoint has moved into the database file
/// yet, so it is copied too. (`immutable=1` would avoid the files, but it silently ignores the
/// `-wal` and those rows.) The copy lives under `$TMPDIR` (default `/tmp`) and is removed when
/// the read ends; the source directory is never written.
pub fn read(path: &Path) -> Result<LegacyDb, ImportError> {
    let copy = PrivateCopy::of(path)?;
    let conn = Connection::open_with_flags(
        copy.db(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| ImportError::Source(format!("cannot open {}: {e}", path.display())))?;
    // The first read opens the WAL or finds a hot journal.
    if let Err(e) = conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(())) {
        if e.sqlite_error().map(|e| e.extended_code)
            == Some(rusqlite::ffi::SQLITE_READONLY_ROLLBACK)
        {
            return Err(ImportError::Source(format!(
                "{} has a hot journal (an interrupted write): open it once with sqlite3 or the \
                 legacy app to roll it back, then import it again",
                path.display()
            )));
        }
        return Err(ImportError::Source(format!(
            "cannot read {}: {e}",
            path.display()
        )));
    }
    // `conn` is dropped before `copy`, which removes the directory.
    read_conn(&conn)
}

/// The files SQLite reads with a database besides the file itself: committed rows not yet
/// checkpointed (`-wal`), and a hot rollback journal (`-journal`; with it, SQLite refuses a
/// read-only open instead of reading a half-written file). The `-shm` index is rebuilt from
/// the `-wal`.
const SIDECARS: [&str; 2] = ["-wal", "-journal"];

/// A private temporary directory (mode 0700) holding a copy of a legacy database and its
/// sidecar files; removed on drop.
struct PrivateCopy {
    dir: PathBuf,
}

impl PrivateCopy {
    /// Copies `source` and those of its [`SIDECARS`] that exist.
    ///
    /// A symlink is resolved first: SQLite looks for the sidecars of a linked database next to
    /// the file the link points to (`snap/2026-11-01.db-wal` for `bookclub.db ->
    /// snap/2026-11-01.db`), so the copy takes them from there too. Next to the link, the
    /// `-wal` would be missing and its rows silently lost.
    fn of(source: &Path) -> Result<PrivateCopy, ImportError> {
        let resolved = fs::canonicalize(source)
            .map_err(|e| ImportError::Source(format!("cannot open {}: {e}", source.display())))?;
        let source = resolved.as_path();
        let copy = PrivateCopy {
            dir: private_dir().map_err(|e| {
                ImportError::Source(format!(
                    "cannot create a private copy of {} in {}: {e}",
                    source.display(),
                    std::env::temp_dir().display()
                ))
            })?,
        };
        copy_private(source, &copy.db()).map_err(|e| {
            ImportError::Source(format!(
                "cannot copy {} into {}: {e}",
                source.display(),
                copy.dir.display()
            ))
        })?;
        for suffix in SIDECARS {
            let from = with_suffix(source, suffix);
            match copy_private(&from, &with_suffix(&copy.db(), suffix)) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(ImportError::Source(format!(
                        "cannot copy {} into {}: {e}",
                        from.display(),
                        copy.dir.display()
                    )));
                }
            }
        }
        Ok(copy)
    }

    /// The copied database file.
    fn db(&self) -> PathBuf {
        self.dir.join("legacy.db")
    }
}

impl Drop for PrivateCopy {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.dir) {
            tracing::warn!(dir = %self.dir.display(), %error, "cannot remove the copy of the legacy database");
        }
    }
}

/// A new directory `bookclub-legacy-<random>` under the temporary directory, readable by this
/// user only.
fn private_dir() -> io::Result<PathBuf> {
    let mut random = [0u8; 8];
    crate::auth::random::fill(&mut random);
    let name: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let dir = std::env::temp_dir().join(format!("bookclub-legacy-{name}"));
    fs::DirBuilder::new().mode(0o700).create(&dir)?;
    Ok(dir)
}

/// Copies `from` to the new file `to` (mode 0600), whatever the source's mode.
fn copy_private(from: &Path, to: &Path) -> io::Result<()> {
    let mut source = File::open(from)?;
    let mut target = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(to)?;
    io::copy(&mut source, &mut target)?;
    Ok(())
}

/// `<path><suffix>`, e.g. `bookclub.db-wal`.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Reads every table the importer maps from an open connection (tests load the SQL fixtures
/// into one).
pub fn read_conn(conn: &Connection) -> Result<LegacyDb, ImportError> {
    if !has_table(conn, "users")? {
        return Err(ImportError::Source(
            "this is not a Bookclub database: it has no users table".into(),
        ));
    }
    Ok(LegacyDb {
        users: read_users(conn)?,
        invites: read_invites(conn)?,
        books: read_books(conn)?,
        shelf: read_shelf(conn)?,
        layouts: read_layouts(conn)?,
        favorites: read_favorites(conn)?,
        picks: read_picks(conn)?,
        milestones: read_milestones(conn)?,
        pick_posts: read_pick_posts(conn)?,
        pick_post_reactions: read_reactions(conn, "post_reactions")?,
        posts: read_posts(conn)?,
        post_reactions: read_reactions(conn, "book_post_reactions")?,
        quotes: read_quotes(conn)?,
        votes: read_votes(conn)?,
        nominations: read_nominations(conn)?,
        ballots: read_ballots(conn)?,
        events: read_events(conn)?,
        search_history: read_search_history(conn)?,
        push_subscriptions: count_rows(conn, "push_subscriptions")?,
    })
}

fn read_users(conn: &Connection) -> Result<Vec<LegacyUser>, ImportError> {
    rows(
        conn,
        "users",
        &[
            ("id", REQUIRED),
            ("username", REQUIRED),
            ("password_hash", REQUIRED),
            ("theme", "'paper'"),
            ("color_mode", "'system'"),
            ("locale", "'en'"),
            ("created_at", REQUIRED),
            ("notify_meeting", "1"),
            ("notify_pick", "1"),
            ("notify_note", "1"),
            ("avatar", "NULL"),
            ("avatar_mime", "NULL"),
            ("avatar_updated_at", "NULL"),
        ],
        |r| {
            Ok(LegacyUser {
                id: r.id,
                username: r.req_text("username")?,
                password_hash: r.req_text("password_hash")?,
                theme: r.text("theme")?.unwrap_or_else(|| "paper".into()),
                color_mode: r.text("color_mode")?.unwrap_or_else(|| "system".into()),
                locale: r.text("locale")?.unwrap_or_else(|| "en".into()),
                created_at: r.req_time("created_at")?,
                notify_meeting: r.flag("notify_meeting", true)?,
                notify_pick: r.flag("notify_pick", true)?,
                notify_note: r.flag("notify_note", true)?,
                avatar: r.blob("avatar")?.filter(|b| !b.is_empty()),
                avatar_mime: r.text("avatar_mime")?,
                avatar_updated_at: r.time("avatar_updated_at")?,
            })
        },
    )
}

fn read_invites(conn: &Connection) -> Result<Vec<LegacyInvite>, ImportError> {
    rows(
        conn,
        "invites",
        &[
            ("id", REQUIRED),
            ("code", REQUIRED),
            ("created_by_id", "NULL"),
            ("used_by_id", "NULL"),
            ("created_at", REQUIRED),
            ("used_at", "NULL"),
        ],
        |r| {
            Ok(LegacyInvite {
                id: r.id,
                code: r.req_text("code")?,
                created_by_id: r.int("created_by_id")?,
                used_by_id: r.int("used_by_id")?,
                created_at: r.req_time("created_at")?,
                used_at: r.time("used_at")?,
            })
        },
    )
}

fn read_books(conn: &Connection) -> Result<Vec<LegacyBook>, ImportError> {
    rows(
        conn,
        "books",
        &[
            ("id", REQUIRED),
            ("ol_work_key", REQUIRED),
            ("title", REQUIRED),
            ("authors", "''"),
            ("cover_id", "NULL"),
            ("cover_image_url", "NULL"),
            ("spine_color", "NULL"),
            ("year", "NULL"),
            ("description", "''"),
            ("pages", "NULL"),
            ("subjects", "'[]'"),
            ("ol_rating", "NULL"),
            ("details_fetched_at", "NULL"),
        ],
        |r| {
            Ok(LegacyBook {
                id: r.id,
                ol_work_key: r.req_text("ol_work_key")?,
                title: r.text("title")?.unwrap_or_default(),
                authors: r.text("authors")?.unwrap_or_default(),
                cover_id: r.int("cover_id")?,
                cover_image_url: r.text("cover_image_url")?.filter(|u| !u.trim().is_empty()),
                spine_color: r.text("spine_color")?,
                year: r.int("year")?,
                description: r.text("description")?.unwrap_or_default(),
                pages: r.int("pages")?,
                subjects: r.text("subjects")?.unwrap_or_else(|| "[]".into()),
                ol_rating: r.real("ol_rating")?,
                details_fetched_at: r.time("details_fetched_at")?,
            })
        },
    )
}

fn read_shelf(conn: &Connection) -> Result<Vec<LegacyShelf>, ImportError> {
    rows(
        conn,
        "shelf",
        &[
            ("id", REQUIRED),
            ("user_id", REQUIRED),
            ("book_id", REQUIRED),
            ("status", REQUIRED),
            ("position", "0"),
            ("shelf_position", "NULL"),
            ("rating", "NULL"),
            ("take", "''"),
            ("dnf_reason", "''"),
            ("progress", "NULL"),
            ("updated_at", REQUIRED),
            ("started_at", "NULL"),
            ("finished_at", "NULL"),
        ],
        |r| {
            Ok(LegacyShelf {
                id: r.id,
                user_id: r.req_int("user_id")?,
                book_id: r.req_int("book_id")?,
                status: r.req_text("status")?,
                position: r.int("position")?.unwrap_or(0),
                shelf_position: r.int("shelf_position")?,
                rating: r.int("rating")?,
                take: r.text("take")?.unwrap_or_default(),
                dnf_reason: r.text("dnf_reason")?.unwrap_or_default(),
                progress: r.int("progress")?,
                updated_at: r.req_time("updated_at")?,
                started_at: r.time("started_at")?,
                finished_at: r.time("finished_at")?,
            })
        },
    )
}

fn read_layouts(conn: &Connection) -> Result<Vec<LegacyLayout>, ImportError> {
    rows_by(
        conn,
        "physical_shelf_layouts",
        "user_id",
        &[("user_id", REQUIRED), ("layout_json", REQUIRED)],
        |r| {
            Ok(LegacyLayout {
                user_id: r.id,
                layout_json: r.text("layout_json")?.unwrap_or_default(),
            })
        },
    )
}

fn read_favorites(conn: &Connection) -> Result<Vec<LegacyFavorite>, ImportError> {
    rows(
        conn,
        "user_favorites",
        &[
            ("id", REQUIRED),
            ("user_id", REQUIRED),
            ("book_id", REQUIRED),
            ("position", REQUIRED),
        ],
        |r| {
            Ok(LegacyFavorite {
                id: r.id,
                user_id: r.req_int("user_id")?,
                book_id: r.req_int("book_id")?,
                position: r.req_int("position")?,
            })
        },
    )
}

fn read_picks(conn: &Connection) -> Result<Vec<LegacyPick>, ImportError> {
    rows(
        conn,
        "club_picks",
        &[
            ("id", REQUIRED),
            ("book_id", REQUIRED),
            ("set_by_id", "NULL"),
            ("note", "''"),
            ("started_at", REQUIRED),
            ("ended_at", "NULL"),
            ("meeting_at", "NULL"),
            ("reminder_sent_at", "NULL"),
        ],
        |r| {
            Ok(LegacyPick {
                id: r.id,
                book_id: r.req_int("book_id")?,
                set_by_id: r.int("set_by_id")?,
                note: r.text("note")?.unwrap_or_default(),
                started_at: r.req_time("started_at")?,
                ended_at: r.time("ended_at")?,
                meeting_at: r.time("meeting_at")?,
                reminder_sent_at: r.time("reminder_sent_at")?,
            })
        },
    )
}

fn read_milestones(conn: &Connection) -> Result<Vec<LegacyMilestone>, ImportError> {
    rows(
        conn,
        "pick_milestones",
        &[
            ("id", REQUIRED),
            ("pick_id", REQUIRED),
            ("title", REQUIRED),
            ("chapter_from", "NULL"),
            ("chapter_to", "NULL"),
            ("due_at", "NULL"),
            ("position", "0"),
            ("created_by_id", "NULL"),
            ("created_at", REQUIRED),
        ],
        |r| {
            Ok(LegacyMilestone {
                id: r.id,
                pick_id: r.req_int("pick_id")?,
                title: r.text("title")?.unwrap_or_default(),
                chapter_from: r.int("chapter_from")?,
                chapter_to: r.int("chapter_to")?,
                due_at: r.time("due_at")?,
                position: r.int("position")?.unwrap_or(0),
                created_by_id: r.int("created_by_id")?,
                created_at: r.req_time("created_at")?,
            })
        },
    )
}

fn read_pick_posts(conn: &Connection) -> Result<Vec<LegacyPickPost>, ImportError> {
    rows(
        conn,
        "club_pick_posts",
        &[
            ("id", REQUIRED),
            ("pick_id", REQUIRED),
            ("author_id", REQUIRED),
            ("body", REQUIRED),
            ("spoiler_upto", "NULL"),
            ("milestone_id", "NULL"),
            ("created_at", REQUIRED),
            ("updated_at", "NULL"),
        ],
        |r| {
            Ok(LegacyPickPost {
                id: r.id,
                pick_id: r.req_int("pick_id")?,
                author_id: r.req_int("author_id")?,
                body: r.text("body")?.unwrap_or_default(),
                spoiler_upto: r.int("spoiler_upto")?,
                milestone_id: r.int("milestone_id")?,
                created_at: r.req_time("created_at")?,
                updated_at: r.time("updated_at")?,
            })
        },
    )
}

fn read_reactions(
    conn: &Connection,
    table: &'static str,
) -> Result<Vec<LegacyReaction>, ImportError> {
    rows(
        conn,
        table,
        &[
            ("id", REQUIRED),
            ("post_id", REQUIRED),
            ("user_id", REQUIRED),
            ("emoji", REQUIRED),
            ("created_at", REQUIRED),
        ],
        |r| {
            Ok(LegacyReaction {
                id: r.id,
                post_id: r.req_int("post_id")?,
                user_id: r.req_int("user_id")?,
                emoji: r.req_text("emoji")?,
                created_at: r.req_time("created_at")?,
            })
        },
    )
}

fn read_posts(conn: &Connection) -> Result<Vec<LegacyPost>, ImportError> {
    rows(
        conn,
        "book_posts",
        &[
            ("id", REQUIRED),
            ("book_id", REQUIRED),
            ("author_id", REQUIRED),
            ("parent_id", "NULL"),
            ("body", REQUIRED),
            ("spoiler_upto", "NULL"),
            ("progress_at", "NULL"),
            ("status_at", "NULL"),
            ("pick_id", "NULL"),
            ("legacy_post_id", "NULL"),
            ("deleted_at", "NULL"),
            ("created_at", REQUIRED),
            ("updated_at", "NULL"),
        ],
        |r| {
            Ok(LegacyPost {
                id: r.id,
                book_id: r.req_int("book_id")?,
                author_id: r.req_int("author_id")?,
                parent_id: r.int("parent_id")?,
                body: r.text("body")?.unwrap_or_default(),
                spoiler_upto: r.int("spoiler_upto")?,
                progress_at: r.int("progress_at")?,
                status_at: r.text("status_at")?.filter(|s| !s.is_empty()),
                pick_id: r.int("pick_id")?,
                legacy_post_id: r.int("legacy_post_id")?,
                deleted_at: r.time("deleted_at")?,
                created_at: r.req_time("created_at")?,
                updated_at: r.time("updated_at")?,
            })
        },
    )
}

fn read_quotes(conn: &Connection) -> Result<Vec<LegacyQuote>, ImportError> {
    rows(
        conn,
        "quotes",
        &[
            ("id", REQUIRED),
            ("user_id", REQUIRED),
            ("book_id", REQUIRED),
            ("body", REQUIRED),
            ("page", "NULL"),
            ("created_at", REQUIRED),
        ],
        |r| {
            Ok(LegacyQuote {
                id: r.id,
                user_id: r.req_int("user_id")?,
                book_id: r.req_int("book_id")?,
                body: r.text("body")?.unwrap_or_default(),
                page: r.int("page")?,
                created_at: r.req_time("created_at")?,
            })
        },
    )
}

fn read_votes(conn: &Connection) -> Result<Vec<LegacyVote>, ImportError> {
    rows(
        conn,
        "next_up_votes",
        &[
            ("id", REQUIRED),
            ("started_at", REQUIRED),
            ("ended_at", "NULL"),
            ("closes_at", "NULL"),
            ("winner_nomination_id", "NULL"),
        ],
        |r| {
            Ok(LegacyVote {
                id: r.id,
                started_at: r.req_time("started_at")?,
                ended_at: r.time("ended_at")?,
                closes_at: r.time("closes_at")?,
                winner_nomination_id: r.int("winner_nomination_id")?,
            })
        },
    )
}

fn read_nominations(conn: &Connection) -> Result<Vec<LegacyNomination>, ImportError> {
    rows(
        conn,
        "next_up_nominations",
        &[
            ("id", REQUIRED),
            ("vote_id", REQUIRED),
            ("book_id", REQUIRED),
            ("nominated_by_id", "NULL"),
            ("created_at", REQUIRED),
        ],
        |r| {
            Ok(LegacyNomination {
                id: r.id,
                vote_id: r.req_int("vote_id")?,
                book_id: r.req_int("book_id")?,
                nominated_by_id: r.int("nominated_by_id")?,
                created_at: r.req_time("created_at")?,
            })
        },
    )
}

fn read_ballots(conn: &Connection) -> Result<Vec<LegacyBallot>, ImportError> {
    rows(
        conn,
        "next_up_ballots",
        &[
            ("id", REQUIRED),
            ("vote_id", REQUIRED),
            ("nomination_id", REQUIRED),
            ("user_id", REQUIRED),
            ("created_at", REQUIRED),
        ],
        |r| {
            Ok(LegacyBallot {
                id: r.id,
                vote_id: r.req_int("vote_id")?,
                nomination_id: r.req_int("nomination_id")?,
                user_id: r.req_int("user_id")?,
                created_at: r.req_time("created_at")?,
            })
        },
    )
}

fn read_events(conn: &Connection) -> Result<Vec<LegacyEvent>, ImportError> {
    rows(
        conn,
        "events",
        &[
            ("id", REQUIRED),
            ("actor_id", "NULL"),
            ("kind", REQUIRED),
            ("book_id", "NULL"),
            ("pick_id", "NULL"),
            ("payload", "'{}'"),
            ("created_at", REQUIRED),
        ],
        |r| {
            Ok(LegacyEvent {
                id: r.id,
                actor_id: r.int("actor_id")?,
                kind: r.req_text("kind")?,
                book_id: r.int("book_id")?,
                pick_id: r.int("pick_id")?,
                payload: r.text("payload")?.unwrap_or_else(|| "{}".into()),
                created_at: r.req_time("created_at")?,
            })
        },
    )
}

fn read_search_history(conn: &Connection) -> Result<Vec<LegacySearch>, ImportError> {
    rows(
        conn,
        "search_history",
        &[
            ("id", REQUIRED),
            ("user_id", REQUIRED),
            ("query", REQUIRED),
            ("normalized_query", "''"),
            ("last_used_at", REQUIRED),
        ],
        |r| {
            Ok(LegacySearch {
                id: r.id,
                user_id: r.req_int("user_id")?,
                query: r.text("query")?.unwrap_or_default(),
                normalized_query: r.text("normalized_query")?.unwrap_or_default(),
                last_used_at: r.req_time("last_used_at")?,
            })
        },
    )
}

fn count_rows(conn: &Connection, table: &str) -> Result<u64, ImportError> {
    if !has_table(conn, table)? {
        return Ok(0);
    }
    conn.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |row| {
        row.get::<_, i64>(0)
    })
    .map(|n| n.max(0) as u64)
    .map_err(source_error)
}

// ---------------------------------------------------------------------------------------------
// Table and row access
// ---------------------------------------------------------------------------------------------

/// The default of a column that must exist: its absence aborts the import.
const REQUIRED: &str = "";

fn source_error(e: rusqlite::Error) -> ImportError {
    ImportError::Source(e.to_string())
}

fn has_table(conn: &Connection, table: &str) -> Result<bool, ImportError> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get::<_, bool>(0),
    )
    .map_err(source_error)
}

fn columns(conn: &Connection, table: &str) -> Result<HashSet<String>, ImportError> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .map_err(source_error)?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(source_error)?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(source_error)?;
    Ok(names)
}

/// Every row of `table` (none when the table does not exist), in id order. `wanted` lists each
/// column with the SQL default a missing column reads as; [`REQUIRED`] columns must exist.
/// Every table must have an `id` column, which error messages name.
fn rows<T>(
    conn: &Connection,
    table: &'static str,
    wanted: &[(&'static str, &'static str)],
    map: impl FnMut(&Cells<'_>) -> Result<T, ImportError>,
) -> Result<Vec<T>, ImportError> {
    rows_by(conn, table, "id", wanted, map)
}

/// [`rows`] for a table whose integer key column is `key` (`physical_shelf_layouts` has
/// `user_id`): rows come in key order, and [`Cells::id`] is the key.
fn rows_by<T>(
    conn: &Connection,
    table: &'static str,
    key: &'static str,
    wanted: &[(&'static str, &'static str)],
    mut map: impl FnMut(&Cells<'_>) -> Result<T, ImportError>,
) -> Result<Vec<T>, ImportError> {
    if !has_table(conn, table)? {
        return Ok(Vec::new());
    }
    let present = columns(conn, table)?;
    let mut select = Vec::with_capacity(wanted.len());
    for (name, default) in wanted {
        if present.contains(*name) {
            select.push(format!("\"{name}\" AS \"{name}\""));
        } else if *default == REQUIRED {
            return Err(ImportError::Source(format!(
                "table {table} has no column {name}"
            )));
        } else {
            select.push(format!("{default} AS \"{name}\""));
        }
    }
    let sql = format!(
        "SELECT {} FROM \"{table}\" ORDER BY \"{key}\"",
        select.join(", ")
    );
    let mut stmt = conn.prepare(&sql).map_err(source_error)?;
    let mut result = stmt.query([]).map_err(source_error)?;
    let mut out = Vec::new();
    while let Some(row) = result.next().map_err(source_error)? {
        let id = match row.get_ref(key).map_err(source_error)? {
            ValueRef::Integer(id) => id,
            other => {
                return Err(ImportError::Source(format!(
                    "{table}: a row has the {key} {other:?}, not an integer"
                )));
            }
        };
        out.push(map(&Cells { table, id, row })?);
    }
    Ok(out)
}

/// One row, with lenient typed getters whose errors name the table, id and column.
pub struct Cells<'r> {
    pub table: &'static str,
    pub id: i64,
    row: &'r rusqlite::Row<'r>,
}

impl Cells<'_> {
    fn error(&self, column: &str, problem: impl std::fmt::Display) -> ImportError {
        ImportError::Source(format!(
            "{} id {}, column {column}: {problem}",
            self.table, self.id
        ))
    }

    fn value(&self, column: &str) -> Result<ValueRef<'_>, ImportError> {
        self.row.get_ref(column).map_err(|e| self.error(column, e))
    }

    /// An integer; text holding an integer is accepted, as is a real without a fraction.
    pub fn int(&self, column: &str) -> Result<Option<i64>, ImportError> {
        match self.value(column)? {
            ValueRef::Null => Ok(None),
            ValueRef::Integer(i) => Ok(Some(i)),
            ValueRef::Real(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => Ok(Some(f as i64)),
            ValueRef::Text(t) => {
                let text = String::from_utf8_lossy(t);
                let text = text.trim();
                if text.is_empty() {
                    return Ok(None);
                }
                text.parse()
                    .map(Some)
                    .map_err(|_| self.error(column, format!("{text:?} is not an integer")))
            }
            other => Err(self.error(column, format!("{other:?} is not an integer"))),
        }
    }

    /// A number.
    pub fn real(&self, column: &str) -> Result<Option<f64>, ImportError> {
        match self.value(column)? {
            ValueRef::Null => Ok(None),
            ValueRef::Integer(i) => Ok(Some(i as f64)),
            ValueRef::Real(f) => Ok(Some(f)),
            ValueRef::Text(t) => {
                let text = String::from_utf8_lossy(t);
                let text = text.trim();
                if text.is_empty() {
                    return Ok(None);
                }
                text.parse()
                    .map(Some)
                    .map_err(|_| self.error(column, format!("{text:?} is not a number")))
            }
            other => Err(self.error(column, format!("{other:?} is not a number"))),
        }
    }

    /// Text; numbers are written out.
    pub fn text(&self, column: &str) -> Result<Option<String>, ImportError> {
        match self.value(column)? {
            ValueRef::Null => Ok(None),
            ValueRef::Text(t) | ValueRef::Blob(t) => String::from_utf8(t.to_vec())
                .map(Some)
                .map_err(|_| self.error(column, "the text is not UTF-8")),
            ValueRef::Integer(i) => Ok(Some(i.to_string())),
            ValueRef::Real(f) => Ok(Some(f.to_string())),
        }
    }

    pub fn req_int(&self, column: &str) -> Result<i64, ImportError> {
        self.int(column)?
            .ok_or_else(|| self.error(column, "the value is missing"))
    }

    pub fn req_text(&self, column: &str) -> Result<String, ImportError> {
        self.text(column)?
            .ok_or_else(|| self.error(column, "the value is missing"))
    }

    pub fn blob(&self, column: &str) -> Result<Option<Vec<u8>>, ImportError> {
        match self.value(column)? {
            ValueRef::Null => Ok(None),
            ValueRef::Blob(b) | ValueRef::Text(b) => Ok(Some(b.to_vec())),
            other => Err(self.error(column, format!("{other:?} is not a blob"))),
        }
    }

    /// A SQLite boolean (0/1); NULL reads as `default`.
    pub fn flag(&self, column: &str, default: bool) -> Result<bool, ImportError> {
        match self.value(column)? {
            ValueRef::Null => Ok(default),
            ValueRef::Integer(i) => Ok(i != 0),
            ValueRef::Real(f) => Ok(f != 0.0),
            ValueRef::Text(t) => match String::from_utf8_lossy(t).trim().to_lowercase().as_str() {
                "1" | "true" => Ok(true),
                "0" | "false" | "" => Ok(false),
                other => Err(self.error(column, format!("{other:?} is not a boolean"))),
            },
            ValueRef::Blob(_) => Err(self.error(column, "a blob is not a boolean")),
        }
    }

    /// A SQLite DATETIME: naive UTC text (see [`parse_timestamp`]).
    pub fn time(&self, column: &str) -> Result<Option<OffsetDateTime>, ImportError> {
        match self.text(column)? {
            None => Ok(None),
            Some(text) if text.trim().is_empty() => Ok(None),
            Some(text) => parse_timestamp(&text)
                .map(Some)
                .ok_or_else(|| self.error(column, format!("{text:?} is not a timestamp"))),
        }
    }

    pub fn req_time(&self, column: &str) -> Result<OffsetDateTime, ImportError> {
        self.time(column)?
            .ok_or_else(|| self.error(column, "the timestamp is missing"))
    }
}

static TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,9}))?)?)?\s*(Z|[+-]\d{2}:?\d{2})?$",
    )
    .unwrap()
});

/// A legacy SQLite DATETIME: `YYYY-MM-DD HH:MM:SS` with an optional `.ffffff`, naive UTC (as
/// SQLAlchemy stores an aware value). A `T` separator, a missing time or seconds, and a `Z` or
/// `±HH:MM` offset (converted to UTC) are tolerated. `None` for anything else.
pub fn parse_timestamp(text: &str) -> Option<OffsetDateTime> {
    let caps = TIMESTAMP.captures(text.trim())?;
    let num = |i: usize| -> Option<u32> {
        caps.get(i)
            .map_or(Some(0), |m| m.as_str().parse::<u32>().ok())
    };
    let year: i32 = caps[1].parse().ok()?;
    let month = Month::try_from(u8::try_from(num(2)?).ok()?).ok()?;
    let date = Date::from_calendar_date(year, month, u8::try_from(num(3)?).ok()?).ok()?;
    let nanos = match caps.get(7) {
        Some(m) => {
            let digits = m.as_str();
            let padded = format!("{digits:0<9}");
            padded.parse::<u32>().ok()?
        }
        None => 0,
    };
    let time = Time::from_hms_nano(
        u8::try_from(num(4)?).ok()?,
        u8::try_from(num(5)?).ok()?,
        u8::try_from(num(6)?).ok()?,
        nanos,
    )
    .ok()?;
    let offset = match caps.get(8).map(|m| m.as_str()) {
        None | Some("Z") => UtcOffset::UTC,
        Some(raw) => {
            let sign: i8 = if raw.starts_with('-') { -1 } else { 1 };
            let digits: String = raw[1..].chars().filter(char::is_ascii_digit).collect();
            let hours: i8 = digits[..2].parse().ok()?;
            let minutes: i8 = digits[2..].parse().ok()?;
            UtcOffset::from_hms(sign * hours, sign * minutes, 0).ok()?
        }
    };
    Some(
        PrimitiveDateTime::new(date, time)
            .assume_offset(offset)
            .to_offset(UtcOffset::UTC),
    )
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use time::macros::datetime;

    use super::*;

    #[test]
    fn timestamps_parse_in_both_legacy_forms() {
        assert_eq!(
            parse_timestamp("2026-10-27 11:00:28.202443"),
            Some(datetime!(2026-10-27 11:00:28.202443 UTC))
        );
        assert_eq!(
            parse_timestamp("2024-01-01 00:00:00"),
            Some(datetime!(2024-01-01 00:00:00 UTC))
        );
        assert_eq!(
            parse_timestamp("2024-01-01T08:30:00+02:00"),
            Some(datetime!(2024-01-01 06:30:00 UTC))
        );
        assert_eq!(
            parse_timestamp("2024-01-01 08:30:00.5Z"),
            Some(datetime!(2024-01-01 08:30:00.5 UTC))
        );
        assert_eq!(
            parse_timestamp("2024-01-01"),
            Some(datetime!(2024-01-01 00:00:00 UTC))
        );
        assert_eq!(parse_timestamp("yesterday"), None);
        assert_eq!(parse_timestamp("2024-02-30 00:00:00"), None);
        assert_eq!(parse_timestamp("2024-01-01 25:00:00"), None);
    }

    #[test]
    fn missing_tables_and_columns_read_as_their_defaults() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, username VARCHAR, password_hash VARCHAR,
                                 created_at DATETIME);
             INSERT INTO users VALUES (1, 'ada', 'x', '2024-01-01 00:00:00');
             CREATE TABLE books (id INTEGER PRIMARY KEY, ol_work_key VARCHAR, title VARCHAR,
                                 authors VARCHAR, cover_id INTEGER, year INTEGER);
             INSERT INTO books VALUES (1, '/works/OL1W', 'Circe', 'Madeline Miller', 1, '2018');",
        )
        .unwrap();
        let db = read_conn(&conn).unwrap();
        let user = &db.users[0];
        assert_eq!(
            (
                user.theme.as_str(),
                user.color_mode.as_str(),
                user.locale.as_str()
            ),
            ("paper", "system", "en")
        );
        assert!(user.notify_meeting && user.notify_pick && user.notify_note);
        assert!(user.avatar.is_none());
        assert!(db.invites.is_empty());
        let book = &db.books[0];
        assert_eq!(book.year, Some(2018));
        assert_eq!(
            (book.subjects.as_str(), book.description.as_str()),
            ("[]", "")
        );
    }

    /// A copy taken in the middle of a write has a hot rollback journal: the half-written file
    /// is refused with the reason instead of being read.
    #[test]
    fn a_hot_journal_is_refused_with_its_reason() {
        /// The first bytes of a rollback journal whose header SQLite has synced. A journal
        /// without them (zeros: nothing was written to the database file yet) is not hot.
        const JOURNAL_MAGIC: [u8; 8] = [0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7];

        let live = tempfile::tempdir().unwrap();
        let path = live.path().join("bookclub.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, username VARCHAR, password_hash VARCHAR,
                                 created_at DATETIME);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 2000)
             INSERT INTO users SELECT i, 'user' || i, hex(randomblob(64)), '2024-01-01' FROM n;",
        )
        .unwrap();
        let committed = fs::read(&path).unwrap();
        conn.execute_batch("BEGIN; UPDATE users SET password_hash = hex(randomblob(64));")
            .unwrap();
        // Write the changed pages into the database file while the transaction is open: SQLite
        // first syncs the journal, which holds the original pages. Cache pressure does not do
        // this reliably: next to other SQLite connections in the process, the write sometimes
        // stays in the cache.
        conn.cache_flush().unwrap();
        let copied = tempfile::tempdir().unwrap();
        let copy = copied.path().join("bookclub.db");
        fs::copy(&path, &copy).unwrap();
        fs::copy(
            with_suffix(&path, "-journal"),
            with_suffix(&copy, "-journal"),
        )
        .unwrap();
        drop(conn);
        // The premise: the copy is half-written, and its journal is hot.
        assert!(
            fs::read(&copy).unwrap() != committed,
            "no changed page reached the database file"
        );
        let journal = fs::read(with_suffix(&copy, "-journal")).unwrap();
        assert_eq!(
            journal.get(..8),
            Some(&JOURNAL_MAGIC[..]),
            "the journal header was not synced, so the journal is not hot"
        );

        let error = read(&copy).unwrap_err().to_string();
        assert!(error.contains("has a hot journal"), "{error}");
        // The source is not touched: its journal is still there.
        assert!(with_suffix(&copy, "-journal").exists());
    }

    /// The private copy is removed after the read, and a missing file names the path.
    #[test]
    fn the_private_copy_is_removed_and_a_missing_file_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bookclub.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, username VARCHAR, password_hash VARCHAR,
                                 created_at DATETIME);
             INSERT INTO users VALUES (1, 'ada', 'x', '2024-01-01 00:00:00');",
        )
        .unwrap();
        drop(conn);
        let copy = PrivateCopy::of(&path).unwrap();
        assert!(copy.db().exists());
        let dir_of_copy = copy.dir.clone();
        assert_eq!(dir_of_copy.parent(), Some(std::env::temp_dir().as_path()));
        assert!(dir_of_copy.to_string_lossy().contains("/bookclub-legacy-"));
        let mode = fs::metadata(&dir_of_copy).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        drop(copy);
        assert!(!dir_of_copy.exists());
        assert_eq!(read(&path).unwrap().users.len(), 1);

        let missing = dir.path().join("nope.db");
        let error = read(&missing).unwrap_err().to_string();
        assert!(error.contains("nope.db"), "{error}");
    }

    /// A database reached through a symlink keeps the committed rows of the `-wal` next to the
    /// file the link points to, where SQLite itself looks for them.
    #[test]
    fn a_symlinked_database_is_read_with_the_wal_next_to_its_target() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("snap")).unwrap();
        let target = dir.path().join("snap/2026-11-01.db");
        {
            let conn = Connection::open(&target).unwrap();
            conn.execute_batch(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, username VARCHAR,
                                     password_hash VARCHAR, created_at DATETIME);
                 INSERT INTO users VALUES (1, 'ada', 'x', '2024-01-01 00:00:00');",
            )
            .unwrap();
            let mode: String = conn
                .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
            // Leave the second user in the WAL: no automatic checkpoint, none on close.
            conn.set_db_config(
                rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
                true,
            )
            .unwrap();
            conn.execute_batch(
                "PRAGMA wal_autocheckpoint = 0;
                 INSERT INTO users VALUES (2, 'wally', 'x', '2024-01-02 00:00:00');",
            )
            .unwrap();
        }
        let _ = fs::remove_file(with_suffix(&target, "-shm"));
        let link = dir.path().join("bookclub.db");
        std::os::unix::fs::symlink("snap/2026-11-01.db", &link).unwrap();
        assert!(fs::metadata(with_suffix(&target, "-wal")).unwrap().len() > 0);
        assert!(!with_suffix(&link, "-wal").exists());

        let db = read(&link).unwrap();
        let names: Vec<&str> = db.users.iter().map(|u| u.username.as_str()).collect();
        assert_eq!(names, ["ada", "wally"]);

        // SQLite, opening the link in place, finds the same rows.
        let conn = Connection::open_with_flags(&link, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let users: i64 = conn
            .query_row("SELECT count(*) FROM users", [], |row| row.get(0))
            .unwrap();
        assert_eq!(users, 2);
    }

    #[test]
    fn unreadable_values_name_their_table_id_and_column() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, username VARCHAR, password_hash VARCHAR,
                                 created_at DATETIME);
             INSERT INTO users VALUES (7, 'ada', 'x', 'last tuesday');",
        )
        .unwrap();
        let error = read_conn(&conn).unwrap_err().to_string();
        assert!(error.contains("users id 7, column created_at"), "{error}");
    }
}

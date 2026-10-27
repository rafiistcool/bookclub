//! The legacy SQLite database, read once into memory (docs/phase1/legacy-mapping.md §0).
//!
//! The file is opened read-only. Both legacy schema shapes are accepted: a missing table reads
//! as empty, and a column `COLUMN_MIGRATIONS` added later reads as its DDL default
//! (`PRAGMA table_info` decides). Values are coerced leniently (SQLite has no column types);
//! a value that cannot be read aborts with its table, id and column.

use std::collections::HashSet;
use std::path::Path;
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

/// Everything the importer reads from the legacy database.
#[derive(Debug, Clone, Default)]
pub struct LegacyDb {
    pub users: Vec<LegacyUser>,
    pub invites: Vec<LegacyInvite>,
    pub books: Vec<LegacyBook>,
}

/// Opens `path` read-only and reads every table the importer maps.
pub fn read(path: &Path) -> Result<LegacyDb, ImportError> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| ImportError::Source(format!("cannot open {}: {e}", path.display())))?;
    read_conn(&conn)
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
        "SELECT {} FROM \"{table}\" ORDER BY \"id\"",
        select.join(", ")
    );
    let mut stmt = conn.prepare(&sql).map_err(source_error)?;
    let mut result = stmt.query([]).map_err(source_error)?;
    let mut out = Vec::new();
    while let Some(row) = result.next().map_err(source_error)? {
        let id = match row.get_ref("id").map_err(source_error)? {
            ValueRef::Integer(id) => id,
            other => {
                return Err(ImportError::Source(format!(
                    "{table}: a row has the id {other:?}, not an integer"
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

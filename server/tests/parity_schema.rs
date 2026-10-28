//! The #37 acceptance criterion as built (docs/phase1/parity-checklist.md, "#37 schema"):
//! club-scoped tables carry `club_id` with a foreign key, personal tables don't, with the two
//! recorded deviations: `posts` and `events` hold both scopes in one table, and
//! `post_reactions` inherits its scope through `post_id`.
//!
//! Every table of the schema is in exactly one class below, and the checklist's #37 table
//! lists the same tables in the same classes. So a new table fails
//! `club_scoped_tables_carry_club_id_and_personal_tables_do_not` until it is classified here,
//! and `the_checklist_classifies_every_table_like_the_schema_test` until the checklist says the
//! same.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use regex::Regex;
use sqlx::PgPool;

/// Club data: `club_id NOT NULL`, with a foreign key from `club_id` alone to `clubs`, or a
/// composite `(club_id, parent_id)` key with every column NOT NULL into another club table.
const CLUB: [&str; 7] = [
    "club_memberships",
    "club_removals",
    "club_picks",
    "milestones",
    "next_up_votes",
    "next_up_nominations",
    "next_up_ballots",
];
/// A club's invites, or (`club_id` NULL) a sign-up invite an instance admin minted. A foreign
/// key from `club_id` alone to `clubs`.
const CLUB_OR_INSTANCE: [&str; 1] = ["club_invites"];
/// Deviation 1: both scopes in one table, `club_id` NULL = personal. A foreign key from
/// `club_id` alone to `clubs`: a composite key such as `events_pick_fk` is MATCH SIMPLE and
/// not checked while `pick_id` is NULL, so it does not count.
const BOTH_SCOPES: [&str; 2] = ["posts", "events"];
/// Deviation 2 (and the invite log, the same pattern): no `club_id`; the row belongs to its
/// parent through a NOT NULL foreign key with ON DELETE CASCADE.
const INHERITED: [(&str, &str, &str); 2] = [
    ("post_reactions", "post_id", "posts"),
    ("club_invite_uses", "invite_id", "club_invites"),
];
/// Personal data: owned by a user through a NOT NULL `user_id` → `users`, no `club_id`.
const PERSONAL: [&str; 6] = [
    "shelf_entries",
    "physical_shelf_layouts",
    "user_favorites",
    "search_history",
    "quotes",
    "import_jobs",
];
/// Accounts, the clubs themselves, the shared catalog, media and jobs: neither club nor
/// personal data, no `club_id`.
const GLOBAL: [&str; 15] = [
    "users",
    "sessions",
    "email_tokens",
    "clubs",
    "media",
    "media_removed_hashes",
    "authors",
    "works",
    "work_authors",
    "editions",
    "edition_contributors",
    "legacy_work_keys",
    "catalog_edit_requests",
    "reports",
    "harvest_state",
];

struct ForeignKey {
    name: String,
    target: String,
    columns: Vec<String>,
    on_delete_cascade: bool,
    /// Every column of the key is NOT NULL, so the key is checked for every row.
    not_null: bool,
}

async fn tables(db: &PgPool) -> BTreeSet<String> {
    sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE'
           AND table_name <> '_sqlx_migrations'",
    )
    .fetch_all(db)
    .await
    .unwrap()
    .into_iter()
    .collect()
}

/// `Some(nullable)` when the table has a `club_id` column.
async fn club_id(db: &PgPool, table: &str) -> Option<bool> {
    sqlx::query_scalar::<_, String>(
        "SELECT is_nullable::text FROM information_schema.columns
         WHERE table_schema = 'public' AND table_name = $1 AND column_name = 'club_id'",
    )
    .bind(table)
    .fetch_optional(db)
    .await
    .unwrap()
    .map(|nullable| nullable == "YES")
}

async fn foreign_keys(db: &PgPool, table: &str) -> Vec<ForeignKey> {
    let rows: Vec<(String, String, Vec<String>, String, bool)> = sqlx::query_as(
        "SELECT c.conname::text, c.confrelid::regclass::text,
                array(SELECT a.attname::text FROM unnest(c.conkey) WITH ORDINALITY AS k(n, i)
                      JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.n
                      ORDER BY k.i),
                c.confdeltype::text,
                (SELECT bool_and(a.attnotnull) FROM unnest(c.conkey) AS k(n)
                 JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.n)
         FROM pg_constraint c
         WHERE c.contype = 'f' AND c.conrelid = $1::regclass",
    )
    .bind(table)
    .fetch_all(db)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(name, target, columns, on_delete, not_null)| ForeignKey {
            name,
            target,
            columns,
            on_delete_cascade: on_delete == "c",
            not_null,
        })
        .collect()
}

async fn has_constraint(db: &PgPool, table: &str, name: &str) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM pg_constraint WHERE conrelid = $1::regclass AND conname = $2",
    )
    .bind(table)
    .bind(name)
    .fetch_one(db)
    .await
    .unwrap()
        == 1
}

/// A foreign key from `club_id` alone to `clubs`.
fn club_id_to_clubs(key: &ForeignKey) -> bool {
    key.columns == ["club_id"] && key.target == "clubs"
}

/// Club data's key: `club_id` alone to `clubs`, or a composite key over `club_id` into another
/// club table, checked for every row (no NULL column, so MATCH SIMPLE never skips it).
fn scoped_by_club(keys: &[ForeignKey]) -> bool {
    keys.iter().any(|key| {
        club_id_to_clubs(key)
            || (key.columns.len() > 1
                && key.columns.iter().any(|column| column == "club_id")
                && CLUB.contains(&key.target.as_str())
                && key.not_null)
    })
}

#[sqlx::test]
async fn club_scoped_tables_carry_club_id_and_personal_tables_do_not(db: PgPool) {
    let mut problems = Vec::new();

    let mut classified = BTreeSet::new();
    for (_, tables) in classes() {
        for table in tables {
            if !classified.insert(table.to_owned()) {
                problems.push(format!("{table}: classified twice"));
            }
        }
    }
    let actual = tables(&db).await;
    for table in actual.difference(&classified) {
        problems.push(format!(
            "{table}: not classified (add it here and to the #37 section of the checklist)"
        ));
    }
    for table in classified.difference(&actual) {
        problems.push(format!("{table}: classified, but not in the schema"));
    }

    for table in CLUB {
        if club_id(&db, table).await != Some(false) {
            problems.push(format!("{table}: club data needs club_id NOT NULL"));
        }
        if !scoped_by_club(&foreign_keys(&db, table).await) {
            problems.push(format!(
                "{table}: needs a foreign key from club_id alone to clubs, or a composite key \
                 over club_id into a club table with every column NOT NULL"
            ));
        }
    }
    for table in CLUB_OR_INSTANCE.iter().chain(&BOTH_SCOPES) {
        if club_id(&db, table).await != Some(true) {
            problems.push(format!("{table}: needs a nullable club_id"));
        }
        if !foreign_keys(&db, table).await.iter().any(club_id_to_clubs) {
            problems.push(format!(
                "{table}: needs a foreign key from club_id alone to clubs"
            ));
        }
    }
    // Deviation 1: what keeps the two scopes apart inside one table.
    for (table, constraint) in [
        ("posts", "posts_parent_fk"),
        ("posts", "posts_club_needs_pick"),
        ("posts", "posts_visibility_scope"),
        ("events", "events_scope"),
        ("events", "events_personal_no_pick"),
    ] {
        if !has_constraint(&db, table, constraint).await {
            problems.push(format!("{table}: {constraint} is missing"));
        }
    }
    let parent = foreign_keys(&db, "posts").await;
    let parent = parent.iter().find(|key| key.name == "posts_parent_fk");
    if parent.map(|key| key.columns.clone())
        != Some(vec![
            "parent_id".to_owned(),
            "work_id".to_owned(),
            "scope_key".to_owned(),
        ])
    {
        problems.push("posts_parent_fk: not on (parent_id, work_id, scope_key)".to_owned());
    }
    // Deviation 2.
    for (table, column, target) in INHERITED {
        if club_id(&db, table).await.is_some() {
            problems.push(format!("{table}: inherits its scope, so no club_id"));
        }
        let keys = foreign_keys(&db, table).await;
        if !keys.iter().any(|key| {
            key.columns == [column] && key.target == target && key.on_delete_cascade && key.not_null
        }) {
            problems.push(format!(
                "{table}: needs {column} NOT NULL → {target} ON DELETE CASCADE"
            ));
        }
    }
    for table in PERSONAL.iter().chain(&GLOBAL) {
        if club_id(&db, table).await.is_some() {
            problems.push(format!(
                "{table}: personal and global tables carry no club_id"
            ));
        }
    }
    for table in PERSONAL {
        let keys = foreign_keys(&db, table).await;
        if !keys
            .iter()
            .any(|key| key.columns == ["user_id"] && key.target == "users" && key.not_null)
        {
            problems.push(format!(
                "{table}: personal data needs user_id NOT NULL → users"
            ));
        }
    }

    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The classes as the checklist's #37 table names them, with their tables.
fn classes() -> [(&'static str, Vec<&'static str>); 6] {
    [
        ("Club data", CLUB.to_vec()),
        ("Club or instance", CLUB_OR_INSTANCE.to_vec()),
        ("Deviation 1: both scopes", BOTH_SCOPES.to_vec()),
        (
            "Deviation 2: inherited scope",
            INHERITED.iter().map(|(table, _, _)| *table).collect(),
        ),
        ("Personal data", PERSONAL.to_vec()),
        ("Neither", GLOBAL.to_vec()),
    ]
}

/// The checklist's #37 table: class → its Tables column.
fn checklist_classes() -> BTreeMap<String, BTreeSet<String>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/phase1/parity-checklist.md");
    let text = fs::read_to_string(path).unwrap();
    let start = text
        .find("\n## #37 schema")
        .expect("the checklist has a #37 section");
    let section = &text[start + 1..];
    let section = &section[..section.find("\n## ").unwrap_or(section.len())];
    let row = Regex::new(r"(?m)^\| ([^|]+?) \| ([^|]+?) \| [^|]+ \|$").unwrap();
    let name = Regex::new(r"`(\w+)`").unwrap();
    let mut classes = BTreeMap::new();
    for found in row.captures_iter(section) {
        if &found[1] == "Class" || found[1].starts_with("---") {
            continue;
        }
        let tables = name
            .captures_iter(&found[2])
            .map(|table| table[1].to_owned())
            .collect();
        let previous = classes.insert(found[1].to_owned(), tables);
        assert!(previous.is_none(), "#37 table: two rows for {}", &found[1]);
    }
    classes
}

#[test]
fn the_checklist_classifies_every_table_like_the_schema_test() {
    let mut documented = checklist_classes();
    let mut problems = Vec::new();
    for (class, tables) in classes() {
        let tables: BTreeSet<String> = tables.iter().map(|table| table.to_string()).collect();
        let Some(listed) = documented.remove(class) else {
            problems.push(format!("#37 table: no row `| {class} | … |`"));
            continue;
        };
        for table in tables.difference(&listed) {
            problems.push(format!(
                "{class}: {table} is in this test, not in the checklist"
            ));
        }
        for table in listed.difference(&tables) {
            problems.push(format!(
                "{class}: {table} is in the checklist, not in this test"
            ));
        }
    }
    for class in documented.keys() {
        problems.push(format!("#37 table: {class} is no class of this test"));
    }
    assert!(
        problems.is_empty(),
        "docs/phase1/parity-checklist.md, #37 section:\n{}",
        problems.join("\n")
    );
}

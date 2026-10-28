//! The #37 acceptance criterion as built (docs/phase1/parity-checklist.md, "#37 schema"):
//! club-scoped tables carry `club_id` with a foreign key, personal tables don't, with the two
//! recorded deviations: `posts` and `events` hold both scopes in one table, and
//! `post_reactions` inherits its scope through `post_id`. Every table of the schema is in
//! exactly one class below, so a new table fails this test until it is classified here and in
//! the checklist.

use std::collections::BTreeSet;

use sqlx::PgPool;

/// Club data: `club_id NOT NULL`, with a foreign key over `club_id` to `clubs` or to another
/// club-scoped table (the composite `(club_id, parent_id)` keys).
const CLUB: [&str; 7] = [
    "club_memberships",
    "club_removals",
    "club_picks",
    "milestones",
    "next_up_votes",
    "next_up_nominations",
    "next_up_ballots",
];
/// A club's invites, or (`club_id` NULL) a sign-up invite an instance admin minted.
const CLUB_OR_INSTANCE: [&str; 1] = ["club_invites"];
/// Deviation 1: both scopes in one table, `club_id` NULL = personal.
const BOTH_SCOPES: [&str; 2] = ["posts", "events"];
/// Deviation 2 (and the invite log, the same pattern): no `club_id`; the row belongs to its
/// parent through a NOT NULL foreign key with ON DELETE CASCADE.
const INHERITED: [(&str, &str, &str); 2] = [
    ("post_reactions", "post_id", "posts"),
    ("club_invite_uses", "invite_id", "club_invites"),
];
/// Personal data: owned by a user, no `club_id`.
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
    let rows: Vec<(String, String, Vec<String>, String)> = sqlx::query_as(
        "SELECT c.conname::text, c.confrelid::regclass::text,
                array(SELECT a.attname::text FROM unnest(c.conkey) WITH ORDINALITY AS k(n, i)
                      JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.n
                      ORDER BY k.i),
                c.confdeltype::text
         FROM pg_constraint c
         WHERE c.contype = 'f' AND c.conrelid = $1::regclass",
    )
    .bind(table)
    .fetch_all(db)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(name, target, columns, on_delete)| ForeignKey {
            name,
            target,
            columns,
            on_delete_cascade: on_delete == "c",
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

/// A foreign key over `club_id` to `clubs` or to a club-scoped table.
fn scoped_by_club(keys: &[ForeignKey]) -> bool {
    keys.iter().any(|key| {
        key.columns.iter().any(|column| column == "club_id")
            && (key.target == "clubs" || CLUB.contains(&key.target.as_str()))
    })
}

#[sqlx::test]
async fn club_scoped_tables_carry_club_id_and_personal_tables_do_not(db: PgPool) {
    let mut problems = Vec::new();

    let classified: BTreeSet<String> = CLUB
        .iter()
        .chain(&CLUB_OR_INSTANCE)
        .chain(&BOTH_SCOPES)
        .chain(INHERITED.iter().map(|(table, _, _)| table))
        .chain(&PERSONAL)
        .chain(&GLOBAL)
        .map(|table| table.to_string())
        .collect();
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
            problems.push(format!("{table}: club_id has no foreign key to its club"));
        }
    }
    for table in CLUB_OR_INSTANCE.iter().chain(&BOTH_SCOPES) {
        if club_id(&db, table).await != Some(true) {
            problems.push(format!("{table}: needs a nullable club_id"));
        }
        if !scoped_by_club(&foreign_keys(&db, table).await) {
            problems.push(format!("{table}: club_id has no foreign key to clubs"));
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
        if !keys
            .iter()
            .any(|key| key.columns == [column] && key.target == target && key.on_delete_cascade)
        {
            problems.push(format!(
                "{table}: needs {column} → {target} ON DELETE CASCADE"
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
            .any(|key| key.columns == ["user_id"] && key.target == "users")
        {
            problems.push(format!("{table}: personal data needs user_id → users"));
        }
    }

    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

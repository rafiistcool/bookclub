//! The legacy importer's frame and accounts (docs/phase1/legacy-mapping.md §0–§2, §17): the one
//! transaction, the non-empty target, the dry run, lookups and their errors, the report, users
//! and avatars, the club, memberships and invites, on the fixtures of tests/fixtures/legacy.

mod common;
#[path = "fixtures/legacy/harness.rs"]
mod legacy;

use std::process::Command;
use std::time::Duration;

use bookclub_server::catalog::shared::ingest::Source;
use bookclub_server::importer::{self, ImportError, IsbnOutcome};
use common::*;
use legacy::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use time::OffsetDateTime;
use time::macros::datetime;

/// id, username, theme, color_mode, locale, notify_meeting, notify_note, created_at.
type UserRow = (
    i64,
    String,
    String,
    String,
    String,
    bool,
    bool,
    OffsetDateTime,
);
/// username, theme, color_mode, locale, notify_meeting, notify_pick, notify_note, created_at.
type PrefsRow = (
    String,
    String,
    String,
    String,
    bool,
    bool,
    bool,
    OffsetDateTime,
);

/// id, status, rating, take, dnf_reason, shelf_position.
type ShelfRow = (i64, String, Option<i32>, String, String, Option<i32>);

/// Every row count the importer writes, in one comparable value.
async fn counts(app: &TestApp) -> Vec<i64> {
    let mut out = Vec::new();
    for table in [
        "users",
        "media",
        "clubs",
        "club_memberships",
        "club_invites",
        "club_invite_uses",
        "works",
        "editions",
        "legacy_work_keys",
    ] {
        out.push(
            app.scalar_i64(&format!("SELECT count(*) FROM {table}"))
                .await,
        );
    }
    out
}

/// An upstream URL nothing listens on (the discard port), for the binary's catalog clients.
const UNREACHABLE: &str = "http://127.0.0.1:9";

/// The Open Library work the merge fixture's ISBN row resolves to: book 9 (`OL45883W`) and the
/// ISBN key `9780000000019` become one work.
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

/// #38 AC 1: fresh.sql imports, and every entity's target count equals its source count apart
/// from the merges and skips the report lists. The fresh shape has every `COLUMN_MIGRATIONS`
/// column, and the reader takes it.
/// parity: test_migrations.py::test_fresh_database_matches_migration_list
#[sqlx::test]
async fn the_fresh_fixture_imports_with_matching_counts(db: PgPool) {
    let app = TestApp::new(db).await;
    program_merge(&app);
    let legacy = load("fresh.sql");
    let report = import_ok(&app, legacy.args("demo")).await;

    let line = |name: &str| {
        let e = report
            .entity(name)
            .unwrap_or_else(|| panic!("no {name} line"));
        (e.source, e.target, e.detail.clone())
    };
    assert_eq!(line("users"), (Some(5), Some(5), String::new()));
    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 5);
    let (source, target, detail) = line("avatars");
    assert_eq!((source, target), (Some(3), Some(2)), "{detail}");
    assert!(detail.contains("1 not a JPEG or WebP image"), "{detail}");
    assert_eq!(
        line("club"),
        (
            None,
            Some(1),
            format!("\"Thursday Readers\" ({SLUG}), owner demo, 5 members")
        )
    );
    assert_eq!(
        line("invites"),
        (
            Some(5),
            Some(5),
            "(3 used, 2 unused imported as revoked)".to_owned()
        )
    );
    // 17 books: 10 user (BC), 1 OL, the matched ISBN row merged into the OL work, 2 unmatched
    // ISBNs (one an ISBN-10 key), 1 invalid ISBN, 1 Google volume, 1 other. Every row counts in
    // its class, the merged one too, so the classes add up to the source count.
    let (source, target, detail) = line("books");
    assert_eq!((source, target), (Some(17), Some(16)), "{detail}");
    assert_eq!(
        detail,
        "works (openlibrary 1, user 10, isbn matched 1, isbn unmatched 2, isbn error 0, \
         isbn invalid 1, google volume 1, other 1; 1 merged)"
    );
    // The editions the ISBN keys identified: the matched one (the lookup's) and the two
    // unmatched bare-facts ones.
    assert_eq!(
        line("editions"),
        (
            None,
            Some(3),
            "(the editions ISBN keys identified)".to_owned()
        )
    );
    assert_eq!(
        app.scalar_i64("SELECT count(DISTINCT work_id) FROM legacy_work_keys")
            .await,
        16
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM legacy_work_keys")
            .await,
        17
    );
    assert!(
        report.merges[0].starts_with("book 10 (/works/ISBN9780000000019) → book 9"),
        "{:?}",
        report.merges
    );

    // The rest of the entities: (line, source, target, target table, its row count). Every
    // difference is a listed merge or skip.
    let rest: [(&str, u64, u64, &str, i64); 15] = [
        ("shelf", 27, 26, "shelf_entries", 26),
        ("physical_shelf_layouts", 2, 2, "physical_shelf_layouts", 2),
        ("user_favorites", 8, 7, "user_favorites", 7),
        ("club_picks", 2, 2, "club_picks", 2),
        ("pick_milestones", 4, 4, "milestones", 4),
        // book_posts and club_pick_posts both land in posts: 12 + 1.
        ("book_posts", 12, 12, "posts WHERE id <= 13", 12),
        ("club_pick_posts", 4, 1, "posts WHERE id > 13", 1),
        // Both reaction tables land in post_reactions: 4 + 1.
        (
            "book_post_reactions",
            4,
            4,
            "post_reactions WHERE post_id <= 13",
            4,
        ),
        (
            "post_reactions",
            5,
            1,
            "post_reactions WHERE post_id > 13",
            1,
        ),
        ("quotes", 3, 3, "quotes", 3),
        ("next_up_votes", 2, 2, "next_up_votes", 2),
        ("next_up_nominations", 6, 5, "next_up_nominations", 5),
        ("next_up_ballots", 5, 5, "next_up_ballots", 5),
        ("events", 48, 48, "events", 48),
        ("search_history", 4, 4, "search_history", 4),
    ];
    for (name, source, target, table, rows) in rest {
        let (s, t, detail) = line(name);
        assert_eq!((s, t), (Some(source), Some(target)), "{name}: {detail}");
        assert_eq!(
            app.scalar_i64(&format!("SELECT count(*) FROM {table}"))
                .await,
            rows,
            "{table}"
        );
    }
    assert_eq!(
        line("push_subscriptions"),
        (Some(2), None, "(Phase 4)".to_owned())
    );
    // The merges: the book, then each collision it caused.
    assert_eq!(
        report.merges[1..],
        [
            "shelf 20 → shelf 19 (demo): books 10 and 9 are one work",
            "user_favorites 6 → user_favorites 4: books 9 and 10 are one work",
            "next_up_nominations 5 → next_up_nominations 4 (vote 2): books 10 and 9 are one work",
        ]
    );
    // The skips: the mirror copies (2 of 4 pick notes, listed in the line) and the deleted copy.
    let (_, _, detail) = line("club_pick_posts");
    assert_eq!(
        detail,
        "(2 mirrored: their book_posts copy is canonical; 1 skipped (2): their diary copy was \
         deleted; 1 new (4): imported as club posts with new ids above the kept ones)"
    );
    assert!(!report.dry_run);
    assert!(report.lines.iter().any(|l| l.starts_with("users ")));

    // Every kept-id table's sequence continues after its highest id (legacy-mapping.md §0), so
    // the first row created after the cutover cannot collide with an imported one.
    for table in [
        "users",
        "club_invites",
        "shelf_entries",
        "posts",
        "quotes",
        "club_picks",
        "milestones",
        "next_up_votes",
        "next_up_nominations",
        "events",
        "search_history",
    ] {
        let step = app
            .scalar_i64(&format!(
                "SELECT nextval(pg_get_serial_sequence('{table}', 'id'))
                        - (SELECT max(id) FROM {table})"
            ))
            .await;
        assert_eq!(step, 1, "{table}");
    }
}

/// The sequences no other test reads: invites, picks, votes and nominations continue after the
/// fixture's highest ids (nomination 5 is merged away; 6 is the highest kept one).
#[sqlx::test]
async fn the_sequences_of_invites_picks_votes_and_nominations_are_advanced(db: PgPool) {
    let app = TestApp::new(db).await;
    program_merge(&app);
    let legacy = load("fresh.sql");
    import_ok(&app, legacy.args("demo")).await;
    for (table, next) in [
        ("club_invites", 6),
        ("club_picks", 3),
        ("next_up_votes", 3),
        ("next_up_nominations", 7),
    ] {
        assert_eq!(
            app.scalar_i64(&format!(
                "SELECT nextval(pg_get_serial_sequence('{table}', 'id'))"
            ))
            .await,
            next,
            "{table}"
        );
    }
}

/// #38 AC 4: a dry run prints the same report as the real run and leaves the target empty, with
/// nothing written to the media store.
#[sqlx::test]
async fn a_dry_run_reports_like_the_real_run_and_writes_nothing(db: PgPool) {
    let app = TestApp::new(db).await;
    // An existing work: a FakeHit::New would keep the ids the dry run's rollback removed.
    let work = app.ol_work("OL45883W", "Middlemarch", "George Eliot").await;
    app.catalog.on_isbn(
        ISBN_MATCH,
        FakeHit::Existing {
            work_id: work.id,
            edition_id: None,
        },
    );
    let legacy = load("fresh.sql");
    let before = counts(&app).await;

    let mut args = legacy.args("demo");
    args.dry_run = true;
    let dry = import_ok(&app, args).await;
    assert!(dry.dry_run);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 0);
    assert_eq!(counts(&app).await, before);
    assert!(app.media.keys().is_empty(), "{:?}", app.media.keys());
    assert!(dry.to_string().starts_with("DRY RUN"));

    let real = import_ok(&app, legacy.args("demo")).await;
    assert!(!real.dry_run);
    assert_eq!(dry.lines, real.lines);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 5);
    assert_eq!(app.media.keys().len(), 2);
}

/// #38 AC 5: a second run on the now non-empty target fails with `TargetNotEmpty` and writes
/// nothing.
#[sqlx::test]
async fn a_second_run_is_refused_and_writes_nothing(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    import_ok(&app, legacy.args("demo")).await;
    let before = counts(&app).await;
    let media_before = app.media.keys();

    let error = import(&app, legacy.args("demo")).await.unwrap_err();
    assert!(matches!(error, ImportError::TargetNotEmpty), "{error}");
    assert_eq!(error.to_string(), "target database already has users");
    assert_eq!(counts(&app).await, before);
    assert_eq!(app.media.keys(), media_before);
}

/// A sign-up invite minted with `beta-invite` before the import makes the target non-empty for
/// invites: the importer refuses before its first write and before any lookup, instead of
/// failing on a duplicate invite id after every lookup ran.
#[sqlx::test]
async fn a_target_with_a_beta_invite_is_refused_before_any_lookup(db: PgPool) {
    let app = TestApp::new(db).await;
    let mut conn = app.db.acquire().await.unwrap();
    bookclub_server::admin::invites::mint_signup_invite(&mut conn, None, None, None)
        .await
        .unwrap();
    drop(conn);
    let legacy = load("fresh.sql");

    let error = import(&app, legacy.args("demo")).await.unwrap_err();
    assert!(matches!(error, ImportError::Other(_)), "{error}");
    assert_eq!(error.to_string(), importer::TARGET_HAS_INVITES);
    assert!(app.catalog.calls().is_empty(), "{:?}", app.catalog.calls());
    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 0);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM club_invites").await, 1);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM clubs").await, 0);
    assert!(app.media.keys().is_empty());
}

/// #38 AC 5 through the binary: `import-legacy` exits with 2 on a non-empty target, after an
/// offline run (no lookups, so no network) that succeeded with 0.
#[sqlx::test]
async fn the_cli_exits_with_2_on_a_non_empty_target(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let media_dir = tempfile::tempdir().unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_bookclub-server"))
            .args(["import-legacy"])
            .arg(&legacy.path)
            .args([
                "--owner",
                "demo",
                "--club-name",
                "Thursday Readers",
                "--timezone",
                "Europe/Berlin",
                "--offline",
            ])
            .env("DATABASE_URL", database_url(&app))
            .env("DEV_MODE", "true")
            .env("MEDIA_DIR", media_dir.path())
            .env("RUST_LOG", "warn")
            // Nothing listens there: if --offline ever made a lookup, it would fail fast and
            // locally instead of reaching the real services.
            .env("OPENLIBRARY_URL", UNREACHABLE)
            .env("OPENLIBRARY_COVERS_URL", UNREACHABLE)
            .env("DNB_SRU_URL", UNREACHABLE)
            .env("DNB_OAI_URL", UNREACHABLE)
            .output()
            .unwrap()
    };
    let first = run();
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let stdout = String::from_utf8_lossy(&first.stdout);
    assert!(stdout.contains("users                  5 → 5"), "{stdout}");
    let slug: String = sqlx::query_scalar("SELECT slug FROM clubs")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(
        slug.starts_with("thursday-readers-") && slug.len() == 21,
        "{slug}"
    );
    let before = counts(&app).await;

    let second = run();
    assert_eq!(second.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&second.stderr).contains("target database already has users"));
    assert_eq!(counts(&app).await, before);
}

/// #38 AC 3: a legacy user signs in with the old password (the argon2-cffi hash is copied byte
/// for byte), has no email, and gets the email prompt.
#[sqlx::test]
async fn a_legacy_user_logs_in_with_the_old_password(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    import_ok(&app, legacy.args("demo")).await;

    let client = app.anon();
    client
        .post(
            "/api/auth/login",
            json!({"username": "lena", "password": "wrong password"}),
        )
        .await
        .assert_status(401);
    client
        .post(
            "/api/auth/login",
            json!({"username": "lena", "password": KNOWN_PASSWORD}),
        )
        .await
        .assert_status(204);
    let me = client.get("/api/auth/me").await;
    me.assert_status(200);
    let me = me.json();
    assert_eq!(me["username"], "lena");
    assert_eq!(me["email"], json!(null));
    assert_eq!(me["email_verified"], json!(false));
    assert_eq!(me["needs_email_prompt"], json!(true));
    assert_eq!(me["timezone"], "Europe/Berlin");
    assert_eq!(
        (&me["theme"], &me["color_mode"], &me["locale"]),
        (&json!("forest"), &json!("dark"), &json!("en"))
    );
    assert_eq!(me["is_admin"], json!(false));
    assert!(me["avatar_url"].as_str().unwrap().starts_with("/media/"));
    assert_eq!(me["clubs"][0]["slug"], SLUG);
    assert_eq!(me["clubs"][0]["role"], "member");
    assert_eq!(me["clubs"][0]["timezone"], "Europe/Berlin");

    // The imported club works for them: its members, with what they are reading (the imported
    // shelf entries).
    let members = client.get(&format!("/api/clubs/{SLUG}/members")).await;
    members.assert_status(200);
    let members = members.json();
    let items = members["items"].as_array().unwrap();
    assert_eq!(items.len(), 5);
    assert_eq!(
        (&items[0]["username"], &items[0]["role"]),
        (&json!("demo"), &json!("owner"))
    );
    let lena = items.iter().find(|m| m["username"] == "lena").unwrap();
    assert_eq!(lena["you"], json!(true));
    assert_eq!(lena["currently_reading_count"], json!(1));
    let demo = &items[0];
    assert_eq!(demo["currently_reading_count"], json!(2));
}

/// Users keep their ids, hashes, preferences and creation time; values outside the target's
/// sets get the legacy defaults (counted); the sequence continues after the highest id.
#[sqlx::test]
async fn users_keep_their_ids_hashes_and_preferences(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let report = import_ok(&app, legacy.args("demo")).await;

    let rows: Vec<UserRow> = sqlx::query_as(
        "SELECT id, username, theme, color_mode, locale, notify_meeting, notify_note,
                    created_at
             FROM users ORDER BY id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    let names: Vec<(i64, &str)> = rows.iter().map(|r| (r.0, r.1.as_str())).collect();
    assert_eq!(
        names,
        vec![
            (1, "demo"),
            (2, "anna"),
            (3, "max"),
            (4, "lena"),
            (5, "otto")
        ]
    );
    let lena = &rows[3];
    assert_eq!(
        (lena.2.as_str(), lena.3.as_str(), lena.4.as_str()),
        ("forest", "dark", "en")
    );
    assert!(!lena.5 && !lena.6, "lena turned every notification off");
    assert_eq!(lena.7, datetime!(2026-08-02 12:00:00.123456 UTC));
    // otto had theme "sepia" and locale "fr".
    let otto = &rows[4];
    assert_eq!((otto.2.as_str(), otto.4.as_str()), ("paper", "en"));
    assert_eq!(report.normalised("users.theme"), 1);
    assert_eq!(report.normalised("users.locale"), 1);
    assert_eq!(report.normalised("users.color_mode"), 0);

    let same: i64 = app
        .scalar_i64(
            "SELECT count(*) FROM users
             WHERE password_hash = '$argon2id$v=19$m=65536,t=3,p=4$eNpQ0hjz4XDBl334aMDIDg$OWwxoO8q9V8SLqTzhxy4R5WPEPmw3rB+36vq4hPrdeM'
               AND email IS NULL AND email_verified_at IS NULL AND NOT is_admin
               AND timezone = 'Europe/Berlin' AND password_changed_at = created_at",
        )
        .await;
    assert_eq!(same, 5);
    let next: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ('newbie', 'x') RETURNING id",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(next, 6);
}

/// Avatar blobs become `legacy` media rows with their bytes unchanged under
/// `avatars/<pp>/<public_id>/a`; the size comes from the JPEG header; a blob without
/// `avatar_updated_at` gets the import time; a PNG (not a type the media table holds) is left
/// out and counted.
#[sqlx::test]
async fn avatars_move_to_the_media_store_unchanged(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let conn = rusqlite::Connection::open(&legacy.path).unwrap();
    let blob = |name: &str| -> Vec<u8> {
        conn.query_row(
            "SELECT avatar FROM users WHERE username = ?1",
            [name],
            |r| r.get(0),
        )
        .unwrap()
    };
    let (lena_blob, otto_blob) = (blob("lena"), blob("otto"));
    let started = OffsetDateTime::now_utc();
    import_ok(&app, legacy.args("demo")).await;

    #[derive(sqlx::FromRow)]
    struct Avatar {
        username: String,
        public_id: String,
        kind: String,
        source: String,
        content_type: Option<String>,
        variants: Vec<String>,
        width: Option<i32>,
        height: Option<i32>,
        byte_size: Option<i64>,
        sha256: Option<Vec<u8>>,
        uploaded_by: Option<i64>,
        storage_key: Option<String>,
        created_at: OffsetDateTime,
        user_id: i64,
    }
    let avatars: Vec<Avatar> = sqlx::query_as(
        "SELECT u.username, m.public_id, m.kind, m.source, m.content_type, m.variants, m.width,
                m.height, m.byte_size, m.sha256, m.uploaded_by, m.storage_key, m.created_at,
                u.id AS user_id
         FROM users u JOIN media m ON m.id = u.avatar_media_id ORDER BY u.id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    let names: Vec<&str> = avatars.iter().map(|a| a.username.as_str()).collect();
    assert_eq!(names, vec!["lena", "otto"]);
    for (avatar, bytes) in avatars.iter().zip([&lena_blob, &otto_blob]) {
        assert_eq!(
            (avatar.kind.as_str(), avatar.source.as_str()),
            ("avatar", "legacy")
        );
        assert_eq!(avatar.content_type.as_deref(), Some("image/jpeg"));
        assert_eq!(avatar.variants, vec!["a".to_owned()]);
        assert_eq!((avatar.width, avatar.height), (Some(256), Some(256)));
        assert_eq!(avatar.byte_size, Some(bytes.len() as i64));
        assert_eq!(
            avatar.sha256.as_deref(),
            Some(Sha256::digest(bytes).as_slice())
        );
        assert_eq!(avatar.uploaded_by, Some(avatar.user_id));
        let prefix = format!("avatars/{}/{}", &avatar.public_id[..2], avatar.public_id);
        assert_eq!(avatar.storage_key.as_deref(), Some(prefix.as_str()));
        let key = format!("{prefix}/a");
        let stored = app.state.media.get(&key).await.unwrap().unwrap();
        assert_eq!(stored.as_ref(), bytes.as_slice(), "{key}");
        assert_eq!(app.media.content_type(&key).as_deref(), Some("image/jpeg"));
    }
    assert_eq!(
        avatars[0].created_at,
        datetime!(2026-08-29 12:00:00.123456 UTC)
    );
    assert!(avatars[1].created_at >= started - Duration::from_secs(5));
    assert_eq!(
        app.scalar_i64(
            "SELECT count(*) FROM users WHERE username = 'max' AND avatar_media_id IS NULL"
        )
        .await,
        1
    );
}

/// The club gets `--club-name`, `--timezone` and `--club-slug`; `--owner` owns it and everyone
/// else is a member who joined when their account was created, invited by whoever minted the
/// invite they redeemed.
#[sqlx::test]
async fn everyone_joins_the_club_and_the_owner_owns_it(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    import_ok(&app, legacy.args("Demo ")).await;

    let club: (i64, String, String, String, Option<i64>, OffsetDateTime) =
        sqlx::query_as("SELECT id, slug, name, timezone, created_by, created_at FROM clubs")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        (club.1.as_str(), club.2.as_str(), club.3.as_str(), club.4),
        (SLUG, "Thursday Readers", "Europe/Berlin", Some(1))
    );
    // The club is as old as its first member.
    assert_eq!(club.5, datetime!(2026-07-18 12:00:00.123456 UTC));
    let members: Vec<(String, String, Option<String>, bool)> = sqlx::query_as(
        "SELECT u.username, m.role, i.username, m.joined_at = u.created_at
         FROM club_memberships m JOIN users u ON u.id = m.user_id
         LEFT JOIN users i ON i.id = m.invited_by
         WHERE m.club_id = $1 ORDER BY u.id",
    )
    .bind(club.0)
    .fetch_all(&app.db)
    .await
    .unwrap();
    let expected: Vec<(String, String, Option<String>, bool)> = [
        ("demo", "owner", None),
        ("anna", "member", None),
        ("max", "member", None),
        ("lena", "member", Some("demo")),
        ("otto", "member", Some("anna")),
    ]
    .into_iter()
    .map(|(u, r, i)| (u.to_owned(), r.to_owned(), i.map(str::to_owned), true))
    .collect();
    assert_eq!(members, expected);
}

/// Invites keep id, code, creator and creation time in the club; every one is single-use
/// without expiry. A used one has its redemption (a missing `used_at` is the redeemer's
/// sign-up); an unused one is revoked.
#[sqlx::test]
async fn invites_keep_their_history_and_unused_ones_are_revoked(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    import_ok(&app, legacy.args("demo")).await;

    #[derive(Debug, PartialEq, sqlx::FromRow)]
    struct Invite {
        id: i64,
        code: String,
        created_by: Option<i64>,
        max_uses: Option<i32>,
        use_count: i32,
        expires_at: Option<OffsetDateTime>,
        revoked: bool,
        used_by: Option<i64>,
        used_at: Option<OffsetDateTime>,
        in_club: bool,
    }
    let invites: Vec<Invite> = sqlx::query_as(
        "SELECT i.id, i.code, i.created_by, i.max_uses, i.use_count, i.expires_at,
                i.revoked_at IS NOT NULL AS revoked, u.user_id AS used_by, u.used_at,
                i.club_id = (SELECT id FROM clubs) AS in_club
         FROM club_invites i LEFT JOIN club_invite_uses u ON u.invite_id = i.id
         ORDER BY i.id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    let invite = |id, code: &str, created_by, used: Option<(i64, OffsetDateTime)>| Invite {
        id,
        code: code.to_owned(),
        created_by,
        max_uses: Some(1),
        use_count: i32::from(used.is_some()),
        expires_at: None,
        revoked: used.is_none(),
        used_by: used.map(|u| u.0),
        used_at: used.map(|u| u.1),
        in_club: true,
    };
    assert_eq!(
        invites,
        vec![
            invite(1, "DEMOTEST2026", Some(1), None),
            invite(
                2,
                "BOOTSTRAPSECRETCODE2024",
                None,
                Some((1, datetime!(2026-07-18 12:00:00.123456 UTC)))
            ),
            invite(
                3,
                "LENA7K2M9Q",
                Some(1),
                Some((4, datetime!(2026-08-02 12:00:00.123456 UTC)))
            ),
            // No used_at: otto's sign-up time.
            invite(
                4,
                "OTTO4H8P2X",
                Some(2),
                Some((5, datetime!(2026-08-12 12:00:00.123456 UTC)))
            ),
            invite(5, "CLIMINTED01", None, None),
        ]
    );
    // A revoked legacy code cannot be redeemed.
    let newcomer = app.user("newcomer").await;
    newcomer
        .post("/api/invites/DEMOTEST2026/accept", json!({}))
        .await
        .assert_detail(400, "That invite code is not valid");
}

/// #38 AC 2: migrated.sql (the ALTER TABLE shape, timestamps without fractions) imports with
/// the column defaults: preferences paper/system/en, every notification on, and the shelf's
/// NULL take and dnf_reason as ''.
/// parity: test_migrations.py::test_legacy_database_is_upgraded_in_place
#[sqlx::test]
async fn migrated_databases_import_with_the_defaults(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("migrated.sql");
    let report = import_ok(&app, legacy.args("ada")).await;
    assert_eq!(report.entity("users").unwrap().target, Some(2));
    let users: Vec<PrefsRow> = sqlx::query_as(
        "SELECT username, theme, color_mode, locale, notify_meeting, notify_pick,
                    notify_note, created_at
             FROM users ORDER BY id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(users.len(), 2);
    for u in &users {
        assert_eq!(
            (u.1.as_str(), u.2.as_str(), u.3.as_str(), u.4, u.5, u.6),
            ("paper", "system", "en", true, true, true),
            "{}",
            u.0
        );
    }
    assert_eq!(users[0].7, datetime!(2024-01-01 00:00:00 UTC));
    // A junk hash is copied too; it simply never verifies.
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM users WHERE username = 'ada' AND password_hash = 'x'")
            .await,
        1
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM legacy_work_keys")
            .await,
        2
    );
    // The shelf columns ALTER TABLE added: NULL take and dnf_reason import as '', an old row
    // has no rating and is not arranged.
    let shelf: Vec<ShelfRow> = sqlx::query_as(
        "SELECT id, status, rating, take, dnf_reason, shelf_position
         FROM shelf_entries ORDER BY id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    let row = |id: i64, status: &str, rating: Option<i32>| {
        (
            id,
            status.to_owned(),
            rating,
            String::new(),
            String::new(),
            None,
        )
    };
    assert_eq!(
        shelf,
        vec![
            row(1, "finished", None),
            row(2, "did_not_finish", None),
            row(3, "finished", Some(4)),
        ]
    );
    // The book's subjects ('[]' after the boot) and the note mirrored at the boot.
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM works WHERE subjects = '{}'")
            .await,
        2
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM posts WHERE club_id IS NOT NULL AND pick_id = 1")
            .await,
        1
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM next_up_votes WHERE closes_at IS NULL")
            .await,
        1
    );
}

/// The very old shape: LEGACY_SCHEMA as written, never booted, so the users table has none of
/// the preference columns. The reader takes their DDL defaults: paper/system/en and every
/// notification on, as Python's backfill gave the users that existed before the columns.
/// parity: test_migrations.py::test_legacy_database_is_upgraded_in_place
/// parity: test_auth.py::test_preference_columns_backfill_pre_existing_users
#[sqlx::test]
async fn a_database_from_before_every_column_migration_imports(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("legacy_schema.sql");
    let report = import_ok(&app, legacy.args("ada")).await;
    assert_eq!(report.entity("users").unwrap().target, Some(1));
    assert_eq!(report.entity("invites").unwrap().source, Some(0));
    assert_eq!(
        app.scalar_i64(
            "SELECT count(*) FROM users WHERE theme = 'paper' AND color_mode = 'system'
               AND locale = 'en' AND notify_meeting AND notify_pick AND notify_note"
        )
        .await,
        1
    );
    let work: (String, String, Option<i32>, String) = sqlx::query_as(
        "SELECT w.title, w.author_names, w.first_publish_year, k.match
         FROM legacy_work_keys k JOIN works w ON w.id = k.work_id",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        work,
        (
            "Circe".to_owned(),
            "Madeline Miller".to_owned(),
            Some(2018),
            "openlibrary".to_owned()
        )
    );
    // The shelf row predates take, dnf_reason, rating and shelf_position.
    let shelf: (String, Option<i32>, String, String, Option<i32>) = sqlx::query_as(
        "SELECT status, rating, take, dnf_reason, shelf_position FROM shelf_entries",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        shelf,
        (
            "finished".to_owned(),
            None,
            String::new(),
            String::new(),
            None
        )
    );
}

/// #38 AC 14: a throttled lookup is waited out and retried, and the report shows the wait.
#[sqlx::test]
async fn a_throttled_lookup_is_retried_until_it_answers(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let (work, _) = app
        .dnb_work("Ein erfundenes Buch", "Erika Mustermann", "9783000000027")
        .await;
    app.catalog.on_isbn(
        ISBN_UNMATCHED,
        FakeHit::Existing {
            work_id: work.id,
            edition_id: None,
        },
    );
    app.catalog
        .fail_isbn(ISBN_UNMATCHED, FakeError::Throttled, Some(2));
    let mut args = legacy.args("demo");
    args.retry_wait_ms = Some(5);
    let report = import_ok(&app, args).await;

    let calls: Vec<String> = app
        .catalog
        .calls()
        .into_iter()
        .filter(|c| c.contains(ISBN_UNMATCHED))
        .collect();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert_eq!(report.lookup_wait, Duration::from_millis(10));
    let id = book_id(&legacy, &format!("/works/ISBN{ISBN_UNMATCHED}"));
    let line = report
        .isbns
        .iter()
        .find(|l| l.legacy_book_id == id)
        .unwrap();
    assert!(
        matches!(&line.outcome, IsbnOutcome::Matched { work_id, .. } if *work_id == work.id),
        "{line:?}"
    );
    assert_eq!(report.books[&id].work_id, work.id);
}

/// #38 AC 14: an unavailable source aborts the run with nothing written.
#[sqlx::test]
async fn an_unavailable_source_aborts_the_import(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let before = counts(&app).await;
    app.catalog
        .fail_isbn(ISBN_UNMATCHED, FakeError::Unavailable, None);
    let error = import(&app, legacy.args("demo")).await.unwrap_err();
    assert!(matches!(error, ImportError::Upstream(_)), "{error}");
    assert!(error.to_string().contains(ISBN_UNMATCHED), "{error}");
    assert_eq!(counts(&app).await, before);
    // The avatars written before the failure are removed again.
    assert!(app.media.keys().is_empty(), "{:?}", app.media.keys());
}

/// #38 AC 14: with `--allow-unmatched`, an unavailable source keeps the row as bare facts,
/// counted as `isbn_error` apart from `isbn_unmatched`.
#[sqlx::test]
async fn allow_unmatched_keeps_unresolvable_rows_as_bare_facts(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    app.catalog
        .fail_isbn(ISBN_UNMATCHED, FakeError::Unavailable, None);
    let mut args = legacy.args("demo");
    args.allow_unmatched = true;
    let report = import_ok(&app, args).await;

    let detail = &report.entity("books").unwrap().detail;
    assert!(
        detail.contains("isbn unmatched 2, isbn error 1,"),
        "{detail}"
    );
    let row: (String, String, Option<String>) = sqlx::query_as(
        "SELECT k.match, w.source, e.isbn13
         FROM legacy_work_keys k JOIN works w ON w.id = k.work_id
         LEFT JOIN editions e ON e.id = k.edition_id
         WHERE k.legacy_key = $1",
    )
    .bind(format!("/works/ISBN{ISBN_UNMATCHED}"))
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            "isbn_error".to_owned(),
            "legacy".to_owned(),
            Some(ISBN_UNMATCHED.to_owned())
        )
    );
    assert!(
        report
            .lines
            .iter()
            .any(|l| l.contains(ISBN_UNMATCHED) && l.contains("error: source unavailable")),
        "{:?}",
        report.lines
    );
}

/// `--offline` makes no lookup: an ISBN the local catalog knows (a harvested edition) is
/// matched, every other one is kept as bare facts.
#[sqlx::test]
async fn offline_imports_check_only_the_local_catalog(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let (work, edition) = app
        .dnb_work("Ein erfundenes Buch", "Erika Mustermann", ISBN_UNMATCHED)
        .await;
    program_merge(&app);
    let mut args = legacy.args("demo");
    args.offline = true;
    let report = import_ok(&app, args).await;

    assert!(app.catalog.calls().is_empty(), "{:?}", app.catalog.calls());
    let local = report.books[&book_id(&legacy, &format!("/works/ISBN{ISBN_UNMATCHED}"))];
    assert_eq!(
        (local.work_id, local.edition_id),
        (work.id, Some(edition.id))
    );
    let remote = report.books[&book_id(&legacy, &format!("/works/ISBN{ISBN_MATCH}"))];
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM works WHERE id = {} AND source = 'legacy'",
            remote.work_id
        ))
        .await,
        1
    );
    let detail = &report.entity("books").unwrap().detail;
    assert!(
        detail.contains("isbn matched 1, isbn unmatched 2,"),
        "{detail}"
    );
}

/// The importer refuses before writing: an unknown `--owner`, a username the target refuses
/// (listing the rows), a bad time zone or slug, and an unreadable timestamp (naming the table,
/// id and column).
#[sqlx::test]
async fn bad_input_is_refused_before_anything_is_written(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");

    let error = import(&app, legacy.args("nobody")).await.unwrap_err();
    assert!(matches!(error, ImportError::Source(_)), "{error}");
    assert!(error.to_string().contains("nobody"), "{error}");

    let mut args = legacy.args("demo");
    args.timezone = "Mars/Olympus".into();
    assert!(import(&app, args).await.is_err());
    let mut args = legacy.args("demo");
    args.club_slug = Some("Not A Slug".into());
    assert!(import(&app, args).await.is_err());
    let mut args = legacy.args("demo");
    args.club_name = "   ".into();
    assert!(import(&app, args).await.is_err());

    let bad = load("fresh.sql");
    bad.exec("UPDATE users SET username = 'Max Power' WHERE id = 3;");
    let error = import(&app, bad.args("demo")).await.unwrap_err();
    assert!(
        error.to_string().contains("users id 3: \"Max Power\""),
        "{error}"
    );

    let bad = load("fresh.sql");
    bad.exec("UPDATE shelf SET status = 'reading' WHERE id = 3;");
    let error = import(&app, bad.args("demo")).await.unwrap_err();
    assert!(matches!(error, ImportError::Source(_)), "{error}");
    assert!(
        error.to_string().contains("shelf id 3, column status"),
        "{error}"
    );

    let bad = load("fresh.sql");
    bad.exec("UPDATE invites SET created_at = 'soon' WHERE id = 2;");
    let error = import(&app, bad.args("demo")).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("invites id 2, column created_at"),
        "{error}"
    );

    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 0);
    assert!(app.media.keys().is_empty());
}

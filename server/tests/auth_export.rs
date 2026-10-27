//! `GET /api/auth/me/export`, the DSGVO export (endpoint-map §2, docs/phase1/conventions.md
//! §5.7).

mod common;

use std::collections::BTreeSet;
use std::io::{Cursor, Read};

use bookclub_server::media::shared::{MediaStore, NewMedia, register, urls};
use bytes::Bytes;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// Every `data.json` key and the foreign-key columns to `users(id)` it exports (§5.7). The
/// profile is the `users` row itself.
const SOURCES: &[(&str, &[&str])] = &[
    ("profile", &[]),
    ("sessions", &["sessions.user_id"]),
    ("shelf_entries", &["shelf_entries.user_id"]),
    ("physical_shelf_layout", &["physical_shelf_layouts.user_id"]),
    ("favorites", &["user_favorites.user_id"]),
    ("search_history", &["search_history.user_id"]),
    ("quotes", &["quotes.user_id"]),
    ("posts", &["posts.author_id"]),
    ("reactions", &["post_reactions.user_id"]),
    ("memberships", &["club_memberships.user_id"]),
    ("members_invited", &["club_memberships.invited_by"]),
    ("club_removals", &["club_removals.user_id"]),
    ("clubs_created", &["clubs.created_by"]),
    ("invites_created", &["club_invites.created_by"]),
    ("invite_uses", &["club_invite_uses.user_id"]),
    ("picks_set", &["club_picks.set_by"]),
    ("milestones_created", &["milestones.created_by"]),
    ("nominations", &["next_up_nominations.nominated_by"]),
    ("ballots", &["next_up_ballots.user_id"]),
    ("events", &["events.actor_id"]),
    ("import_jobs", &["import_jobs.user_id"]),
    (
        "catalog_contributions",
        &["works.created_by", "editions.created_by"],
    ),
    (
        "edit_requests",
        &[
            "catalog_edit_requests.created_by",
            "catalog_edit_requests.decided_by",
        ],
    ),
    ("reports", &["reports.created_by", "reports.resolved_by"]),
    ("uploaded_media", &["media.uploaded_by"]),
];

/// Foreign-key columns to `users(id)` the export leaves out on purpose, with the reason.
const EXEMPT: &[(&str, &str)] = &[(
    "email_tokens.user_id",
    "one-time secrets that expire within 48 hours; their one personal datum, a pending \
     address, is exported as profile.pending_email",
)];

async fn export(client: &TestClient) -> TestResponse {
    client.get("/api/auth/me/export").await
}

/// The files of an export: name → bytes.
fn unzip(res: &TestResponse) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(Cursor::new(res.body.to_vec())).expect("a ZIP");
    let mut files = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        files.push((file.name().unwrap().into_owned(), bytes));
    }
    files.sort();
    files
}

fn data_json(res: &TestResponse) -> Value {
    let files = unzip(res);
    let (_, bytes) = files
        .iter()
        .find(|(name, _)| name == "data.json")
        .expect("data.json");
    serde_json::from_slice(bytes).unwrap()
}

/// Every timestamp in the export is RFC 3339 in UTC with `Z`.
fn assert_utc_timestamps(value: &Value, path: &str) {
    match value {
        Value::Object(fields) => {
            for (key, field) in fields {
                let here = format!("{path}.{key}");
                if key.ends_with("_at")
                    && let Value::String(text) = field
                    && !matches!(
                        text.as_str(),
                        "want_to_read" | "currently_reading" | "finished" | "did_not_finish"
                    )
                {
                    assert!(text.ends_with('Z'), "{here} = {text}");
                }
                assert_utc_timestamps(field, &here);
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_utc_timestamps(item, path);
            }
        }
        _ => {}
    }
}

/// parity: test_backup.py::test_backup_requires_auth
///
/// Reasoned change: the instance backup is replaced by the personal export (Postgres backups
/// are the operator's job, #51).
#[sqlx::test]
async fn the_export_needs_a_session(db: PgPool) {
    let app = TestApp::new(db).await;
    export(&app.anon())
        .await
        .assert_detail(401, "Not signed in");
}

/// parity: test_backup.py::test_backup_returns_sqlite_for_member
/// parity: test_selfhost.py::test_backup_copies_sqlite
///
/// Reasoned change: a member downloads a ZIP of their own data, not the club's SQLite file.
#[sqlx::test]
async fn a_member_downloads_a_zip_of_their_data(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    app.exec("UPDATE users SET timezone = 'Pacific/Kiritimati' WHERE username = 'ada'")
        .await;
    let res = export(&ada).await;
    res.assert_status(200);
    assert_eq!(res.header("content-type"), Some("application/zip"));
    assert_eq!(res.header("cache-control"), Some("no-store"));
    let today: String =
        sqlx::query_scalar("SELECT to_char(now() AT TIME ZONE 'Pacific/Kiritimati', 'YYYYMMDD')")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        res.header("content-disposition"),
        Some(format!("attachment; filename=\"bookclub-export-ada-{today}.zip\"").as_str())
    );
    assert!(res.body.starts_with(b"PK\x03\x04"));
    let names: Vec<String> = unzip(&res).into_iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["data.json"]);

    let data = data_json(&res);
    let profile = &data["profile"];
    assert_eq!(profile["username"], "ada");
    assert_eq!(profile["email"], "ada@example.test");
    assert_eq!(profile["timezone"], "Pacific/Kiritimati");
    assert_eq!(profile["pending_email"], Value::Null);
    assert!(profile.get("password_hash").is_none());
    let sessions = data["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["kind"], "cookie");
    assert!(sessions[0].get("token_hash").is_none());
    assert_utc_timestamps(&data, "data");
    let text = String::from_utf8(
        unzip(&res)
            .into_iter()
            .find(|(name, _)| name == "data.json")
            .unwrap()
            .1,
    )
    .unwrap();
    assert!(!text.contains("$argon2"), "no password hash anywhere");
}

/// §5.7: every foreign-key column that references `users(id)` is exported or exempted with a
/// reason, and `data.json` has exactly one key per entry of [`SOURCES`]. A new column that
/// points at users fails here until the export covers it.
#[sqlx::test]
async fn the_export_covers_every_foreign_key_to_users(db: PgPool) {
    let app = TestApp::new(db).await;
    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT c.conrelid::regclass::text || '.' || a.attname
         FROM pg_constraint c
         CROSS JOIN LATERAL unnest(c.conkey) AS k(attnum)
         JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum
         WHERE c.contype = 'f' AND c.confrelid = 'users'::regclass",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    let in_schema: BTreeSet<String> = columns.into_iter().collect();
    let mut covered = BTreeSet::new();
    for (_, sources) in SOURCES {
        for source in *sources {
            assert!(covered.insert(source.to_string()), "{source} twice");
        }
    }
    for (column, reason) in EXEMPT {
        assert!(!reason.is_empty());
        assert!(covered.insert(column.to_string()), "{column} twice");
    }
    assert_eq!(in_schema, covered);
    assert_eq!(in_schema.len(), 28);

    let ada = app.user("ada").await;
    let data = data_json(&export(&ada).await);
    let keys: BTreeSet<&str> = data
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let expected: BTreeSet<&str> = SOURCES.iter().map(|(key, _)| *key).collect();
    assert_eq!(keys, expected);
}

/// Stores an uploaded picture's bytes and registers its row; returns the public id.
async fn upload(app: &TestApp, uploader: i64, kind: &str, seed: u8) -> String {
    store(app, kind, "image/webp", "upload", Some(uploader), seed).await
}

/// Stores a picture's bytes (8 times `seed` per variant) and registers its row; returns the
/// public id.
async fn store(
    app: &TestApp,
    kind: &str,
    content_type: &str,
    source: &str,
    uploaded_by: Option<i64>,
    seed: u8,
) -> String {
    let public_id = urls::new_public_id();
    let variants: &[&str] = if kind == "avatar" {
        &["a"]
    } else {
        &["s", "l"]
    };
    for variant in variants {
        app.media
            .put(
                &urls::object_key(kind, &public_id, variant),
                Bytes::from(vec![seed; 8]),
                content_type,
            )
            .await
            .unwrap();
    }
    let mut conn = app.db.acquire().await.unwrap();
    register(
        &mut conn,
        &public_id,
        &NewMedia {
            kind,
            source,
            content_type,
            variants,
            width: 100,
            height: 150,
            byte_size: 8,
            sha256: [seed; 32],
            dominant_color: None,
            uploaded_by,
            created_at: None,
        },
    )
    .await
    .unwrap();
    public_id
}

/// Asserts the size of every list in an export: each array section, and each list of the
/// sections made of lists (`catalog_contributions`, `edit_requests`, `reports`), holds one row
/// unless `sizes` says otherwise.
fn assert_section_sizes(data: &Value, sizes: &[(&str, usize)]) {
    let expected = |key: &str| {
        sizes
            .iter()
            .find(|(name, _)| *name == key)
            .map_or(1, |(_, size)| *size)
    };
    for (key, section) in data.as_object().unwrap() {
        match section {
            Value::Array(rows) => assert_eq!(rows.len(), expected(key), "{key}: {rows:?}"),
            Value::Object(parts)
                if !matches!(key.as_str(), "profile" | "physical_shelf_layout") =>
            {
                for (part, rows) in parts {
                    let rows = rows.as_array().unwrap_or_else(|| panic!("{key}.{part}"));
                    let name = format!("{key}.{part}");
                    assert_eq!(rows.len(), expected(&name), "{name}: {rows:?}");
                }
            }
            _ => {}
        }
    }
}

/// Ada and Bob each have a row behind every foreign-key column of [`SOURCES`]. Each section of
/// an export holds exactly the exporting user's rows: the sizes are checked for every list, and
/// a value that tells the two users apart for every section.
#[sqlx::test]
async fn every_section_holds_the_users_rows(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cleo = app.user("cleo").await;
    let dan = app.user("dan").await;
    let (a, b) = (ada.id(), bob.id());
    let readers = app.club(&ada, "Readers").await;
    app.join(&readers, &bob, "member").await;
    app.join(&readers, &dan, "member").await;
    let old = app.club(&cleo, "Old Club").await;
    let third = app.club(&cleo, "Third Club").await;
    let work = app.work("Circe").await;
    let other = app.work("Ulysses").await;
    let edition = app.edition(&work, "9780316556347").await;
    let bobs_edition = app.edition(&other, "9780679722762").await;
    let avatar = upload(&app, a, "avatar", 1).await;
    let cover = upload(&app, a, "cover", 2).await;
    let bobs_avatar = upload(&app, b, "avatar", 3).await;
    ada.put(
        "/api/auth/me/email",
        json!({"email": "next@example.test", "current_password": PASSWORD}),
    )
    .await
    .assert_status(204);
    let (c, o, t, w, w2) = (readers.id, old.id, third.id, work.id, other.id);
    app.exec(&format!(
        "UPDATE users SET avatar_media_id = (SELECT id FROM media WHERE public_id = '{avatar}')
             WHERE id = {a};
         UPDATE clubs SET created_by = {b} WHERE id = {o};
         UPDATE club_memberships SET invited_by = {a} WHERE club_id = {c} AND user_id = {b};
         UPDATE club_memberships SET invited_by = {b} WHERE club_id = {c} AND user_id = {dan};
         INSERT INTO club_removals (club_id, user_id) VALUES ({o}, {a}), ({t}, {b});
         INSERT INTO club_invites (club_id, code, created_by, max_uses) VALUES ({c}, 'ADAS', {a}, 5);
         INSERT INTO club_invites (club_id, code, created_by, max_uses) VALUES ({o}, 'BOBS', {b}, 5);
         INSERT INTO club_invite_uses (invite_id, user_id)
             SELECT id, {a} FROM club_invites WHERE code = 'BOBS';
         INSERT INTO club_invite_uses (invite_id, user_id)
             SELECT id, {b} FROM club_invites WHERE code = 'ADAS';
         INSERT INTO club_invite_uses (invite_id, user_id)
             SELECT id, {cleo} FROM club_invites WHERE code = 'BOBS';
         INSERT INTO club_picks (club_id, work_id, set_by, note) VALUES ({c}, {w}, {a}, 'our pick');
         INSERT INTO club_picks (club_id, work_id, set_by, note) VALUES ({o}, {w}, {b}, 'not hers');
         INSERT INTO milestones (club_id, pick_id, title, created_by)
             SELECT {c}, id, 'Part one', {a} FROM club_picks WHERE club_id = {c};
         INSERT INTO milestones (club_id, pick_id, title, created_by)
             SELECT {o}, id, 'Part two', {b} FROM club_picks WHERE club_id = {o};
         INSERT INTO next_up_votes (club_id) VALUES ({c});
         INSERT INTO next_up_nominations (club_id, vote_id, work_id, nominated_by)
             SELECT {c}, id, {w2}, {a} FROM next_up_votes;
         INSERT INTO next_up_nominations (club_id, vote_id, work_id, nominated_by)
             SELECT {c}, id, {w}, {b} FROM next_up_votes;
         INSERT INTO next_up_ballots (club_id, vote_id, user_id, nomination_id)
             SELECT {c}, vote_id, nominated_by, id FROM next_up_nominations;
         INSERT INTO events (club_id, actor_id, kind) VALUES ({c}, {a}, 'member_joined');
         INSERT INTO events (club_id, actor_id, kind) VALUES ({o}, {b}, 'member_joined');
         INSERT INTO import_jobs (user_id, status, total) VALUES ({a}, 'done', 3), ({b}, 'done', 7);
         UPDATE works SET created_by = {a} WHERE id = {w};
         UPDATE works SET created_by = {b} WHERE id = {w2};
         UPDATE editions SET created_by = {a} WHERE id = {e};
         UPDATE editions SET created_by = {b} WHERE id = {e2};
         INSERT INTO catalog_edit_requests
             (kind, work_id, note, status, created_by, decided_by, decided_at, decision_note)
             VALUES ('work_edit', {w}, 'a typo', 'rejected', {a}, {b}, now(), 'not a typo'),
                    ('work_edit', {w}, 'bobs fix', 'approved', {b}, {a}, now(), 'looks right');
         INSERT INTO reports
             (target_kind, work_id, reason, details, status, created_by, resolved_by, resolved_at)
             VALUES ('cover', {w}, 'wrong_book', 'not this one', 'dismissed', {a}, {b}, now()),
                    ('cover', {w}, 'other', 'bobs report', 'removed', {b}, {a}, now());
         INSERT INTO shelf_entries (user_id, work_id, status, rating, take)
             VALUES ({a}, {w}, 'finished', 5, 'loved it'), ({b}, {w}, 'finished', 1, 'bobs take');
         INSERT INTO physical_shelf_layouts (user_id, layout)
             VALUES ({a}, '{{\"rows\": [[1]]}}'), ({b}, '{{\"rows\": [[2]]}}');
         INSERT INTO user_favorites (user_id, work_id, position) VALUES ({a}, {w}, 1), ({b}, {w2}, 1);
         INSERT INTO search_history (user_id, query, normalized_query)
             VALUES ({a}, 'Circe', 'circe'), ({b}, 'Ulysses', 'ulysses');
         INSERT INTO quotes (user_id, work_id, body, page) VALUES ({a}, {w}, 'A line', 12),
             ({b}, {w}, 'bobs line', 3);
         INSERT INTO posts (work_id, author_id, body) VALUES ({w}, {a}, 'my diary'), ({w}, {b}, 'bobs diary');
         INSERT INTO posts (work_id, author_id, club_id, pick_id, body)
             SELECT {w}, {a}, {c}, id, 'club talk' FROM club_picks WHERE club_id = {c};
         INSERT INTO post_reactions (post_id, user_id, emoji)
             SELECT id, {a}, '🔥' FROM posts WHERE body = 'bobs diary';
         INSERT INTO post_reactions (post_id, user_id, emoji)
             SELECT id, {b}, '📚' FROM posts WHERE body = 'my diary';",
        cleo = cleo.id(),
        dan = dan.id(),
        e = edition.id,
        e2 = bobs_edition.id,
    ))
    .await;

    let res = export(&ada).await;
    res.assert_status(200);
    let data = data_json(&res);
    let bobs = data_json(&export(&bob).await);
    assert_utc_timestamps(&data, "data");
    assert_section_sizes(&data, &[("posts", 2), ("uploaded_media", 2)]);
    assert_section_sizes(&bobs, &[]);

    // One value per section that tells the two users apart: Ada's, then Bob's.
    let sessions = (session_id(&app, a).await, session_id(&app, b).await);
    let first = |data: &Value, path: &str, field: &str| {
        let mut section = data;
        for key in path.split('.') {
            section = &section[key];
        }
        section[0][field].clone()
    };
    for (path, field, adas, bobs_value) in [
        ("sessions", "id", json!(sessions.0), json!(sessions.1)),
        (
            "shelf_entries",
            "take",
            json!("loved it"),
            json!("bobs take"),
        ),
        ("favorites", "work_title", json!("Circe"), json!("Ulysses")),
        ("search_history", "query", json!("Circe"), json!("Ulysses")),
        ("quotes", "body", json!("A line"), json!("bobs line")),
        ("posts", "body", json!("my diary"), json!("bobs diary")),
        ("reactions", "emoji", json!("🔥"), json!("📚")),
        ("memberships", "role", json!("owner"), json!("member")),
        ("members_invited", "username", json!("bob"), json!("dan")),
        (
            "club_removals",
            "club_name",
            json!("Old Club"),
            json!("Third Club"),
        ),
        ("clubs_created", "name", json!("Readers"), json!("Old Club")),
        ("invites_created", "code", json!("ADAS"), json!("BOBS")),
        (
            "invite_uses",
            "club_name",
            json!("Old Club"),
            json!("Readers"),
        ),
        ("picks_set", "note", json!("our pick"), json!("not hers")),
        (
            "milestones_created",
            "title",
            json!("Part one"),
            json!("Part two"),
        ),
        (
            "nominations",
            "work_title",
            json!("Ulysses"),
            json!("Circe"),
        ),
        ("ballots", "work_title", json!("Ulysses"), json!("Circe")),
        ("events", "club_name", json!("Readers"), json!("Old Club")),
        ("import_jobs", "total", json!(3), json!(7)),
        (
            "catalog_contributions.works",
            "title",
            json!("Circe"),
            json!("Ulysses"),
        ),
        (
            "catalog_contributions.editions",
            "isbn13",
            json!("9780316556347"),
            json!("9780679722762"),
        ),
        (
            "edit_requests.filed",
            "note",
            json!("a typo"),
            json!("bobs fix"),
        ),
        (
            "edit_requests.decided",
            "decision_note",
            json!("looks right"),
            json!("not a typo"),
        ),
        (
            "reports.filed",
            "details",
            json!("not this one"),
            json!("bobs report"),
        ),
        (
            "reports.resolved",
            "status",
            json!("removed"),
            json!("dismissed"),
        ),
        (
            "uploaded_media",
            "public_id",
            json!(avatar),
            json!(bobs_avatar),
        ),
    ] {
        assert_eq!(first(&data, path, field), adas, "Ada's {path}");
        assert_eq!(first(&bobs, path, field), bobs_value, "Bob's {path}");
    }
    assert_eq!(data["profile"]["pending_email"], "next@example.test");
    assert_eq!(bobs["profile"]["pending_email"], Value::Null);
    assert_eq!(
        data["physical_shelf_layout"]["layout"],
        json!({"rows": [[1]]})
    );
    assert_eq!(
        bobs["physical_shelf_layout"]["layout"],
        json!({"rows": [[2]]})
    );

    // The rows lose the reference to the user and gain readable names.
    assert_eq!(one_row(&data, "sessions")["label"], "test");
    let shelf = one_row(&data, "shelf_entries");
    assert_eq!(shelf["work_title"], "Circe");
    assert!(shelf.get("user_id").is_none());
    let posts = data["posts"].as_array().unwrap();
    let bodies: Vec<&str> = posts.iter().map(|p| p["body"].as_str().unwrap()).collect();
    assert_eq!(bodies, ["my diary", "club talk"]);
    assert_eq!(posts[0]["club_name"], Value::Null);
    assert_eq!(posts[1]["club_name"], "Readers");
    assert_eq!(one_row(&data, "memberships")["club_name"], "Readers");
    let used = one_row(&data, "invite_uses");
    assert!(used.get("code").is_none(), "another user's invite code");
    let event = one_row(&data, "events");
    assert_eq!(event["kind"], "member_joined");

    let media = data["uploaded_media"].as_array().unwrap();
    let ids: Vec<&str> = media
        .iter()
        .map(|m| m["public_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [avatar.as_str(), cover.as_str()]);
    assert_eq!(media[0]["files"], json!([format!("media/{avatar}/a.webp")]));
    assert_eq!(
        media[1]["files"],
        json!([
            format!("media/{cover}/s.webp"),
            format!("media/{cover}/l.webp")
        ])
    );
    let files = unzip(&res);
    let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
    let mut expected = vec![
        "data.json".to_owned(),
        format!("media/{avatar}/a.webp"),
        format!("media/{cover}/l.webp"),
        format!("media/{cover}/s.webp"),
    ];
    expected.sort();
    assert_eq!(names, expected);
    let avatar_bytes = &files
        .iter()
        .find(|(name, _)| name.ends_with("/a.webp"))
        .unwrap()
        .1;
    assert_eq!(avatar_bytes, &vec![1u8; 8]);
}

/// The id of the user's one session.
async fn session_id(app: &TestApp, user_id: i64) -> i64 {
    app.scalar_i64(&format!(
        "SELECT id FROM sessions WHERE user_id = {user_id}"
    ))
    .await
}

/// The one row of an array section.
fn one_row(data: &Value, key: &str) -> Value {
    let rows = data[key].as_array().unwrap_or_else(|| panic!("{key}"));
    assert_eq!(rows.len(), 1, "{key}: {rows:?}");
    rows[0].clone()
}

/// The current avatar is exported even when its row names no uploader, and a JPEG (every
/// imported avatar is one, legacy-mapping §1.1) keeps its extension.
#[sqlx::test]
async fn the_current_avatar_is_exported_as_stored(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let avatar = store(&app, "avatar", "image/jpeg", "legacy", None, 6).await;
    app.exec(&format!(
        "UPDATE users SET avatar_media_id = (SELECT id FROM media WHERE public_id = '{avatar}')
         WHERE id = {}",
        ada.id()
    ))
    .await;
    let res = export(&ada).await;
    res.assert_status(200);
    let media = data_json(&res)["uploaded_media"].clone();
    assert_eq!(media.as_array().unwrap().len(), 1);
    assert_eq!(media[0]["public_id"], avatar.as_str());
    assert_eq!(media[0]["source"], "legacy");
    assert_eq!(media[0]["content_type"], "image/jpeg");
    let path = format!("media/{avatar}/a.jpg");
    assert_eq!(media[0]["files"], json!([path]));
    let files = unzip(&res);
    let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["data.json", path.as_str()]);
    assert_eq!(files[1].1, vec![6u8; 8]);
}

/// A picture that was taken down is listed, but its bytes are gone and not exported.
#[sqlx::test]
async fn removed_pictures_are_listed_without_files(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let cover = upload(&app, ada.id(), "cover", 4).await;
    app.exec(&format!(
        "UPDATE media SET removed_at = now() WHERE public_id = '{cover}'"
    ))
    .await;
    let res = export(&ada).await;
    let data = data_json(&res);
    let media = data["uploaded_media"].as_array().unwrap();
    assert_eq!(media.len(), 1);
    assert_eq!(media[0]["files"], json!([]));
    assert!(media[0]["removed_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(unzip(&res).len(), 1);
}

#[sqlx::test]
async fn exports_are_limited_to_five_an_hour(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for _ in 0..5 {
        export(&ada).await.assert_status(200);
    }
    let limited = export(&ada).await;
    limited.assert_detail(429, "Slow down a little. Try again later.");
    assert!(limited.header("retry-after").is_some());
    export(&app.user("bob").await).await.assert_status(200);
}

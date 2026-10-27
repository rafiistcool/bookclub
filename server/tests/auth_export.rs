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
                "image/webp",
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
            source: "upload",
            content_type: "image/webp",
            variants,
            width: 100,
            height: 150,
            byte_size: 8,
            sha256: [seed; 32],
            dominant_color: None,
            uploaded_by: Some(uploader),
            created_at: None,
        },
    )
    .await
    .unwrap();
    public_id
}

/// One row for Ada behind every foreign-key column of [`SOURCES`]; each section of her export
/// holds it, and Bob's matching rows stay out.
#[sqlx::test]
async fn every_section_holds_the_users_rows(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cleo = app.user("cleo").await;
    let (a, b) = (ada.id(), bob.id());
    let readers = app.club(&ada, "Readers").await;
    app.join(&readers, &bob, "member").await;
    let old = app.club(&bob, "Old Club").await;
    app.join(&old, &cleo, "member").await;
    let work = app.work("Circe").await;
    let other = app.work("Ulysses").await;
    let edition = app.edition(&work, "9780316556347").await;
    let avatar = upload(&app, a, "avatar", 1).await;
    let cover = upload(&app, a, "cover", 2).await;
    upload(&app, b, "avatar", 3).await;
    ada.put(
        "/api/auth/me/email",
        json!({"email": "next@example.test", "current_password": PASSWORD}),
    )
    .await
    .assert_status(204);
    let (c, o, w, w2) = (readers.id, old.id, work.id, other.id);
    app.exec(&format!(
        "UPDATE users SET avatar_media_id = (SELECT id FROM media WHERE public_id = '{avatar}')
             WHERE id = {a};
         UPDATE club_memberships SET invited_by = {a} WHERE club_id = {c} AND user_id = {b};
         INSERT INTO club_removals (club_id, user_id) VALUES ({o}, {a});
         INSERT INTO club_invites (club_id, code, created_by, max_uses) VALUES ({c}, 'ADAS', {a}, 5);
         INSERT INTO club_invites (club_id, code, created_by, max_uses) VALUES ({o}, 'BOBS', {b}, 5);
         INSERT INTO club_invite_uses (invite_id, user_id)
             SELECT id, {a} FROM club_invites WHERE code = 'BOBS';
         INSERT INTO club_invite_uses (invite_id, user_id)
             SELECT id, {cleo} FROM club_invites WHERE code = 'BOBS';
         INSERT INTO club_picks (club_id, work_id, set_by, note) VALUES ({c}, {w}, {a}, 'our pick');
         INSERT INTO club_picks (club_id, work_id, set_by, note) VALUES ({o}, {w}, {b}, 'not hers');
         INSERT INTO milestones (club_id, pick_id, title, created_by)
             SELECT {c}, id, 'Part one', {a} FROM club_picks WHERE club_id = {c};
         INSERT INTO next_up_votes (club_id) VALUES ({c});
         INSERT INTO next_up_nominations (club_id, vote_id, work_id, nominated_by)
             SELECT {c}, id, {w2}, {a} FROM next_up_votes;
         INSERT INTO next_up_ballots (club_id, vote_id, user_id, nomination_id)
             SELECT {c}, vote_id, {a}, id FROM next_up_nominations;
         INSERT INTO events (club_id, actor_id, kind) VALUES ({c}, {a}, 'member_joined');
         INSERT INTO events (club_id, actor_id, kind) VALUES ({c}, {b}, 'member_joined');
         INSERT INTO import_jobs (user_id, status, total) VALUES ({a}, 'done', 3);
         UPDATE works SET created_by = {a} WHERE id = {w};
         UPDATE editions SET created_by = {a} WHERE id = {e};
         INSERT INTO catalog_edit_requests (kind, work_id, note, created_by)
             VALUES ('work_edit', {w}, 'a typo', {a});
         INSERT INTO catalog_edit_requests (kind, work_id, status, created_by, decided_by, decided_at, decision_note)
             VALUES ('work_edit', {w}, 'approved', {b}, {a}, now(), 'looks right');
         INSERT INTO reports (target_kind, work_id, reason, details, created_by)
             VALUES ('cover', {w}, 'wrong_book', 'not this one', {a});
         INSERT INTO reports (target_kind, work_id, reason, status, created_by, resolved_by, resolved_at)
             VALUES ('cover', {w}, 'other', 'dismissed', {b}, {a}, now());
         INSERT INTO shelf_entries (user_id, work_id, status, rating, take)
             VALUES ({a}, {w}, 'finished', 5, 'loved it'), ({b}, {w}, 'finished', 1, 'bobs take');
         INSERT INTO physical_shelf_layouts (user_id, layout) VALUES ({a}, '{{\"rows\": [[1]]}}');
         INSERT INTO user_favorites (user_id, work_id, position) VALUES ({a}, {w}, 1);
         INSERT INTO search_history (user_id, query, normalized_query) VALUES ({a}, 'Circe', 'circe');
         INSERT INTO quotes (user_id, work_id, body, page) VALUES ({a}, {w}, 'A line', 12),
             ({b}, {w}, 'bobs line', 3);
         INSERT INTO posts (work_id, author_id, body) VALUES ({w}, {a}, 'my diary'), ({w}, {b}, 'bobs diary');
         INSERT INTO posts (work_id, author_id, club_id, pick_id, body)
             SELECT {w}, {a}, {c}, id, 'club talk' FROM club_picks WHERE club_id = {c};
         INSERT INTO post_reactions (post_id, user_id, emoji)
             SELECT id, {a}, '🔥' FROM posts WHERE body = 'bobs diary';",
        cleo = cleo.id(),
        e = edition.id,
    ))
    .await;

    let res = export(&ada).await;
    res.assert_status(200);
    let data = data_json(&res);
    assert_utc_timestamps(&data, "data");
    let rows = |key: &str| {
        data[key]
            .as_array()
            .unwrap_or_else(|| panic!("{key}"))
            .clone()
    };
    let one = |key: &str| {
        let rows = rows(key);
        assert_eq!(rows.len(), 1, "{key}: {rows:?}");
        rows[0].clone()
    };

    assert_eq!(data["profile"]["pending_email"], "next@example.test");
    assert_eq!(one("sessions")["label"], "test");
    let shelf = one("shelf_entries");
    assert_eq!(shelf["work_title"], "Circe");
    assert_eq!(shelf["take"], "loved it");
    assert!(shelf.get("user_id").is_none());
    assert_eq!(
        data["physical_shelf_layout"]["layout"],
        json!({"rows": [[1]]})
    );
    assert_eq!(one("favorites")["work_title"], "Circe");
    assert_eq!(one("search_history")["query"], "Circe");
    assert_eq!(one("quotes")["body"], "A line");
    let posts = rows("posts");
    let bodies: Vec<&str> = posts.iter().map(|p| p["body"].as_str().unwrap()).collect();
    assert_eq!(bodies, ["my diary", "club talk"]);
    assert_eq!(posts[0]["club_name"], Value::Null);
    assert_eq!(posts[1]["club_name"], "Readers");
    assert_eq!(one("reactions")["emoji"], "🔥");
    let membership = one("memberships");
    assert_eq!(membership["club_name"], "Readers");
    assert_eq!(membership["role"], "owner");
    let invited = one("members_invited");
    assert_eq!(invited["username"], "bob");
    assert_eq!(one("club_removals")["club_name"], "Old Club");
    assert_eq!(one("clubs_created")["name"], "Readers");
    assert_eq!(one("invites_created")["code"], "ADAS");
    let used = one("invite_uses");
    assert_eq!(used["club_name"], "Old Club");
    assert!(used.get("code").is_none(), "another user's invite code");
    assert_eq!(one("picks_set")["note"], "our pick");
    assert_eq!(one("milestones_created")["title"], "Part one");
    assert_eq!(one("nominations")["work_title"], "Ulysses");
    assert_eq!(one("ballots")["work_title"], "Ulysses");
    let event = one("events");
    assert_eq!(event["kind"], "member_joined");
    assert_eq!(event["club_name"], "Readers");
    assert_eq!(one("import_jobs")["total"], 3);
    let contributions = &data["catalog_contributions"];
    assert_eq!(contributions["works"][0]["title"], "Circe");
    assert_eq!(contributions["editions"][0]["isbn13"], "9780316556347");
    assert_eq!(data["edit_requests"]["filed"][0]["note"], "a typo");
    assert_eq!(
        data["edit_requests"]["decided"][0]["decision_note"],
        "looks right"
    );
    assert_eq!(data["reports"]["filed"][0]["details"], "not this one");
    assert_eq!(data["reports"]["resolved"][0]["status"], "dismissed");
    for (key, size) in [
        ("edit_requests", "filed"),
        ("edit_requests", "decided"),
        ("reports", "filed"),
        ("reports", "resolved"),
    ] {
        assert_eq!(data[key][size].as_array().unwrap().len(), 1, "{key}.{size}");
    }

    let media = rows("uploaded_media");
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

    // Nothing of Bob's leaks into Ada's export, and the other way round.
    let text = serde_json::to_string(&data).unwrap();
    for theirs in ["bobs take", "bobs line", "bobs diary", "not hers"] {
        assert!(!text.contains(theirs), "{theirs}");
    }
    let bobs = data_json(&export(&bob).await);
    assert_eq!(bobs["shelf_entries"][0]["take"], "bobs take");
    assert_eq!(bobs["uploaded_media"].as_array().unwrap().len(), 1);
    assert_eq!(bobs["memberships"].as_array().unwrap().len(), 2);
    assert!(!serde_json::to_string(&bobs).unwrap().contains("loved it"));
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

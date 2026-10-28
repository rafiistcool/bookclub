//! Database helpers outside the request path: `admin::make_admin` (the CLI), the frozen `dto`
//! loaders (docs/phase1/conventions.md §20), `clubs::extract::club_refs` (§5.3),
//! `media::shared::register`/`is_removed_hash` (§8) and `activity::record` (§15).

use std::panic::AssertUnwindSafe;

use bookclub_server::activity::{self, EventKind, NewEvent};
use bookclub_server::clubs::extract::club_refs;
use bookclub_server::media::shared::{self as media, NewMedia, urls};
use bookclub_server::{admin, dto};
use futures::FutureExt;
use serde_json::{Map, Value, json};
use sqlx::PgPool;

async fn insert_user(db: &PgPool, username: &str) -> i64 {
    sqlx::query_scalar("INSERT INTO users (username, password_hash) VALUES ($1, 'x') RETURNING id")
        .bind(username)
        .fetch_one(db)
        .await
        .unwrap()
}

/// Gives the user an avatar row with this public id, taken down or not.
async fn set_avatar(db: &PgPool, user_id: i64, public_id: &str, removed: bool) {
    sqlx::query(
        "WITH avatar AS (
             INSERT INTO media (public_id, kind, source, uploaded_by, removed_at)
             VALUES ($2, 'avatar', 'upload', $1, CASE WHEN $3 THEN now() END)
             RETURNING id
         )
         UPDATE users SET avatar_media_id = (SELECT id FROM avatar) WHERE id = $1",
    )
    .bind(user_id)
    .bind(public_id)
    .bind(removed)
    .execute(db)
    .await
    .unwrap();
}

#[sqlx::test]
async fn make_admin_promotes_exactly_the_named_user(db: PgPool) {
    insert_user(&db, "ada").await;
    insert_user(&db, "bob").await;
    let mut conn = db.acquire().await.unwrap();

    assert!(admin::make_admin(&mut conn, "  Ada ").await.unwrap());
    assert!(!admin::make_admin(&mut conn, "nobody").await.unwrap());

    let admins: Vec<String> = sqlx::query_scalar("SELECT username FROM users WHERE is_admin")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(admins, ["ada"]);
}

#[sqlx::test]
async fn user_summaries_leave_out_removed_avatars(db: PgPool) {
    let ada = insert_user(&db, "ada").await;
    let bob = insert_user(&db, "bob").await;
    let cy = insert_user(&db, "cy").await;
    set_avatar(&db, ada, "aaaaaaaaaaaaaaaaaaaaaaaaaa", false).await;
    set_avatar(&db, bob, "bbbbbbbbbbbbbbbbbbbbbbbbbb", true).await;
    let mut conn = db.acquire().await.unwrap();

    let missing = cy + 1000;
    let summaries = dto::user_summaries(&mut conn, &[ada, bob, cy, missing])
        .await
        .unwrap();

    assert_eq!(summaries.len(), 3);
    assert!(!summaries.contains_key(&missing));
    assert_eq!(summaries[&ada].username, "ada");
    assert_eq!(
        summaries[&ada].avatar_url.as_deref(),
        Some("/media/aaaaaaaaaaaaaaaaaaaaaaaaaa/a")
    );
    assert_eq!(summaries[&bob].username, "bob");
    assert_eq!(summaries[&bob].avatar_url, None);
    assert_eq!(summaries[&cy].avatar_url, None);
}

/// A club owned by `owner`, created with its owner membership in one statement.
async fn insert_club(db: &PgPool, owner: i64, slug: &str) -> i64 {
    sqlx::query_scalar(
        "WITH club AS (INSERT INTO clubs (slug, name) VALUES ($2, 'Club') RETURNING id)
         INSERT INTO club_memberships (club_id, user_id, role)
         SELECT id, $1, 'owner' FROM club RETURNING club_id",
    )
    .bind(owner)
    .bind(slug)
    .fetch_one(db)
    .await
    .unwrap()
}

#[sqlx::test]
async fn club_refs_name_each_existing_club(db: PgPool) {
    let ada = insert_user(&db, "ada").await;
    let a = insert_club(&db, ada, "first-abcd").await;
    let b = insert_club(&db, ada, "second-efgh").await;
    sqlx::query("UPDATE clubs SET name = 'Second' WHERE id = $1")
        .bind(b)
        .execute(&db)
        .await
        .unwrap();
    let mut conn = db.acquire().await.unwrap();

    let refs = club_refs(&mut conn, &[b, a, b + 1000]).await.unwrap();
    let mut pairs: Vec<(i64, &str, &str)> = refs
        .iter()
        .map(|(id, r)| (*id, r.slug.as_str(), r.name.as_str()))
        .collect();
    pairs.sort();
    assert_eq!(
        pairs,
        [(a, "first-abcd", "Club"), (b, "second-efgh", "Second")]
    );
    assert!(club_refs(&mut conn, &[]).await.unwrap().is_empty());
}

fn payload(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[sqlx::test]
async fn activity_record_writes_both_scopes_and_drops_nulls(db: PgPool) {
    let ada = insert_user(&db, "ada").await;
    let club = insert_club(&db, ada, "readers-k3x9").await;
    let mut conn = db.acquire().await.unwrap();

    let joined = activity::record(
        &mut conn,
        NewEvent {
            club_id: Some(club),
            actor_id: Some(ada),
            kind: EventKind::MemberJoined,
            work_id: None,
            pick_id: None,
            payload: Map::new(),
        },
    )
    .await
    .unwrap();
    let quote = activity::record(
        &mut conn,
        NewEvent {
            club_id: None,
            actor_id: Some(ada),
            kind: EventKind::QuoteAdded,
            work_id: None,
            pick_id: None,
            payload: payload(json!({"quote_id": 7, "page": null, "excerpt": "Hi"})),
        },
    )
    .await
    .unwrap();
    assert!(quote > joined);
    let rows: Vec<(Option<i64>, String, Value)> =
        sqlx::query_as("SELECT club_id, kind, payload FROM events ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        rows,
        [
            (Some(club), "member_joined".to_owned(), json!({})),
            (
                None,
                "quote_added".to_owned(),
                json!({"quote_id": 7, "excerpt": "Hi"})
            ),
        ]
    );
}

#[sqlx::test]
async fn activity_record_stores_every_kind_the_schema_allows(db: PgPool) {
    let ada = insert_user(&db, "ada").await;
    let club = insert_club(&db, ada, "readers-k3x9").await;
    let mut conn = db.acquire().await.unwrap();
    for kind in EventKind::ALL {
        // The events.kind and events_scope CHECKs reject a misspelled or mis-scoped kind.
        let club_id = (!kind.is_personal()).then_some(club);
        activity::record(
            &mut conn,
            NewEvent {
                club_id,
                actor_id: Some(ada),
                kind,
                work_id: None,
                pick_id: None,
                payload: Map::new(),
            },
        )
        .await
        .unwrap_or_else(|e| panic!("{kind:?}: {e}"));
    }
    let kinds: Vec<String> = sqlx::query_scalar("SELECT kind FROM events ORDER BY id")
        .fetch_all(&db)
        .await
        .unwrap();
    let expected: Vec<&str> = EventKind::ALL.iter().map(|k| k.as_str()).collect();
    assert_eq!(kinds, expected);
}

#[sqlx::test]
async fn activity_record_asserts_the_scope_of_the_kind(db: PgPool) {
    let ada = insert_user(&db, "ada").await;
    let club = insert_club(&db, ada, "readers-k3x9").await;
    let mut conn = db.acquire().await.unwrap();
    for (club_id, kind) in [
        (Some(club), EventKind::ShelfAdded),
        (None, EventKind::PickSet),
    ] {
        let event = NewEvent {
            club_id,
            actor_id: Some(ada),
            kind,
            work_id: None,
            pick_id: None,
            payload: Map::new(),
        };
        let outcome = AssertUnwindSafe(activity::record(&mut conn, event))
            .catch_unwind()
            .await;
        assert!(
            outcome.is_err(),
            "{kind:?} with club_id {club_id:?} must panic"
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

/// The media row's columns that `register` sets, apart from the storage key, the bytes'
/// metadata and created_at, as JSON.
async fn media_row(db: &PgPool, id: i64) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object('public_id', public_id, 'kind', kind, 'source', source,
                'width', width, 'height', height, 'byte_size', byte_size,
                'dominant_color', dominant_color, 'uploaded_by', uploaded_by,
                'removed_at', removed_at, 'ol_cover_id', ol_cover_id)
         FROM media WHERE id = $1",
    )
    .bind(id)
    .fetch_one(db)
    .await
    .unwrap()
}

#[sqlx::test]
async fn registered_media_point_at_their_storage_prefix(db: PgPool) {
    let ada = insert_user(&db, "ada").await;
    let mut conn = db.acquire().await.unwrap();
    let public_id = urls::new_public_id();
    let hash = [7u8; 32];
    let created = time::macros::datetime!(2024-05-01 10:00 UTC);
    let id = media::register(
        &mut conn,
        &public_id,
        &NewMedia {
            kind: "avatar",
            source: "legacy",
            content_type: "image/jpeg",
            variants: &["a"],
            width: 256,
            height: 256,
            byte_size: 1234,
            sha256: hash,
            dominant_color: None,
            uploaded_by: Some(ada),
            created_at: Some(created),
        },
    )
    .await
    .unwrap();
    let row: (String, String, Vec<String>, Vec<u8>, time::OffsetDateTime) = sqlx::query_as(
        "SELECT storage_key, content_type, variants, sha256, created_at FROM media WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(row.0, urls::storage_prefix("avatar", &public_id));
    assert!(row.0.starts_with(&format!("avatars/{}/", &public_id[..2])));
    assert_eq!(row.1, "image/jpeg");
    assert_eq!(row.2, ["a"]);
    assert_eq!(row.3, hash);
    assert_eq!(row.4, created);
    assert_eq!(
        media_row(&db, id).await,
        json!({"public_id": public_id, "kind": "avatar", "source": "legacy", "width": 256,
               "height": 256, "byte_size": 1234, "dominant_color": null, "uploaded_by": ada,
               "removed_at": null, "ol_cover_id": null})
    );

    // A cover without created_at gets now(); its hash is not removed until a takedown.
    let cover_public_id = urls::new_public_id();
    let cover = media::register(
        &mut conn,
        &cover_public_id,
        &NewMedia {
            kind: "cover",
            source: "upload",
            content_type: "image/webp",
            variants: &["s", "l"],
            width: 800,
            height: 1200,
            byte_size: 50_000,
            sha256: [9u8; 32],
            dominant_color: Some("#112233".into()),
            uploaded_by: Some(ada),
            created_at: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        media_row(&db, cover).await,
        json!({"public_id": cover_public_id, "kind": "cover",
               "source": "upload", "width": 800, "height": 1200, "byte_size": 50_000,
               "dominant_color": "#112233", "uploaded_by": ada, "removed_at": null,
               "ol_cover_id": null})
    );
    let age: f64 = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM now() - created_at)::float8 FROM media WHERE id = $1",
    )
    .bind(cover)
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(
        (0.0..60.0).contains(&age),
        "created_at None = now(), {age}s ago"
    );
    assert!(!media::is_removed_hash(&mut conn, &[9u8; 32]).await.unwrap());
    sqlx::query("UPDATE media SET removed_at = now() WHERE id = $1")
        .bind(cover)
        .execute(&db)
        .await
        .unwrap();
    assert!(media::is_removed_hash(&mut conn, &[9u8; 32]).await.unwrap());
    assert!(!media::is_removed_hash(&mut conn, &hash).await.unwrap());
}

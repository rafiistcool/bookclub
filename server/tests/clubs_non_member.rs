//! Route-wide guard (docs/phase1/conventions.md §5.3, §17.7): club data is for club members only.
//!
//! - Every `/api/clubs/{slug}…` operation answers a signed-in non-member 404 `No such club`,
//!   exactly as for a club that does not exist, and changes nothing.
//! - Club rows reached by a global id or a query parameter stay hidden too: the
//!   `/api/posts/{post_id}…` operations, a diary reply to a club post, `GET /api/diary?club=`,
//!   and the feeds that mix clubs.
//!
//! Generic: the operations come from the OpenAPI document, so Wave 2 routes are covered as soon
//! as they merge. Until POSTS and CLUB merge, their routes are absent and those checks are
//! skipped (the policy functions behind them are checked directly).

mod common;

use axum::http::Method;
use common::routes::{Operation, find, operations, outcome};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// Club operations of Wave 1 that must be in the walk, so it can never pass vacuously.
const WAVE1_CLUB_OPS: [&str; 11] = [
    "GET /api/clubs/{slug}",
    "PATCH /api/clubs/{slug}",
    "DELETE /api/clubs/{slug}",
    "GET /api/clubs/{slug}/members",
    "PATCH /api/clubs/{slug}/members/{username}",
    "DELETE /api/clubs/{slug}/members/{username}",
    "POST /api/clubs/{slug}/leave",
    "POST /api/clubs/{slug}/transfer",
    "GET /api/clubs/{slug}/invites",
    "POST /api/clubs/{slug}/invites",
    "DELETE /api/clubs/{slug}/invites/{invite_id}",
];

/// A body every club operation would act on if a guard were missing (unknown keys are ignored).
fn takeover(caller: &TestClient) -> Value {
    json!({
        "name": "Taken over",
        "timezone": "Europe/Berlin",
        "role": "admin",
        "username": caller.username(),
        "kind": "link",
    })
}

/// What a club looks like from the database, to show that a refused request changed nothing.
async fn snapshot(app: &TestApp, club: &TestClub) -> Vec<i64> {
    let mut out = Vec::new();
    for sql in [
        format!(
            "SELECT count(*) FROM clubs WHERE id = {} AND name = '{}' AND timezone = 'UTC'",
            club.id, club.name
        ),
        format!(
            "SELECT count(*) * 10 + count(*) FILTER (WHERE role = 'owner') FROM club_memberships
             WHERE club_id = {}",
            club.id
        ),
        format!(
            "SELECT count(*) * 10 + count(*) FILTER (WHERE revoked_at IS NULL) FROM club_invites
             WHERE club_id = {}",
            club.id
        ),
        format!(
            "SELECT count(*) FROM club_removals WHERE club_id = {}",
            club.id
        ),
    ] {
        out.push(app.scalar_i64(&sql).await);
    }
    out
}

#[sqlx::test]
async fn non_members_get_no_such_club_on_every_club_route(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let member = app.user("member").await;
    let club = app.club(&owner, "Club A").await;
    app.join(&club, &member, "member").await;
    let invite = owner
        .post(
            &format!("/api/clubs/{}/invites", club.slug),
            json!({"kind": "link"}),
        )
        .await;
    invite.assert_status(201);
    let invite_id = invite.json()["id"].as_i64().unwrap();
    // A member of another club, and a user in no club at all.
    let outsider = app.user("outsider").await;
    app.club(&outsider, "Club B").await;
    let clubless = app.user("clubless").await;
    let before = snapshot(&app, &club).await;

    let club_ops: Vec<Operation> = operations()
        .into_iter()
        .filter(|op| op.path == "/api/clubs/{slug}" || op.path.starts_with("/api/clubs/{slug}/"))
        .collect();
    for label in WAVE1_CLUB_OPS {
        assert!(
            club_ops.iter().any(|op| op.label() == label),
            "{label} is not in the document"
        );
    }
    for op in &club_ops {
        for caller in [&outsider, &clubless] {
            for slug in [club.slug.as_str(), "no-such-club-zzzz"] {
                let path = op.fill(|name, integer| match (name, integer) {
                    ("slug", _) => slug.to_owned(),
                    ("username", _) => "member".to_owned(),
                    ("invite_id", _) => invite_id.to_string(),
                    (_, true) => "1".to_owned(),
                    (_, false) => "x".to_owned(),
                });
                let res = op.send(caller, &path, &takeover(caller)).await;
                assert_eq!(
                    outcome(&res),
                    (404, json!({"detail": "No such club"})),
                    "{} as {} on {slug}",
                    op.label(),
                    caller.username()
                );
            }
        }
    }
    assert_eq!(snapshot(&app, &club).await, before);
}

/// Club A with a pick, a club post, a member's reply to it, and two club events.
struct ClubA {
    club: TestClub,
    member: TestClient,
    work: TestWork,
    post: i64,
    reply: i64,
    events: [i64; 2],
}

async fn seed_club_a(app: &TestApp) -> ClubA {
    let owner = app.user("owner").await;
    let member = app.user("member").await;
    let club = app.club(&owner, "Club A").await;
    app.join(&club, &member, "member").await;
    let work = app.work("Circe").await;
    let pick: i64 = sqlx::query_scalar(
        "INSERT INTO club_picks (club_id, work_id, set_by) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(club.id)
    .bind(work.id)
    .bind(owner.id())
    .fetch_one(&app.db)
    .await
    .unwrap();
    let insert_post = "INSERT INTO posts (work_id, author_id, club_id, pick_id, parent_id, body)
                       VALUES ($1, $2, $3, $4, $5, $6) RETURNING id";
    let post: i64 = sqlx::query_scalar(insert_post)
        .bind(work.id)
        .bind(owner.id())
        .bind(club.id)
        .bind(pick)
        .bind(None::<i64>)
        .bind("Only for Club A")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let reply: i64 = sqlx::query_scalar(insert_post)
        .bind(work.id)
        .bind(member.id())
        .bind(club.id)
        .bind(pick)
        .bind(post)
        .bind("A reply in Club A")
        .fetch_one(&app.db)
        .await
        .unwrap();
    let mut events = [0; 2];
    for (slot, (kind, payload)) in events.iter_mut().zip([
        ("pick_set", json!({})),
        ("note_posted", json!({"post_id": post})),
    ]) {
        *slot = sqlx::query_scalar(
            "INSERT INTO events (club_id, actor_id, kind, work_id, pick_id, payload)
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        )
        .bind(club.id)
        .bind(owner.id())
        .bind(kind)
        .bind(work.id)
        .bind(pick)
        .bind(payload)
        .fetch_one(&app.db)
        .await
        .unwrap();
    }
    ClubA {
        club,
        member,
        work,
        post,
        reply,
        events,
    }
}

/// The post ids in a work thread (`items` and their `replies`).
fn thread_ids(body: &Value) -> Vec<i64> {
    let mut ids = Vec::new();
    for item in body["items"].as_array().expect("items") {
        ids.extend(item["id"].as_i64());
        for reply in item["replies"].as_array().into_iter().flatten() {
            ids.extend(reply["id"].as_i64());
        }
    }
    ids
}

/// The ids of `items[*]` (`field` = None) or of `items[*][field]`.
fn item_ids(body: &Value, field: Option<&str>) -> Vec<i64> {
    body["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|item| match field {
            Some(field) => item[field]["id"].as_i64(),
            None => item["id"].as_i64(),
        })
        .collect()
}

#[sqlx::test]
async fn club_rows_reached_by_id_or_query_stay_hidden(db: PgPool) {
    let app = TestApp::new(db).await;
    let a = seed_club_a(&app).await;
    let outsider = app.user("outsider").await;
    app.club(&outsider, "Club B").await;
    let clubless = app.user("clubless").await;
    let posts = [a.post, a.reply];

    // The policy the routes read through: members see A's posts, nobody else does.
    for post in posts {
        for (viewer, sees) in [(&a.member, true), (&outsider, false), (&clubless, false)] {
            let seen = app
                .scalar_i64(&format!(
                    "SELECT bc_can_see_post({}, {post})::int::int8",
                    viewer.id()
                ))
                .await;
            assert_eq!(seen == 1, sees, "{} on post {post}", viewer.username());
        }
    }

    // /api/posts/{post_id}… (POSTS): 404 for every post of club A, and nothing changes.
    let post_ops: Vec<Operation> = operations()
        .into_iter()
        .filter(|op| op.path.starts_with("/api/posts/{post_id}"))
        .collect();
    let body = json!({"body": "Edited by an outsider", "emoji": "\u{2764}\u{fe0f}"});
    for op in &post_ops {
        for caller in [&outsider, &clubless] {
            for post in posts {
                let path = op.fill(|name, _| match name {
                    "post_id" => post.to_string(),
                    _ => "1".to_owned(),
                });
                let res = op.send(caller, &path, &body).await;
                assert_eq!(
                    outcome(&res),
                    (404, json!({"detail": "No entry with that id"})),
                    "{} on post {post} as {}",
                    op.label(),
                    caller.username()
                );
            }
        }
    }
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM posts WHERE id IN ({}, {}) AND deleted_at IS NULL
               AND body IN ('Only for Club A', 'A reply in Club A')",
            a.post, a.reply
        ))
        .await,
        2
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM post_reactions").await,
        0
    );

    let thread = format!("/api/books/works/{}/posts", a.work.id);
    // A diary reply to a club post is refused like a reply to a missing post.
    if find(&Method::POST, "/api/books/works/{work_id}/posts").is_some() {
        for caller in [&outsider, &clubless] {
            caller
                .post(&thread, json!({"body": "Sneaking in", "parent_id": a.post}))
                .await
                .assert_detail(400, "That entry is not on this book");
        }
    }
    // The work thread mixes the viewer's clubs: A's posts only for A's members.
    if find(&Method::GET, "/api/books/works/{work_id}/posts").is_some() {
        let seen = thread_ids(&a.member.get(&thread).await.json());
        assert!(posts.iter().all(|post| seen.contains(post)), "{seen:?}");
        for caller in [&outsider, &clubless] {
            let seen = thread_ids(&caller.get(&thread).await.json());
            assert!(posts.iter().all(|post| !seen.contains(post)), "{seen:?}");
        }
    }
    if find(&Method::GET, "/api/diary").is_some() {
        let seen = item_ids(&a.member.get("/api/diary").await.json(), Some("entry"));
        assert!(seen.contains(&a.post), "{seen:?}");
        for caller in [&outsider, &clubless] {
            caller
                .get(&format!("/api/diary?club={}", a.club.slug))
                .await
                .assert_detail(404, "No such club");
            let seen = item_ids(&caller.get("/api/diary").await.json(), Some("entry"));
            assert!(posts.iter().all(|post| !seen.contains(post)), "{seen:?}");
        }
    }
    if find(&Method::GET, "/api/activity").is_some() {
        let seen = item_ids(&a.member.get("/api/activity").await.json(), None);
        assert!(a.events.iter().all(|id| seen.contains(id)), "{seen:?}");
        for caller in [&outsider, &clubless] {
            let seen = item_ids(&caller.get("/api/activity").await.json(), None);
            assert!(a.events.iter().all(|id| !seen.contains(id)), "{seen:?}");
        }
    }
}

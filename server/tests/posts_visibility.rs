//! Who sees which posts (#46, conventions §6.3 and §6.5), across the viewer types of
//! `common::visibility::Viewers`: diary roots by visibility, replies by people outside the
//! viewer's audience, reaction names, `parent_author`, club posts across clubs, reacting to an
//! invisible post (404 without the body), moderation that stops at the post's own club, the
//! live `author_rating` with `only_me` ratings, and the tombstone of a deleted account.

mod common;

use common::visibility::Viewers;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const HEART: &str = "\u{2764}\u{fe0f}";

fn thread(work: &TestWork) -> String {
    format!("/api/books/works/{}/posts", work.id)
}

async fn write(client: &TestClient, work: &TestWork, body: Value) -> Value {
    let res = client.post(&thread(work), body).await;
    res.assert_status(201);
    res.json()
}

/// Every post body in a work thread, replies included, in order.
async fn thread_bodies(viewer: &TestClient, work: &TestWork) -> Vec<String> {
    let res = viewer.get(&thread(work)).await;
    res.assert_status(200);
    let mut out = Vec::new();
    for item in res.json()["items"].as_array().unwrap() {
        out.push(item["body"].as_str().unwrap().to_owned());
        for reply in item["replies"].as_array().unwrap() {
            out.push(reply["body"].as_str().unwrap().to_owned());
        }
    }
    out
}

async fn feed_bodies(viewer: &TestClient) -> Vec<String> {
    let res = viewer.get("/api/diary?limit=50").await;
    res.assert_status(200);
    res.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["entry"]["body"].as_str().unwrap().to_owned())
        .collect()
}

async fn set_pick(app: &TestApp, club: &TestClub, work: &TestWork) -> i64 {
    sqlx::query_scalar("INSERT INTO club_picks (club_id, work_id) VALUES ($1, $2) RETURNING id")
        .bind(club.id)
        .bind(work.id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn club_post(client: &TestClient, club: &TestClub, body: &str) -> Value {
    let res = client
        .post(
            &format!("/api/clubs/{}/pick/posts", club.slug),
            json!({ "body": body }),
        )
        .await;
    res.assert_status(201);
    res.json()
}

/// A 404 `No entry with that id` that carries nothing else.
fn assert_no_entry(res: &TestResponse, who: &str) {
    assert_eq!(res.status.as_u16(), 404, "{who}: {}", res.text());
    assert_eq!(
        res.json(),
        json!({"detail": "No entry with that id"}),
        "{who}"
    );
}

#[sqlx::test]
async fn diary_roots_follow_their_visibility(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    write(&v.owner, &circe, json!({"body": "Open"})).await;
    let private = write(
        &v.owner,
        &circe,
        json!({"body": "Private", "visibility": "only_me"}),
    )
    .await;
    assert_eq!(private["visibility"], "only_me");

    assert_eq!(thread_bodies(&v.owner, &circe).await, ["Open", "Private"]);
    assert_eq!(feed_bodies(&v.owner).await, ["Private", "Open"]);
    for viewer in v.audience() {
        assert_eq!(thread_bodies(viewer, &circe).await, ["Open"]);
        assert_eq!(feed_bodies(viewer).await, ["Open"]);
    }
    for viewer in v.strangers() {
        assert!(thread_bodies(viewer, &circe).await.is_empty());
        assert!(feed_bodies(viewer).await.is_empty());
    }
    // Hidden entries cannot be replied to either; the answer is that of a missing parent.
    for viewer in v.audience().into_iter().chain(v.strangers()) {
        viewer
            .post(
                &thread(&circe),
                json!({"body": "Hi", "parent_id": private["id"]}),
            )
            .await
            .assert_detail(400, "That entry is not on this book");
    }
}

#[sqlx::test]
async fn replies_reach_only_their_authors_audience(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    // `sider` shares a club with the owner, but not with `comember`.
    let sider = app.user("sider").await;
    let side = app.club(&v.owner, "Side").await;
    app.join(&side, &sider, "member").await;
    let circe = app.work("Circe").await;
    let root = write(&v.owner, &circe, json!({"body": "Root"})).await;
    write(
        &sider,
        &circe,
        json!({"body": "From the side", "parent_id": root["id"]}),
    )
    .await;
    write(
        &v.co_member,
        &circe,
        json!({"body": "From the club", "parent_id": root["id"]}),
    )
    .await;

    // The root's author sees every reply; everyone else only the replies of their audience.
    assert_eq!(
        thread_bodies(&v.owner, &circe).await,
        ["Root", "From the side", "From the club"]
    );
    assert_eq!(
        thread_bodies(&v.co_member, &circe).await,
        ["Root", "From the club"]
    );
    assert_eq!(
        thread_bodies(&sider, &circe).await,
        ["Root", "From the side"]
    );
    assert_eq!(feed_bodies(&v.co_member).await, ["From the club", "Root"]);
    for viewer in v.strangers() {
        assert!(thread_bodies(viewer, &circe).await.is_empty());
        assert!(feed_bodies(viewer).await.is_empty());
        viewer
            .post(
                &thread(&circe),
                json!({"body": "Sneaking in", "parent_id": root["id"]}),
            )
            .await
            .assert_detail(400, "That entry is not on this book");
    }
    // A reply someone cannot see answers like a missing post.
    let hidden = app
        .scalar_i64("SELECT id FROM posts WHERE body = 'From the side'")
        .await;
    assert_no_entry(
        &v.co_member
            .post(
                &format!("/api/posts/{hidden}/reactions"),
                json!({"emoji": HEART}),
            )
            .await,
        "comember",
    );
}

#[sqlx::test]
async fn reaction_names_stay_inside_the_audience(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let sider = app.user("sider").await;
    let side = app.club(&v.owner, "Side").await;
    app.join(&side, &sider, "member").await;
    let circe = app.work("Circe").await;
    let root = write(&v.owner, &circe, json!({"body": "Root"})).await;
    let url = format!("/api/posts/{}/reactions", root["id"]);
    sider
        .post(&url, json!({"emoji": HEART}))
        .await
        .assert_status(200);
    let seen = v.co_member.post(&url, json!({"emoji": HEART})).await.json();
    assert_eq!(
        seen["reactions"],
        json!([{"emoji": HEART, "count": 2, "mine": true, "users": ["comember"]}])
    );
    let owners = v.owner.get(&thread(&circe)).await.json();
    assert_eq!(
        owners["items"][0]["reactions"],
        json!([{"emoji": HEART, "count": 2, "mine": false, "users": ["comember", "sider"]}])
    );
    let siders = sider.get(&thread(&circe)).await.json();
    assert_eq!(
        siders["items"][0]["reactions"],
        json!([{"emoji": HEART, "count": 2, "mine": true, "users": ["sider"]}])
    );
}

#[sqlx::test]
async fn parent_author_shows_only_visible_authors(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    let root = write(&v.owner, &circe, json!({"body": "Root"})).await;
    write(
        &v.co_member,
        &circe,
        json!({"body": "Diary reply", "parent_id": root["id"]}),
    )
    .await;
    // A club post by a member who then leaves and shares no other club with `comember`.
    let leaver = app.user("leaver").await;
    app.join(&v.shared, &leaver, "member").await;
    set_pick(&app, &v.shared, &circe).await;
    let left = club_post(&leaver, &v.shared, "Before I go").await;
    v.co_member
        .post(
            &format!("/api/clubs/{}/pick/posts", v.shared.slug),
            json!({"body": "Club reply", "parent_id": left["id"]}),
        )
        .await
        .assert_status(201);
    app.exec(&format!(
        "DELETE FROM club_memberships WHERE club_id = {} AND user_id = {}",
        v.shared.id,
        leaver.id()
    ))
    .await;

    let feed = v.co_member.get("/api/diary").await.json();
    let parent_of = |body: &str| -> Value {
        feed["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["entry"]["body"] == body)
            .unwrap()["parent_author"]
            .clone()
    };
    assert_eq!(parent_of("Diary reply"), "owner");
    assert_eq!(parent_of("Club reply"), Value::Null);
    assert_eq!(parent_of("Root"), Value::Null);
    // The club's own thread still names the former member: club posts are club data.
    let club_thread = v
        .co_member
        .get(&format!("/api/clubs/{}/pick/posts", v.shared.slug))
        .await
        .json();
    assert_eq!(club_thread["items"][0]["author"], "leaver");
}

#[sqlx::test]
async fn club_posts_stay_inside_their_club(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    set_pick(&app, &v.shared, &circe).await;
    set_pick(&app, &v.other, &circe).await;
    let shared_post = club_post(&v.owner, &v.shared, "Shared only").await;
    club_post(&v.other_club, &v.other, "Other only").await;

    // `comember` is in both clubs and sees both, each with its club.
    let both = v.co_member.get(&thread(&circe)).await.json();
    let clubs: Vec<(&str, &str)> = both["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["body"].as_str().unwrap(),
                item["club"]["name"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(clubs, [("Shared only", "Shared"), ("Other only", "Other")]);
    assert_eq!(
        feed_bodies(&v.co_member).await,
        ["Other only", "Shared only"]
    );
    assert_eq!(thread_bodies(&v.owner, &circe).await, ["Shared only"]);
    assert_eq!(feed_bodies(&v.owner).await, ["Shared only"]);
    assert_eq!(thread_bodies(&v.other_club, &circe).await, ["Other only"]);
    assert_eq!(feed_bodies(&v.other_club).await, ["Other only"]);
    for viewer in [&v.stranger, &v.former] {
        assert!(thread_bodies(viewer, &circe).await.is_empty());
        assert!(feed_bodies(viewer).await.is_empty());
        viewer
            .get(&format!("/api/clubs/{}/pick/posts", v.shared.slug))
            .await
            .assert_detail(404, "No such club");
    }

    // Edits, deletes and reactions on another club's post are 404, never 403.
    let path = format!("/api/posts/{}", shared_post["id"]);
    for viewer in v.strangers() {
        let who = viewer.username();
        assert_no_entry(&viewer.patch(&path, json!({"body": "x"})).await, who);
        assert_no_entry(&viewer.delete(&path).await, who);
        assert_no_entry(
            &viewer
                .post(&format!("{path}/reactions"), json!({"emoji": HEART}))
                .await,
            who,
        );
        viewer
            .post(
                &thread(&circe),
                json!({"body": "x", "parent_id": shared_post["id"]}),
            )
            .await
            .assert_detail(400, "That entry is not on this book");
    }
    assert_eq!(app.scalar_i64("SELECT count(*) FROM posts").await, 2);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM post_reactions").await,
        0
    );
}

#[sqlx::test]
async fn reacting_to_an_invisible_post_is_404_without_the_body(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    let open = write(&v.owner, &circe, json!({"body": "Open secret"})).await;
    let private = write(
        &v.owner,
        &circe,
        json!({"body": "Private secret", "visibility": "only_me"}),
    )
    .await;
    let react = |post: &Value| format!("/api/posts/{}/reactions", post["id"]);
    for viewer in v.audience() {
        let res = viewer.post(&react(&private), json!({"emoji": HEART})).await;
        assert_no_entry(&res, viewer.username());
        assert!(!res.text().contains("secret"));
        assert_no_entry(
            &viewer
                .patch(
                    &format!("/api/posts/{}", private["id"]),
                    json!({"body": "x"}),
                )
                .await,
            viewer.username(),
        );
        viewer
            .post(&react(&open), json!({"emoji": HEART}))
            .await
            .assert_status(200);
    }
    for viewer in v.strangers() {
        for post in [&open, &private] {
            let res = viewer.post(&react(post), json!({"emoji": HEART})).await;
            assert_no_entry(&res, viewer.username());
            assert!(!res.text().contains("secret"));
            assert_no_entry(
                &viewer.delete(&format!("/api/posts/{}", post["id"])).await,
                viewer.username(),
            );
        }
    }
    // Only the audience's reaction on the open entry was stored.
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM post_reactions").await,
        1
    );
}

#[sqlx::test]
async fn club_admins_moderate_only_their_own_club(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    set_pick(&app, &v.shared, &circe).await;
    // `comember` administers Other but is a plain member of Shared.
    app.exec(&format!(
        "UPDATE club_memberships SET role = 'admin' WHERE club_id = {} AND user_id = {}",
        v.other.id,
        v.co_member.id()
    ))
    .await;
    let post = club_post(&v.owner, &v.shared, "Owner's post").await;
    let path = format!("/api/posts/{}", post["id"]);
    v.co_member
        .delete(&path)
        .await
        .assert_detail(403, "You can only delete your own entries");
    // The owner of club B cannot even see club A's post.
    assert_no_entry(&v.other_club.delete(&path).await, "otherclub");
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM posts WHERE id = {}",
            post["id"]
        ))
        .await,
        1
    );
    // The owner of the post's club moderates it.
    let members = club_post(&v.co_member, &v.shared, "Member's post").await;
    v.owner
        .delete(&format!("/api/posts/{}", members["id"]))
        .await
        .assert_status(204);
}

#[sqlx::test]
async fn author_rating_respects_only_me(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    let achilles = app.work("Achilles").await;
    for (work, visibility) in [(&circe, "only_me"), (&achilles, "audience")] {
        v.owner
            .post(
                "/api/shelf",
                json!({"work_id": work.id, "status": "finished", "rating": 5,
                       "rating_visibility": visibility}),
            )
            .await
            .assert_status(201);
        write(&v.owner, work, json!({"body": "Done"})).await;
    }
    let rating = |body: Value| body["items"][0]["author_rating"].clone();
    assert_eq!(rating(v.owner.get(&thread(&circe)).await.json()), 5);
    assert_eq!(
        rating(v.co_member.get(&thread(&circe)).await.json()),
        Value::Null
    );
    assert_eq!(rating(v.co_member.get(&thread(&achilles)).await.json()), 5);
    let feed = v.co_member.get("/api/diary").await.json();
    let ratings: Vec<Value> = feed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["entry"]["author_rating"].clone())
        .collect();
    assert_eq!(ratings, [json!(5), Value::Null]);
}

#[sqlx::test]
async fn a_deleted_accounts_tombstone_keeps_its_replies_visible(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    // The owner of `shared` cannot delete their account while it has members; a plain member
    // of it writes the entry instead.
    let writer = app.user("writer").await;
    app.join(&v.shared, &writer, "member").await;
    let circe = app.work("Circe").await;
    let root = write(&writer, &circe, json!({"body": "Root"})).await;
    write(&writer, &circe, json!({"body": "Nobody replied"})).await;
    write(
        &v.co_member,
        &circe,
        json!({"body": "Reply", "parent_id": root["id"]}),
    )
    .await;
    writer
        .delete_json("/api/auth/me", json!({"password": PASSWORD}))
        .await
        .assert_status(204);

    let seen = v.co_member.get(&thread(&circe)).await.json();
    let items = seen["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let ghost = &items[0];
    assert_eq!(ghost["author"], Value::Null);
    assert_eq!(ghost["author_avatar_url"], Value::Null);
    assert_eq!(ghost["deleted"], true);
    assert_eq!(ghost["body"], "");
    assert_eq!(ghost["mine"], false);
    assert_eq!(ghost["author_rating"], Value::Null);
    assert_eq!(ghost["replies"][0]["body"], "Reply");
    let feed = v.co_member.get("/api/diary").await.json();
    assert_eq!(feed["items"].as_array().unwrap().len(), 1);
    assert_eq!(feed["items"][0]["entry"]["body"], "Reply");
    assert_eq!(feed["items"][0]["parent_author"], Value::Null);
    // Each reply now reaches its own author's audience: `otherclub` shares a club with
    // `comember` (conventions §5.6).
    assert_eq!(thread_bodies(&v.other_club, &circe).await, ["", "Reply"]);
    for viewer in [&v.stranger, &v.former] {
        assert!(thread_bodies(viewer, &circe).await.is_empty());
    }
    assert_eq!(thread_bodies(&v.owner, &circe).await, ["", "Reply"]);
    let path = format!("/api/posts/{}", root["id"]);
    assert_no_entry(
        &v.co_member
            .post(&format!("{path}/reactions"), json!({"emoji": HEART}))
            .await,
        "comember",
    );
    v.co_member
        .post(
            &format!("/api/books/works/{}/posts", circe.id),
            json!({"body": "Late", "parent_id": root["id"]}),
        )
        .await
        .assert_detail(400, "That entry was deleted");
}

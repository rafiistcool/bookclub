//! Visibility of personal data in the book detail (#46, conventions §6.5): readers, the rating
//! mean and count and the quote count cover only the viewer's audience (plus the viewer), and
//! other users' `only_me` ratings and `only_me` quotes are left out. They read through
//! `bc_visible_shelf` and `bc_visible_quotes`.
//!
//! The viewer types of `common::visibility::Viewers`: `owner`, `comember` (shares `shared` with
//! the owner and `other` with `otherclub`), `otherclub`, `stranger`, and `former` (left
//! `shared`). The detail has no `?username=`, so there is no unknown-name case.

mod common;

use common::visibility::Viewers;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn shelve(app: &TestApp, who: &TestClient, work: i64, status: &str, extra: &str) {
    app.exec(&format!(
        "INSERT INTO shelf_entries (user_id, work_id, status) VALUES ({}, {work}, '{status}');
         UPDATE shelf_entries SET updated_at = now() {extra}
         WHERE user_id = {} AND work_id = {work};",
        who.id(),
        who.id()
    ))
    .await;
}

async fn quote(app: &TestApp, who: &TestClient, work: i64, visibility: &str) {
    app.exec(&format!(
        "INSERT INTO quotes (user_id, work_id, body, visibility)
         VALUES ({}, {work}, 'A line', '{visibility}')",
        who.id()
    ))
    .await;
}

/// Every viewer type's view of one book with an `only_me` rating and `only_me` quotes.
#[sqlx::test]
async fn book_detail_readers_ratings_and_quotes_follow_the_audience(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let work = app.work("Circe").await.id;
    shelve(
        &app,
        &v.owner,
        work,
        "finished",
        ", rating = 4, take = 'Mine', rating_visibility = 'only_me'",
    )
    .await;
    shelve(&app, &v.co_member, work, "finished", ", rating = 2").await;
    shelve(&app, &v.other_club, work, "finished", ", rating = 5").await;
    shelve(&app, &v.stranger, work, "finished", ", rating = 1").await;
    shelve(
        &app,
        &v.former,
        work,
        "currently_reading",
        ", progress = 10",
    )
    .await;
    quote(&app, &v.owner, work, "audience").await;
    quote(&app, &v.owner, work, "only_me").await;
    quote(&app, &v.co_member, work, "audience").await;
    quote(&app, &v.co_member, work, "only_me").await;
    quote(&app, &v.other_club, work, "audience").await;
    quote(&app, &v.stranger, work, "audience").await;
    quote(&app, &v.former, work, "audience").await;

    let path = format!("/api/books/works/{work}");
    let readers = |body: &Value| -> Vec<(String, Value)> {
        body["readers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["username"].as_str().unwrap().to_owned(),
                    r["rating"].clone(),
                )
            })
            .collect()
    };

    // The owner sees their own only_me rating and take, and their audience.
    let owner = v.owner.get(&path).await.json();
    assert_eq!(owner["rating"], 4);
    assert_eq!(owner["take"], "Mine");
    assert_eq!(owner["rating_visibility"], "only_me");
    assert_eq!(readers(&owner), [("comember".to_owned(), json!(2))]);
    assert_eq!(owner["club_rating"], 3.0);
    assert_eq!(owner["rating_count"], 2);
    assert_eq!(owner["quote_count"], 3);

    // The co-member sees the owner as a reader without the only_me rating, which also stays
    // out of the mean; the owner's only_me quote is not counted.
    let co = v.co_member.get(&path).await.json();
    assert_eq!(
        readers(&co),
        [
            ("otherclub".to_owned(), json!(5)),
            ("owner".to_owned(), Value::Null),
        ]
    );
    assert_eq!(co["readers"][1]["status"], "finished");
    assert_eq!(co["club_rating"], 3.5);
    assert_eq!(co["rating_count"], 2);
    assert_eq!(co["quote_count"], 4);

    // A member of another club sees only the shared member.
    let other = v.other_club.get(&path).await.json();
    assert_eq!(readers(&other), [("comember".to_owned(), json!(2))]);
    assert_eq!(other["club_rating"], 3.5);
    assert_eq!(other["rating_count"], 2);
    assert_eq!(other["quote_count"], 2);

    // Strangers and former co-members see nobody else's data.
    let stranger = v.stranger.get(&path).await.json();
    assert!(readers(&stranger).is_empty());
    assert_eq!(stranger["club_rating"], 1.0);
    assert_eq!(stranger["rating_count"], 1);
    assert_eq!(stranger["quote_count"], 1);
    let former = v.former.get(&path).await.json();
    assert!(readers(&former).is_empty());
    assert_eq!(former["club_rating"], Value::Null);
    assert_eq!(former["rating_count"], 0);
    assert_eq!(former["quote_count"], 1);
    assert_eq!(former["on_shelf"], "currently_reading");
    assert_eq!(former["progress"], 10);
}

/// A reader's `dnf_reason` and progress are shelf state (visible to the audience), and the rating
/// mean rounds half to even like Python.
#[sqlx::test]
async fn reader_state_and_the_rating_mean(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let work = app.work("Circe").await.id;
    // Ratings [1, 1, 1, 2]: mean 1.25 rounds to 1.2 (half to even).
    let extra = app.user("extra").await;
    let more = app.user("more").await;
    app.join(&v.shared, &extra, "member").await;
    app.join(&v.shared, &more, "member").await;
    shelve(&app, &v.owner, work, "finished", ", rating = 1").await;
    shelve(&app, &v.co_member, work, "finished", ", rating = 1").await;
    shelve(&app, &extra, work, "finished", ", rating = 1").await;
    shelve(&app, &more, work, "finished", ", rating = 2").await;
    let path = format!("/api/books/works/{work}");
    let body = v.owner.get(&path).await.json();
    assert_eq!(body["club_rating"], 1.2);
    assert_eq!(body["rating_count"], 4);

    app.exec(&format!(
        "UPDATE shelf_entries SET status = 'currently_reading', rating = NULL, progress = 55
         WHERE user_id = {} AND work_id = {work}",
        v.co_member.id()
    ))
    .await;
    let body = v.owner.get(&path).await.json();
    let co = body["readers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["username"] == "comember")
        .unwrap()
        .clone();
    assert_eq!(
        co,
        json!({"username": "comember", "status": "currently_reading", "rating": null,
               "progress": 55, "avatar_url": null})
    );
    assert_eq!(body["club_rating"], 1.3);
    assert_eq!(body["rating_count"], 3);
}

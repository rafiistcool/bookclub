//! Candidates for the next vote (#45, docs/phase1/spec/club.md §1.22, §2.9). Shelf entries are
//! inserted with SQL (SHELF's routes are another package's).

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn want(app: &TestApp, who: &TestClient, work: &TestWork) {
    app.exec(&format!(
        "INSERT INTO shelf_entries (user_id, work_id, status) VALUES ({}, {}, 'want_to_read')",
        who.id(),
        work.id
    ))
    .await;
}

fn titles(body: &Value) -> Vec<String> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["book"]["title"].as_str().unwrap().to_owned())
        .collect()
}

/// parity: test_vote_deadline.py::test_suggestions_rank_shared_tbr_and_subject_affinity
///
/// Subjects and the Open Library rating are work columns (`works.subjects`, `works.ol_rating`),
/// set with SQL here.
#[sqlx::test]
async fn suggestions_rank_shared_wants_and_subject_affinity(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    let path = |rest: &str| format!("/api/clubs/{}{rest}", club.slug);
    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let achilles = app
        .ol_work("OL2W", "The Song of Achilles", "Madeline Miller")
        .await;
    let dune = app.ol_work("OL3W", "Dune", "Frank Herbert").await;
    let piranesi = app.ol_work("OL4W", "Piranesi", "Susanna Clarke").await;
    let suggestions = ada.get(&path("/vote/suggestions")).await;
    suggestions.assert_status(200);
    assert_eq!(suggestions.json(), json!({"items": []}));

    // A past pick with subjects the club evidently likes.
    ada.put(&path("/pick"), json!({"work_id": dune.id}))
        .await
        .assert_status(200);
    app.exec(&format!(
        "UPDATE works SET subjects = ARRAY['Science fiction', 'Politics'] WHERE id = {}",
        dune.id
    ))
    .await;
    ada.delete(&path("/pick")).await.assert_status(200);

    want(&app, &ada, &circe).await;
    want(&app, &ada, &piranesi).await;
    want(&app, &ada, &dune).await; // a past pick: excluded
    let grace = app.user("grace").await;
    app.join(&club, &grace, "member").await;
    want(&app, &grace, &circe).await;
    want(&app, &grace, &achilles).await;
    app.exec(&format!(
        "UPDATE works SET subjects = ARRAY['Greek mythology', 'Science fiction'], ol_rating = 4.4
         WHERE id = {}",
        achilles.id
    ))
    .await;

    let body = grace.get(&path("/vote/suggestions")).await.json();
    let items = body["items"].as_array().unwrap();
    assert_eq!(titles(&body)[0], "Circe");
    assert!(!titles(&body).contains(&"Dune".to_owned()));
    assert_eq!(items[0]["score"], 6);
    assert_eq!(items[0]["reasons"], json!(["On 2 shelves: ada, grace"]));
    assert_eq!(items[0]["book"]["id"], circe.id);
    let achilles_row = items
        .iter()
        .find(|item| item["book"]["title"] == "The Song of Achilles")
        .unwrap();
    assert_eq!(achilles_row["score"], 3);
    assert_eq!(
        achilles_row["reasons"],
        json!([
            "grace wants to read it",
            "Like past picks: Science fiction",
            "Open Library rating 4.4"
        ])
    );
    assert_eq!(
        items[0]["reason_codes"],
        json!([{"code": "wanted", "usernames": ["ada", "grace"]}])
    );
    assert_eq!(
        achilles_row["reason_codes"],
        json!([
            {"code": "wanted", "usernames": ["grace"]},
            {"code": "like_past_picks", "subjects": ["Science fiction"]},
            {"code": "ol_rating", "rating": 4.4}
        ])
    );
    // Equal scores sort by title: Achilles (3), then Piranesi (1).
    assert_eq!(titles(&body), ["Circe", "The Song of Achilles", "Piranesi"]);

    // Nominating removes a book from the suggestions; so does making it the pick.
    ada.post(&path("/vote/nominations"), json!({"work_id": circe.id}))
        .await
        .assert_status(201);
    let body = ada.get(&path("/vote/suggestions")).await.json();
    assert_eq!(titles(&body), ["The Song of Achilles", "Piranesi"]);
    ada.put(&path("/pick"), json!({"work_id": piranesi.id}))
        .await
        .assert_status(200);
    let body = ada.get(&path("/vote/suggestions")).await.json();
    assert_eq!(titles(&body), ["The Song of Achilles"]);
}

/// Candidates come from the club's members only, at most 6, best first then by title (case
/// folded with `to_lowercase`).
#[sqlx::test]
async fn suggestions_cover_club_members_only(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    let path = format!("/api/clubs/{}/vote/suggestions", club.slug);
    let outsider = app.user("outsider").await;
    let other = app.club(&outsider, "Other").await;
    app.join(&other, &ada, "member").await;
    let shared = app.work("Shared").await;
    want(&app, &ada, &shared).await;
    // The outsider shares another club with ada, but is no member of this one.
    want(&app, &outsider, &shared).await;
    let only_theirs = app.work("Only theirs").await;
    want(&app, &outsider, &only_theirs).await;
    let body = ada.get(&path).await.json();
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["reasons"], json!(["ada wants to read it"]));

    for title in ["beta", "Alpha", "gamma", "Delta", "epsilon", "Zeta", "eta"] {
        let work = app.work(title).await;
        want(&app, &ada, &work).await;
    }
    let body = ada.get(&path).await.json();
    assert_eq!(
        titles(&body),
        ["Alpha", "beta", "Delta", "epsilon", "eta", "gamma"]
    );
    // Statuses other than want-to-read never count.
    app.exec("UPDATE shelf_entries SET status = 'currently_reading'")
        .await;
    assert_eq!(ada.get(&path).await.json(), json!({"items": []}));
}

//! The search history: the caller's last 20 searches, deduplicated by Unicode case fold
//! (endpoint-map §7, docs/phase1/spec/diary.md §1.16–§1.19; #44).

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const PATH: &str = "/api/search-history";

fn queries(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|row| row["query"].as_str().unwrap().to_owned())
        .collect()
}

/// parity: test_search_history.py::test_history_requires_login
#[sqlx::test]
async fn history_requires_login(db: PgPool) {
    let app = TestApp::new(db).await;
    let anon = app.anon();
    anon.get(PATH).await.assert_detail(401, "Not signed in");
    anon.post(PATH, json!({"query": "Dune"}))
        .await
        .assert_detail(401, "Not signed in");
    anon.delete(PATH).await.assert_detail(401, "Not signed in");
    anon.delete(&format!("{PATH}/1"))
        .await
        .assert_detail(401, "Not signed in");
}

/// parity: test_search_history.py::test_history_deduplicates_orders_and_caps
#[sqlx::test]
async fn history_deduplicates_orders_and_caps(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for index in 0..22 {
        ada.post(PATH, json!({"query": format!("Book {index}")}))
            .await
            .assert_status(200);
    }
    let history = ada.get(PATH).await.json();
    assert_eq!(history.as_array().unwrap().len(), 20);
    assert_eq!(history[0]["query"], "Book 21");
    assert_eq!(history[19]["query"], "Book 2");
    assert!(history[0]["last_used_at"].as_str().unwrap().ends_with('Z'));
    let original = history[19]["id"].clone();

    let res = ada.post(PATH, json!({"query": "  BOOK   2  "})).await;
    res.assert_status(200);
    let list = res.json();
    assert_eq!(list[0]["id"], original);
    assert_eq!(list[0]["query"], "BOOK 2");
    assert_eq!(list.as_array().unwrap().len(), 20);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM search_history").await,
        20
    );

    ada.post(PATH, json!({"query": "  "}))
        .await
        .assert_detail(400, "Search must contain between 1 and 300 characters");
    ada.post(PATH, json!({"query": "x".repeat(301)}))
        .await
        .assert_detail(400, "Search must contain between 1 and 300 characters");
    // Characters, not bytes, after collapsing the whitespace.
    ada.post(PATH, json!({"query": format!(" {} ", "ß".repeat(300))}))
        .await
        .assert_status(200);
}

/// The dedupe key is the Unicode case fold, not a lowercase: "Straße" and "STRASSE" are one.
#[sqlx::test]
async fn history_folds_case_like_python(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let first = ada.post(PATH, json!({"query": "Straße Nord"})).await.json();
    let second = ada
        .post(PATH, json!({"query": "STRASSE\u{a0}NORD"}))
        .await
        .json();
    assert_eq!(second.as_array().unwrap().len(), 1);
    assert_eq!(second[0]["id"], first[0]["id"]);
    assert_eq!(queries(&second), ["STRASSE NORD"]);
}

/// parity: test_search_history.py::test_history_is_private_and_delete_is_scoped
#[sqlx::test]
async fn history_is_private_and_delete_is_scoped(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &bob, "member").await;
    let entry = ada.post(PATH, json!({"query": "Circe"})).await.json()[0].clone();

    assert_eq!(bob.get(PATH).await.json(), json!([]));
    bob.delete(&format!("{PATH}/{}", entry["id"]))
        .await
        .assert_detail(404, "Search history entry not found");
    bob.post(PATH, json!({"query": "Dune"}))
        .await
        .assert_status(200);
    bob.delete(PATH).await.assert_status(204);
    assert_eq!(bob.get(PATH).await.json(), json!([]));
    bob.delete(PATH).await.assert_status(204);

    assert_eq!(queries(&ada.get(PATH).await.json()), ["Circe"]);
    ada.delete(&format!("{PATH}/{}", entry["id"]))
        .await
        .assert_status(204);
    assert_eq!(ada.get(PATH).await.json(), json!([]));
    ada.delete(&format!("{PATH}/{}", entry["id"]))
        .await
        .assert_detail(404, "Search history entry not found");
}

/// Searches remembered at the same moment (parallel tabs) never leave more than 20 rows.
#[sqlx::test]
async fn parallel_searches_keep_the_cap(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bodies: Vec<Value> = (0..30)
        .map(|n| json!({"query": format!("Query {n}")}))
        .collect();
    let posts = bodies.iter().map(|body| ada.post(PATH, body.clone()));
    for res in futures::future::join_all(posts).await {
        res.assert_status(200);
    }
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM search_history").await,
        20
    );
    assert_eq!(ada.get(PATH).await.json().as_array().unwrap().len(), 20);
}

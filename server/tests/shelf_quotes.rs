//! Quotes: create, list and filter, edit, delete, the edition a page refers to and the
//! `quote_added` event (endpoint-map §7, docs/phase1/spec/diary.md §1.7–§1.10; #44).

mod common;

use bookclub_server::tz;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// ada and grace, who share a club, so each is in the other's audience.
async fn friends(app: &TestApp) -> (TestClient, TestClient) {
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    (ada, grace)
}

/// Keeps a quote; asserts 201.
async fn quote(client: &TestClient, body: Value) -> Value {
    let res = client.post("/api/quotes", body).await;
    res.assert_status(201);
    res.json()
}

async fn list(client: &TestClient, query: &str) -> Vec<Value> {
    let res = client.get(&format!("/api/quotes{query}")).await;
    res.assert_status(200);
    res.json()["items"].as_array().unwrap().clone()
}

/// The kind and payload of each event the user recorded, oldest first.
async fn events(app: &TestApp, user: &TestClient) -> Vec<(String, Value)> {
    sqlx::query_as::<_, (String, Value)>(
        "SELECT kind, payload FROM events WHERE actor_id = $1 ORDER BY id",
    )
    .bind(user.id())
    .fetch_all(&app.db)
    .await
    .unwrap()
}

/// parity: test_stats_milestones_quotes.py::test_quotes_crud_and_filters
/// Quotes name a catalog work by id: an unknown work is 404 `No such book.` (the legacy
/// "title is required for a new book" is gone), and `?work=` takes the work id. The book
/// detail's `quote_count` is CAT's; the count it reads (`bc_visible_quotes`) is checked here.
#[sqlx::test]
async fn quotes_crud_and_filters(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let achilles = app
        .ol_work("OL2W", "The Song of Achilles", "Madeline Miller")
        .await;
    ada.post("/api/quotes", json!({"work_id": 999_999, "body": "x"}))
        .await
        .assert_detail(404, "No such book.");

    let created = quote(
        &ada,
        json!({"work_id": circe.id, "body": "  Humbling women.  ", "page": 12}),
    )
    .await;
    assert_eq!(created["body"], "Humbling women.");
    assert_eq!(created["page"], 12);
    assert_eq!(created["mine"], true);
    assert_eq!(created["author"], "ada");
    assert_eq!(created["book"]["title"], "Circe");
    assert_eq!(created["visibility"], "audience");
    let id = created["id"].clone();

    quote(
        &ada,
        json!({"work_id": achilles.id, "body": "Name one hero who was happy."}),
    )
    .await;
    assert_eq!(list(&ada, "").await.len(), 2);
    let only_circe = list(&ada, &format!("?work={}", circe.id)).await;
    let titles: Vec<&Value> = only_circe.iter().map(|q| &q["book"]["title"]).collect();
    assert_eq!(titles, vec!["Circe"]);
    assert_eq!(list(&ada, "?work=999999").await, Vec::<Value>::new());

    let edited = ada
        .patch(&format!("/api/quotes/{id}"), json!({"page": null}))
        .await;
    edited.assert_status(200);
    assert_eq!(edited.json()["page"], Value::Null);
    ada.patch(&format!("/api/quotes/{id}"), json!({}))
        .await
        .assert_detail(400, "Nothing to update");
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM bc_visible_quotes({}) WHERE work_id = {}",
            ada.id(),
            circe.id
        ))
        .await,
        1
    );

    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    assert_eq!(list(&grace, "?username=ada").await[0]["mine"], false);
    grace
        .patch(&format!("/api/quotes/{id}"), json!({"body": "hijack"}))
        .await
        .assert_detail(403, "You can only edit your own quotes");
    grace
        .delete(&format!("/api/quotes/{id}"))
        .await
        .assert_detail(403, "You can only delete your own quotes");
    assert_eq!(list(&grace, "?username=grace").await, Vec::<Value>::new());

    ada.delete(&format!("/api/quotes/{id}"))
        .await
        .assert_status(204);
    ada.delete(&format!("/api/quotes/{id}"))
        .await
        .assert_detail(404, "No quote with that id");
    let kinds: Vec<String> = events(&app, &ada).await.into_iter().map(|e| e.0).collect();
    assert!(kinds.contains(&"quote_added".to_owned()), "{kinds:?}");
}

#[sqlx::test]
async fn quote_bodies_and_pages_are_validated(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = app.work("Circe").await;
    for (body, detail) in [
        (
            json!({"work_id": circe.id, "body": "   "}),
            "Paste the line you want to keep",
        ),
        (
            json!({"work_id": circe.id, "body": "é".repeat(1001)}),
            "Keep it to 1000 characters",
        ),
        (
            json!({"work_id": circe.id, "body": "", "page": -1}),
            "Paste the line you want to keep; Input should be greater than or equal to 0",
        ),
        (
            json!({"work_id": circe.id, "body": "x", "page": 20001}),
            "Input should be less than or equal to 20000",
        ),
        (json!({"work_id": circe.id}), "Field required"),
    ] {
        ada.post("/api/quotes", body)
            .await
            .assert_detail(400, detail);
    }
    // 1000 characters (not bytes) fit; the excerpt keeps 80 of them.
    let long = "é".repeat(1000);
    let created = quote(
        &ada,
        json!({"work_id": circe.id, "body": long, "page": 20000}),
    )
    .await;
    let (kind, payload) = events(&app, &ada).await.pop().unwrap();
    assert_eq!(kind, "quote_added");
    assert_eq!(
        payload,
        json!({"page": 20000, "excerpt": "é".repeat(80), "quote_id": created["id"]})
    );
    // Without a page the payload has no `page` key.
    let created = quote(&ada, json!({"work_id": circe.id, "body": "Short"})).await;
    assert_eq!(
        events(&app, &ada).await.pop().unwrap().1,
        json!({"excerpt": "Short", "quote_id": created["id"]})
    );

    let id = &created["id"];
    for (body, detail) in [
        (json!({"body": "  "}), "Paste the line you want to keep"),
        (
            json!({"page": 20001}),
            "Input should be less than or equal to 20000",
        ),
        // `body: null` and `visibility: null` count as absent.
        (json!({"body": null}), "Nothing to update"),
        (json!({"visibility": null}), "Nothing to update"),
    ] {
        ada.patch(&format!("/api/quotes/{id}"), body)
            .await
            .assert_detail(400, detail);
    }
    let edited = ada
        .patch(
            &format!("/api/quotes/{id}"),
            json!({"body": " New ", "page": 3}),
        )
        .await
        .json();
    assert_eq!(
        (edited["body"].clone(), edited["page"].clone()),
        (json!("New"), json!(3))
    );
    // An absent page keeps the page.
    let edited = ada
        .patch(&format!("/api/quotes/{id}"), json!({"body": "Newer"}))
        .await
        .json();
    assert_eq!(edited["page"], 3);
    ada.patch("/api/quotes/x", json!({"page": 1}))
        .await
        .assert_detail(
            400,
            "Input should be a valid integer, unable to parse string as an integer",
        );
    ada.patch("/api/quotes/999999", json!({"page": 1}))
        .await
        .assert_detail(404, "No quote with that id");
}

/// The order of checks on PATCH: validation, 404, 403, then `Nothing to update`.
#[sqlx::test]
async fn patch_checks_run_in_the_legacy_order(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let circe = app.work("Circe").await;
    let id = quote(&ada, json!({"work_id": circe.id, "body": "Line"})).await["id"].clone();
    grace
        .patch(&format!("/api/quotes/{id}"), json!({}))
        .await
        .assert_detail(403, "You can only edit your own quotes");
    grace
        .patch(&format!("/api/quotes/{id}"), json!({"body": ""}))
        .await
        .assert_detail(400, "Paste the line you want to keep");
    grace
        .patch("/api/quotes/999999", json!({}))
        .await
        .assert_detail(404, "No quote with that id");
}

/// The edition a page refers to: the caller's shelf edition by default, or an explicit edition
/// of the work.
#[sqlx::test]
async fn quotes_name_an_edition(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = app.work("Circe").await;
    let edition = app.edition(&circe, "9780316556347").await;
    let other = app.edition(&circe, "9780349142975").await;
    let dune = app.work("Dune").await;
    let dune_edition = app.edition(&dune, "9780441013593").await;

    // Not on the shelf: no edition.
    let first = quote(&ada, json!({"work_id": circe.id, "body": "One"})).await;
    assert_eq!(first["edition_id"], Value::Null);
    ada.post(
        "/api/shelf",
        json!({"work_id": circe.id, "edition_id": edition.id}),
    )
    .await
    .assert_status(201);
    let second = quote(&ada, json!({"work_id": circe.id, "body": "Two"})).await;
    assert_eq!(second["edition_id"], edition.id);
    let third = quote(
        &ada,
        json!({"work_id": circe.id, "edition_id": other.id, "body": "Three"}),
    )
    .await;
    assert_eq!(third["edition_id"], other.id);
    ada.post(
        "/api/quotes",
        json!({"work_id": circe.id, "edition_id": dune_edition.id, "body": "Four"}),
    )
    .await
    .assert_detail(400, "That edition belongs to another book");
    ada.post(
        "/api/quotes",
        json!({"work_id": circe.id, "edition_id": 999_999, "body": "Five"}),
    )
    .await
    .assert_detail(400, "That edition belongs to another book");
}

/// `?work=` follows merges, `limit` is 1..500 and keeps the newest, and `created_label` is in
/// the viewer's zone.
#[sqlx::test]
async fn list_filters_limit_and_labels(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let old = app.work("Old Circe").await;
    let circe = app.work("Circe").await;
    let dune = app.work("Dune").await;
    let first = quote(&ada, json!({"work_id": old.id, "body": "Old"})).await;
    quote(&grace, json!({"work_id": dune.id, "body": "Spice"})).await;
    let newest = quote(&ada, json!({"work_id": dune.id, "body": "Sand"})).await;
    app.exec(&format!(
        "UPDATE works SET merged_into_id = {circe} WHERE id = {old};
         UPDATE quotes SET work_id = {circe} WHERE work_id = {old}",
        circe = circe.id,
        old = old.id
    ))
    .await;
    let merged = list(&ada, &format!("?work={}", old.id)).await;
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0]["id"], first["id"]);

    let bodies = |items: Vec<Value>| -> Vec<String> {
        items
            .iter()
            .map(|q| q["body"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(bodies(list(&ada, "").await), vec!["Sand", "Spice", "Old"]);
    assert_eq!(bodies(list(&ada, "?limit=1").await), vec!["Sand"]);
    assert_eq!(
        bodies(list(&ada, &format!("?username=GRACE&work={}", dune.id)).await),
        vec!["Spice"]
    );
    // Empty filters are ignored; an unknown name gives an empty list.
    assert_eq!(list(&ada, "?username=").await.len(), 3);
    assert_eq!(list(&ada, "?work=").await.len(), 3);
    assert_eq!(
        bodies(list(&ada, "?work=&username=grace").await),
        vec!["Spice"]
    );
    assert_eq!(list(&ada, "?username=nobody").await, Vec::<Value>::new());
    for (query, detail) in [
        ("?limit=0", "Input should be greater than or equal to 1"),
        ("?limit=501", "Input should be less than or equal to 500"),
        (
            "?work=OL1W",
            "Input should be a valid integer, unable to parse string as an integer",
        ),
        (
            "?work=99999999999999999999",
            "Input should be a valid integer, unable to parse string as an integer",
        ),
    ] {
        ada.get(&format!("/api/quotes{query}"))
            .await
            .assert_detail(400, detail);
    }

    app.exec("UPDATE users SET timezone = 'Pacific/Kiritimati' WHERE username = 'grace'")
        .await;
    let created_at = time::OffsetDateTime::parse(
        newest["created_at"].as_str().unwrap(),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    let seen = list(&grace, "?limit=1").await;
    assert_eq!(
        seen[0]["created_label"],
        tz::label(created_at, &tz::zone_or_utc("Pacific/Kiritimati"))
    );
    assert_eq!(
        newest["created_label"],
        tz::label(created_at, &tz::zone_or_utc("UTC"))
    );
}

/// The owner can make a quote `only_me` and back; others then no longer see it.
#[sqlx::test]
async fn visibility_can_be_changed(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let circe = app.work("Circe").await;
    let id = quote(
        &ada,
        json!({"work_id": circe.id, "body": "Mine", "visibility": "only_me"}),
    )
    .await["id"]
        .clone();
    assert_eq!(list(&grace, "?username=ada").await, Vec::<Value>::new());
    let edited = ada
        .patch(
            &format!("/api/quotes/{id}"),
            json!({"visibility": "audience"}),
        )
        .await
        .json();
    assert_eq!(edited["visibility"], "audience");
    assert_eq!(list(&grace, "?username=ada").await[0]["id"], id);
    ada.patch(
        &format!("/api/quotes/{id}"),
        json!({"visibility": "only_me"}),
    )
    .await
    .assert_status(200);
    assert_eq!(list(&grace, "").await, Vec::<Value>::new());
    assert_eq!(list(&ada, "").await[0]["visibility"], "only_me");
}

#[sqlx::test]
async fn quotes_require_sign_in(db: PgPool) {
    let app = TestApp::new(db).await;
    let anon = app.anon();
    anon.get("/api/quotes")
        .await
        .assert_detail(401, "Not signed in");
    anon.post("/api/quotes", json!({"work_id": 1, "body": "x"}))
        .await
        .assert_detail(401, "Not signed in");
    anon.patch("/api/quotes/1", json!({}))
        .await
        .assert_detail(401, "Not signed in");
    anon.delete("/api/quotes/1")
        .await
        .assert_detail(401, "Not signed in");
}

//! Reading dates: `started_at` and `finished_at` across status changes, and `finished_on` in
//! the owner's time zone (docs/phase1/spec/shelf.md §1.7 steps 4 and 9, §2.4; #44).

mod common;

use bookclub_server::tz;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

async fn add(client: &TestClient, work: &TestWork, status: &str, extra: Value) -> Value {
    let mut body = json!({ "work_id": work.id, "status": status });
    if let (Some(body), Value::Object(extra)) = (body.as_object_mut(), extra) {
        body.extend(extra);
    }
    let res = client.post("/api/shelf", body).await;
    res.assert_status(201);
    res.json()
}

async fn patch(client: &TestClient, id: &Value, body: Value) -> Value {
    let res = client.patch(&format!("/api/shelf/{id}"), body).await;
    res.assert_status(200);
    res.json()
}

fn at(value: &Value) -> OffsetDateTime {
    OffsetDateTime::parse(value.as_str().expect("a timestamp"), &Rfc3339).unwrap()
}

async fn circe(app: &TestApp) -> TestWork {
    app.ol_work("OL1W", "Circe", "Madeline Miller").await
}

async fn set_zone(app: &TestApp, user: &TestClient, zone: &str) {
    app.exec(&format!(
        "UPDATE users SET timezone = '{zone}' WHERE id = {}",
        user.id()
    ))
    .await;
}

/// parity: test_reading_dates.py::test_want_to_read_has_no_dates
#[sqlx::test]
async fn want_to_read_has_no_dates(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(&ada, &circe(&app).await, "want_to_read", json!({})).await;
    assert_eq!(item["started_at"], Value::Null);
    assert_eq!(item["finished_at"], Value::Null);
}

/// parity: test_reading_dates.py::test_reading_sets_started_and_finish_sets_finished
#[sqlx::test]
async fn reading_sets_started_and_finish_sets_finished(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(&ada, &circe(&app).await, "want_to_read", json!({})).await;
    let reading = patch(
        &ada,
        &item["id"],
        json!({"status": "currently_reading", "position": 0}),
    )
    .await;
    assert_ne!(reading["started_at"], Value::Null);
    assert_eq!(reading["finished_at"], Value::Null);
    let started = reading["started_at"].clone();

    let finished = patch(
        &ada,
        &item["id"],
        json!({"status": "finished", "position": 0, "rating": 4}),
    )
    .await;
    assert_eq!(finished["started_at"], started);
    assert!(at(&finished["finished_at"]) >= at(&started));
}

/// parity: test_reading_dates.py::test_added_directly_as_finished_stamps_both
#[sqlx::test]
async fn added_directly_as_finished_stamps_both(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(&ada, &circe(&app).await, "finished", json!({"rating": 5})).await;
    assert_ne!(item["started_at"], Value::Null);
    assert_eq!(item["finished_at"], item["started_at"]);
    let age = OffsetDateTime::now_utc() - at(&item["finished_at"]);
    assert!(age.whole_seconds().abs() < 60);
}

/// parity: test_reading_dates.py::test_reorder_within_finished_keeps_finished_at
#[sqlx::test]
async fn reorder_within_finished_keeps_finished_at(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(&ada, &circe(&app).await, "finished", json!({"rating": 5})).await;
    let moved = patch(&ada, &item["id"], json!({"position": 0})).await;
    assert_eq!(moved["finished_at"], item["finished_at"]);
}

/// parity: test_reading_dates.py::test_leaving_finished_clears_finished_at_but_keeps_start
#[sqlx::test]
async fn leaving_finished_clears_finished_at_but_keeps_start(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(&ada, &circe(&app).await, "finished", json!({"rating": 5})).await;
    let back = patch(
        &ada,
        &item["id"],
        json!({"status": "currently_reading", "position": 0}),
    )
    .await;
    assert_eq!(back["finished_at"], Value::Null);
    assert_eq!(back["started_at"], item["started_at"]);
}

/// parity: test_reading_dates.py::test_back_to_want_to_read_resets_dates
#[sqlx::test]
async fn back_to_want_to_read_resets_dates(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(&ada, &circe(&app).await, "finished", json!({"rating": 5})).await;
    let reset = patch(
        &ada,
        &item["id"],
        json!({"status": "want_to_read", "position": 0}),
    )
    .await;
    assert_eq!(reset["started_at"], Value::Null);
    assert_eq!(reset["finished_at"], Value::Null);
}

/// parity: test_reading_dates.py::test_dnf_has_start_but_no_finish
#[sqlx::test]
async fn dnf_has_start_but_no_finish(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(
        &ada,
        &circe(&app).await,
        "did_not_finish",
        json!({"dnf_reason": "Too long"}),
    )
    .await;
    assert_ne!(item["started_at"], Value::Null);
    assert_eq!(item["finished_at"], Value::Null);
}

/// parity: test_reading_dates.py::test_edit_and_clear_finish_date_survive_reordering
/// The legacy instance zone (`BOOKCLUB_TZ`) is now the owner's `users.timezone`.
#[sqlx::test]
async fn edit_and_clear_finish_date_survive_reordering(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    set_zone(&app, &ada, "Pacific/Kiritimati").await;
    let item = add(&ada, &circe(&app).await, "finished", json!({})).await;
    let edited = patch(&ada, &item["id"], json!({"finished_on": "2025-07-01"})).await;
    // Noon on July 1 in UTC+14 is still June 30 in UTC.
    assert_eq!(edited["finished_at"], "2025-06-30T22:00:00Z");
    assert!(at(&edited["started_at"]) <= at(&edited["finished_at"]));
    assert_eq!(
        ada.get("/api/shelf").await.json()["timezone"],
        "Pacific/Kiritimati"
    );

    let moved = patch(
        &ada,
        &item["id"],
        json!({"position": 0, "status": "finished"}),
    )
    .await;
    assert_eq!(moved["finished_at"], edited["finished_at"]);
    assert_eq!(
        patch(&ada, &item["id"], json!({"finished_on": null})).await["finished_at"],
        Value::Null
    );
    assert_eq!(
        patch(
            &ada,
            &item["id"],
            json!({"position": 0, "status": "finished"})
        )
        .await["finished_at"],
        Value::Null
    );
    assert_eq!(
        patch(&ada, &item["id"], json!({"rating": 4})).await["finished_at"],
        Value::Null
    );
}

/// parity: test_reading_dates.py::test_invalid_finish_date_does_not_modify_entry
#[sqlx::test]
async fn invalid_finish_date_does_not_modify_entry(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(&ada, &circe(&app).await, "want_to_read", json!({})).await;
    let path = format!("/api/shelf/{}", item["id"]);
    ada.patch(&path, json!({"finished_on": "2025-07-01"}))
        .await
        .assert_detail(400, "Reading date requires a finished book");
    ada.patch(&path, json!({"finished_on": null}))
        .await
        .assert_detail(400, "Reading date requires a finished book");
    ada.patch(
        &path,
        json!({"status": "finished", "finished_on": "2999-01-01"}),
    )
    .await
    .assert_detail(400, "Reading date cannot be in the future");
    let shelf = ada.get("/api/shelf").await.json();
    assert_eq!(shelf["items"][0]["status"], "want_to_read");
    assert_eq!(shelf["items"][0]["updated_at"], item["updated_at"]);
    for bad in ["invalid", "2025-02-30", "0000-01-01", "-0001-01-01"] {
        ada.patch(&path, json!({"status": "finished", "finished_on": bad}))
            .await
            .assert_status(400);
    }
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM events WHERE kind <> 'shelf_added'")
            .await,
        0
    );
}

/// "Today" is the owner's: the last allowed date is today in their zone, the next day is in the
/// future.
#[sqlx::test]
async fn today_is_the_owners_today(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    set_zone(&app, &ada, "Pacific/Kiritimati").await;
    let item = add(&ada, &circe(&app).await, "finished", json!({})).await;
    let today = tz::today(&tz::zone_or_utc("Pacific/Kiritimati"));
    let tomorrow = today.next_day().unwrap();
    let saved = patch(&ada, &item["id"], json!({"finished_on": today.to_string()})).await;
    assert_eq!(
        at(&saved["finished_at"]),
        tz::noon_local_utc(today, &tz::zone_or_utc("Pacific/Kiritimati"))
    );
    ada.patch(
        &format!("/api/shelf/{}", item["id"]),
        json!({"finished_on": tomorrow.to_string()}),
    )
    .await
    .assert_detail(400, "Reading date cannot be in the future");
}

/// parity: test_reading_dates.py::test_reread_replaces_completion_without_duplicate
#[sqlx::test]
async fn reread_replaces_completion_without_duplicate(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(&ada, &circe(&app).await, "finished", json!({})).await;
    patch(&ada, &item["id"], json!({"finished_on": "2020-07-01"})).await;
    patch(&ada, &item["id"], json!({"status": "currently_reading"})).await;
    let finished = patch(&ada, &item["id"], json!({"status": "finished"})).await;
    assert!(
        !finished["finished_at"]
            .as_str()
            .unwrap()
            .starts_with("2020-07")
    );
    // The re-read keeps the clamped first start.
    assert!(
        finished["started_at"]
            .as_str()
            .unwrap()
            .starts_with("2020-07-01")
    );
    assert_eq!(
        ada.get("/api/shelf").await.json()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

/// parity: test_reading_dates.py::test_move_between_months_saves_order_and_date_together
#[sqlx::test]
async fn move_between_months_saves_order_and_date_together(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    set_zone(&app, &ada, "Europe/Berlin").await;
    let first = add(
        &ada,
        &circe(&app).await,
        "finished",
        json!({"rating": 5, "take": "Keep my review"}),
    )
    .await;
    let second = add(
        &ada,
        &app.ol_work("OL2W", "Song", "").await,
        "finished",
        json!({}),
    )
    .await;
    patch(&ada, &first["id"], json!({"finished_on": "2025-09-23"})).await;
    let moved = patch(
        &ada,
        &first["id"],
        json!({"shelf_position": 1, "finished_on": "2025-08-01"}),
    )
    .await;
    assert_eq!(moved["shelf_position"], 1);
    assert_eq!(moved["finished_at"], "2025-08-01T10:00:00Z");
    assert_eq!(moved["status"], "finished");
    assert_eq!(moved["rating"], 5);
    assert_eq!(moved["take"], "Keep my review");

    let persisted = ada.get("/api/shelf").await.json()["items"].clone();
    let row = |id: &Value| {
        persisted
            .as_array()
            .unwrap()
            .iter()
            .find(|row| &row["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(row(&first["id"])["finished_at"], moved["finished_at"]);
    assert_eq!(row(&second["id"])["shelf_position"], 0);

    // Reordering within August keeps the completion date.
    let reordered = patch(&ada, &first["id"], json!({"shelf_position": 0})).await;
    assert_eq!(reordered["finished_at"], moved["finished_at"]);
    let before = ada.get("/api/shelf").await.json()["items"].clone();
    ada.patch(
        &format!("/api/shelf/{}", first["id"]),
        json!({"shelf_position": 1, "finished_on": "2999-01-01"}),
    )
    .await
    .assert_detail(400, "Reading date cannot be in the future");
    assert_eq!(ada.get("/api/shelf").await.json()["items"], before);
}

/// parity: test_reading_dates.py::test_cannot_change_another_members_completion_month
#[sqlx::test]
async fn cannot_change_another_members_completion_month(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    let item = add(&ada, &circe(&app).await, "finished", json!({})).await;
    grace
        .patch(
            &format!("/api/shelf/{}", item["id"]),
            json!({"shelf_position": 0, "finished_on": "2025-08-01"}),
        )
        .await
        .assert_detail(404, "Book not on your shelf");
    let shelf = ada.get("/api/shelf").await.json();
    assert_eq!(shelf["items"][0]["finished_at"], item["finished_at"]);
    assert_eq!(shelf["items"][0]["shelf_position"], Value::Null);
}

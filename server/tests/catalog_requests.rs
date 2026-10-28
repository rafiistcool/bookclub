//! Edit and merge requests and the admin queue (#41, endpoint-map §4): readers propose, an
//! instance admin approves (through `apply_admin_fields`, `set_cover(…, Admin)` or
//! `merge_works`) or rejects.

mod common;

use std::time::Duration;

use bookclub_server::catalog::merge_works;
use bookclub_server::catalog::shared::ingest;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn request_count(app: &TestApp) -> i64 {
    app.scalar_i64("SELECT count(*) FROM catalog_edit_requests")
        .await
}

/// A cover picture as MED stores an upload (bytes are not needed here).
async fn upload_media(app: &TestApp, uploader: &TestClient) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO media (public_id, kind, source, uploaded_by)
         VALUES (substr(md5(random()::text) || md5(random()::text), 1, 26), 'cover', 'upload', $1)
         RETURNING id",
    )
    .bind(uploader.id())
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// A `cover_replace` request as MED files it when a cover exists.
async fn cover_request(
    app: &TestApp,
    who: &TestClient,
    work: i64,
    edition: Option<i64>,
    media: i64,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO catalog_edit_requests (kind, work_id, edition_id, media_id, created_by)
         VALUES ('cover_replace', $1, $2, $3, $4) RETURNING id",
    )
    .bind(work)
    .bind(edition)
    .bind(media)
    .bind(who.id())
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// A reader proposes a correction; nothing changes until an admin approves it, which writes the
/// fields with provenance `admin`. The queue lists open requests, oldest first.
#[sqlx::test]
async fn work_edits_wait_for_an_admin(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let work = app.ol_work("OL1W", "Cirse", "Madeline Miller").await;

    let res = ada
        .post(
            &format!("/api/books/works/{}/edit-requests", work.id),
            json!({"fields": {"title": "Circe", "year": 2018, "subjects": ["Mythology"]},
                   "note": "  Typo in the title  "}),
        )
        .await;
    res.assert_status(201);
    let id = res.json()["id"].as_i64().unwrap();
    assert_eq!(res.json(), json!({"id": id, "status": "open"}));
    let unchanged = ada
        .get(&format!("/api/books/works/{}", work.id))
        .await
        .json();
    assert_eq!(unchanged["title"], "Cirse");

    let later = ada
        .post(
            "/api/books/works/OL1W/edit-requests",
            json!({"fields": {"description": "A witch."}}),
        )
        .await;
    later.assert_status(201);
    let queue = root.get("/api/admin/edit-requests").await;
    queue.assert_status(200);
    let items = queue.json()["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 2);
    let first = &items[0];
    assert_eq!(first["id"], id);
    assert_eq!(first["kind"], "work_edit");
    assert_eq!(first["status"], "open");
    assert_eq!(first["work_id"], work.id);
    assert_eq!(first["edition_id"], Value::Null);
    assert_eq!(first["book"]["title"], "Cirse");
    assert_eq!(
        first["payload"],
        json!({"title": "Circe", "year": 2018, "subjects": ["Mythology"]})
    );
    assert_eq!(first["note"], "Typo in the title");
    assert_eq!(first["created_by"]["username"], "ada");
    assert!(first["created_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(first["decided_by"], Value::Null);
    assert_eq!(first["decided_at"], Value::Null);
    assert_eq!(first["cover_url"], Value::Null);

    let approved = root
        .post(&format!("/api/admin/edit-requests/{id}/approve"), json!({}))
        .await;
    approved.assert_status(200);
    let body = approved.json();
    assert_eq!(body["status"], "approved");
    assert_eq!(body["decided_by"]["username"], "root");
    assert!(body["decided_at"].is_string());
    assert_eq!(body["book"]["title"], "Circe");
    let detail = ada
        .get(&format!("/api/books/works/{}", work.id))
        .await
        .json();
    assert_eq!(detail["title"], "Circe");
    assert_eq!(detail["year"], 2018);
    assert_eq!(detail["subjects"], json!(["Mythology"]));
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM works WHERE id = {} AND field_sources ->> 'title' = 'admin'
               AND field_sources ->> 'first_publish_year' = 'admin'",
            work.id
        ))
        .await,
        1
    );
    // Decided requests leave the queue and cannot be decided again.
    let queue = root.get("/api/admin/edit-requests").await.json();
    assert_eq!(queue["items"].as_array().unwrap().len(), 1);
    root.post(&format!("/api/admin/edit-requests/{id}/approve"), json!({}))
        .await
        .assert_detail(409, "This request was already decided");
    root.post(&format!("/api/admin/edit-requests/{id}/reject"), json!({}))
        .await
        .assert_detail(409, "This request was already decided");
    root.post("/api/admin/edit-requests/424242/approve", json!({}))
        .await
        .assert_detail(404, "Not found");
}

/// A proposal is checked with its approval's rules before it is queued: unknown keys, bad
/// values, a cleared work title and an empty proposal are 400; unknown books are 404.
#[sqlx::test]
async fn edit_requests_are_checked_like_their_approval(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let work = app.work("My Zine").await;
    let path = format!("/api/books/works/{}/edit-requests", work.id);
    for (fields, detail) in [
        (json!({"isbn": "9780316556347"}), "Unknown field: isbn"),
        (json!({"year": "soon"}), "Invalid value for year"),
        (json!({"title": null}), "Invalid value for title"),
        (json!({"pages": 0}), "Invalid value for pages"),
        (json!({}), "Field required"),
    ] {
        ada.post(&path, json!({"fields": fields}))
            .await
            .assert_detail(400, detail);
    }
    ada.post(&path, json!({"note": "no fields"}))
        .await
        .assert_detail(400, "Field required");
    ada.post(&path, json!({"fields": ["title"]}))
        .await
        .assert_status(400);
    ada.post(
        "/api/books/works/424242/edit-requests",
        json!({"fields": {"title": "X"}}),
    )
    .await
    .assert_detail(404, "No such book.");
    ada.post(
        "/api/books/editions/424242/edit-requests",
        json!({"fields": {"publisher": "X"}}),
    )
    .await
    .assert_detail(404, "No such book.");
    // The dry run wrote nothing.
    assert_eq!(request_count(&app).await, 0);
    let detail = ada
        .get(&format!("/api/books/works/{}", work.id))
        .await
        .json();
    assert_eq!(detail["title"], "My Zine");
    app.anon()
        .post(&path, json!({"fields": {"title": "X"}}))
        .await
        .assert_detail(401, "Not signed in");
}

/// Edition corrections go through the same queue; approving writes the edition's fields.
#[sqlx::test]
async fn edition_edits_wait_for_an_admin(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let (work, edition) = app
        .dnb_work("Der Hobbit", "J. R. R. Tolkien", "9783608938289")
        .await;
    let path = format!("/api/books/editions/{}/edit-requests", edition.id);
    ada.post(&path, json!({"fields": {"format": "scroll"}}))
        .await
        .assert_detail(400, "Invalid value for format");
    ada.post(&path, json!({"fields": {"year": 2001}}))
        .await
        .assert_detail(400, "Unknown field: year");
    let res = ada
        .post(
            &path,
            json!({"fields": {"publisher": "Klett-Cotta", "format": "hardcover", "pages": 380,
                              "planned_publication_date": "2026-11-01"}}),
        )
        .await;
    res.assert_status(201);
    let id = res.json()["id"].as_i64().unwrap();
    let queue = root.get("/api/admin/edit-requests").await.json();
    assert_eq!(queue["items"][0]["kind"], "edition_edit");
    assert_eq!(queue["items"][0]["work_id"], work.id);
    assert_eq!(queue["items"][0]["edition_id"], edition.id);
    root.post(&format!("/api/admin/edit-requests/{id}/approve"), json!({}))
        .await
        .assert_status(200);
    let out = ada
        .get(&format!("/api/books/editions/{}", edition.id))
        .await
        .json();
    assert_eq!(out["publisher"], "Klett-Cotta");
    assert_eq!(out["format"], "hardcover");
    assert_eq!(out["pages"], 380);
    assert_eq!(out["planned_publication_date"], "2026-11-01");
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM editions WHERE id = {} AND field_sources ->> 'publisher' = 'admin'",
            edition.id
        ))
        .await,
        1
    );
}

/// Two open requests on one work and edition (a correction of the edition and a photo for it)
/// both carry the book, not only the first one listed.
#[sqlx::test]
async fn requests_on_the_same_edition_both_carry_the_book(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let (work, edition) = app
        .dnb_work("Der Hobbit", "J. R. R. Tolkien", "9783608938289")
        .await;
    let correction = ada
        .post(
            &format!("/api/books/editions/{}/edit-requests", edition.id),
            json!({"fields": {"publisher": "Klett-Cotta"}}),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    let photo = upload_media(&app, &ada).await;
    let cover = cover_request(&app, &ada, work.id, Some(edition.id), photo).await;

    let queue = root.get("/api/admin/edit-requests").await.json();
    let items = queue["items"].as_array().unwrap();
    let ids: Vec<_> = items.iter().map(|item| item["id"].as_i64()).collect();
    assert_eq!(ids, [Some(correction), Some(cover)]);
    for item in items {
        assert_eq!(item["work_id"], work.id);
        assert_eq!(item["edition_id"], edition.id);
        assert_eq!(item["book"]["id"], work.id, "request {}", item["id"]);
        assert_eq!(
            item["book"]["title"], "Der Hobbit",
            "request {}",
            item["id"]
        );
    }
}

/// Rejecting keeps the catalog as it is and records the admin's note.
#[sqlx::test]
async fn rejecting_changes_nothing(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let work = app.work("My Zine").await;
    let id = ada
        .post(
            &format!("/api/books/works/{}/edit-requests", work.id),
            json!({"fields": {"title": "Spam"}}),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    root.post(
        &format!("/api/admin/edit-requests/{id}/reject"),
        json!(null),
    )
    .await
    .assert_status(400);
    let res = root
        .post(
            &format!("/api/admin/edit-requests/{id}/reject"),
            json!({"note": "  Not an improvement  "}),
        )
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["status"], "rejected");
    assert_eq!(res.json()["decision_note"], "Not an improvement");
    assert_eq!(res.json()["decided_by"]["username"], "root");
    let detail = ada
        .get(&format!("/api/books/works/{}", work.id))
        .await
        .json();
    assert_eq!(detail["title"], "My Zine");
    let queue = root.get("/api/admin/edit-requests").await.json();
    assert_eq!(queue["items"], json!([]));
}

/// A `cover_replace` (filed by MED when a cover exists) becomes the admin's cover when
/// approved; a picture taken down meanwhile cannot be approved.
#[sqlx::test]
async fn approving_a_cover_replacement(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let (work, edition) = app
        .dnb_work("Der Hobbit", "J. R. R. Tolkien", "9783608938289")
        .await;
    let first = upload_media(&app, &ada).await;
    app.exec(&format!(
        "UPDATE editions SET cover_media_id = {first},
                field_sources = field_sources || '{{\"cover_media_id\": \"user\"}}'
         WHERE id = {}",
        edition.id
    ))
    .await;
    let photo = upload_media(&app, &ada).await;
    let id = cover_request(&app, &ada, work.id, Some(edition.id), photo).await;
    let queue = root.get("/api/admin/edit-requests").await.json();
    let listed = &queue["items"][0];
    assert_eq!(listed["kind"], "cover_replace");
    assert!(listed["cover_url"].as_str().unwrap().starts_with("/media/"));
    assert_eq!(listed["payload"], json!({}));
    root.post(&format!("/api/admin/edit-requests/{id}/approve"), json!({}))
        .await
        .assert_status(200);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM editions WHERE id = {} AND cover_media_id = {photo}
               AND field_sources ->> 'cover_media_id' = 'admin'",
            edition.id
        ))
        .await,
        1
    );

    let removed = upload_media(&app, &ada).await;
    let gone = cover_request(&app, &ada, work.id, None, removed).await;
    app.exec(&format!(
        "UPDATE media SET removed_at = now() WHERE id = {removed}"
    ))
    .await;
    root.post(
        &format!("/api/admin/edit-requests/{gone}/approve"),
        json!({}),
    )
    .await
    .assert_detail(400, "This picture was removed");
    let queue = root.get("/api/admin/edit-requests").await.json();
    assert_eq!(queue["items"][0]["id"], gone);
    assert_eq!(queue["items"][0]["cover_url"], Value::Null);
}

/// "These two are the same book": a merge request names both works; approving merges them.
#[sqlx::test]
async fn merge_requests_and_their_approval(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let (dnb, _) = app
        .dnb_work("Circe", "Madeline Miller", "9783423282710")
        .await;
    let ol = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let other = app.work("Other").await;

    let path = format!("/api/books/works/{}/merge-requests", dnb.id);
    ada.post(&path, json!({"into_work_id": dnb.id}))
        .await
        .assert_detail(400, "A book cannot be merged into itself");
    ada.post(&path, json!({"into_work_id": 424242}))
        .await
        .assert_detail(404, "No such book.");
    ada.post(
        "/api/books/works/424242/merge-requests",
        json!({"into_work_id": ol.id}),
    )
    .await
    .assert_detail(404, "No such book.");
    ada.post(&path, json!({"into_work_id": "OL1W"}))
        .await
        .assert_detail(400, "Input should be a valid integer");
    assert_eq!(request_count(&app).await, 0);

    let res = ada
        .post(&path, json!({"into_work_id": ol.id, "note": "Same book"}))
        .await;
    res.assert_status(201);
    let id = res.json()["id"].as_i64().unwrap();
    // An edit request on the work that goes away is rejected by the merge.
    let edit = ada
        .post(
            &format!("/api/books/works/{}/edit-requests", dnb.id),
            json!({"fields": {"subtitle": "Roman"}}),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    // A request to merge another book into the merged one now names the winner.
    let into_loser = ada
        .post(
            &format!("/api/books/works/{}/merge-requests", other.id),
            json!({"into_work_id": dnb.id}),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    let queue = root.get("/api/admin/edit-requests").await.json();
    assert_eq!(queue["items"][0]["kind"], "work_merge");
    assert_eq!(queue["items"][0]["payload"], json!({"into_work_id": ol.id}));

    let approved = root
        .post(&format!("/api/admin/edit-requests/{id}/approve"), json!({}))
        .await;
    approved.assert_status(200);
    assert_eq!(approved.json()["status"], "approved");
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT merged_into_id FROM works WHERE id = {}",
            dnb.id
        ))
        .await,
        ol.id
    );
    let detail = ada
        .get(&format!("/api/books/works/{}", dnb.id))
        .await
        .json();
    assert_eq!(detail["id"], ol.id);
    assert_eq!(detail["editions_count"], 1);

    let queue = root.get("/api/admin/edit-requests").await.json();
    let items = queue["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], into_loser);
    assert_eq!(items[0]["payload"], json!({"into_work_id": ol.id}));
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM catalog_edit_requests WHERE id = {edit} AND status = 'rejected'
               AND decision_note = 'Merged into book {}'",
            ol.id
        ))
        .await,
        1
    );
    // A merge request whose works became one cannot be approved.
    let self_merge = ada
        .post(
            &format!("/api/books/works/{}/merge-requests", other.id),
            json!({"into_work_id": ol.id}),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    root.post(
        &format!("/api/admin/works/{}/merge", other.id),
        json!({"into_work_id": ol.id}),
    )
    .await
    .assert_status(200);
    // Both requests now name "other into ol"; merging `other` away rejected them, as open
    // requests on the work that goes away.
    for id in [into_loser, self_merge] {
        root.post(&format!("/api/admin/edit-requests/{id}/approve"), json!({}))
            .await
            .assert_detail(409, "This request was already decided");
    }
}

/// The direct admin merge answers with the work that stays; the guards and the 400/404s.
#[sqlx::test]
async fn the_direct_admin_merge(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let loser = app.work("Circe (bare)").await;
    let winner = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let path = format!("/api/admin/works/{}/merge", loser.id);

    ada.post(&path, json!({"into_work_id": winner.id}))
        .await
        .assert_detail(404, "Not found");
    app.anon()
        .post(&path, json!({"into_work_id": winner.id}))
        .await
        .assert_detail(401, "Not signed in");
    root.post(&path, json!({"into_work_id": loser.id}))
        .await
        .assert_detail(400, "A book cannot be merged into itself");
    root.post(&path, json!({"into_work_id": 424242}))
        .await
        .assert_detail(404, "No such book.");
    root.post(
        "/api/admin/works/424242/merge",
        json!({"into_work_id": winner.id}),
    )
    .await
    .assert_detail(404, "No such book.");

    let res = root.post(&path, json!({"into_work_id": winner.id})).await;
    res.assert_status(200);
    assert_eq!(res.json()["id"], winner.id);
    assert_eq!(res.json()["title"], "Circe");
    // Merging the merged-away work again names the same work twice.
    root.post(&path, json!({"into_work_id": winner.id}))
        .await
        .assert_detail(400, "A book cannot be merged into itself");
    ada.get("/api/admin/edit-requests")
        .await
        .assert_detail(404, "Not found");
}

/// Edit, edition and merge requests share 50 per day.
#[sqlx::test]
async fn requests_share_a_daily_limit(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let work = app.work("My Zine").await;
    let other = app.work("Other").await;
    let edition = app.edition(&work, "9780316556347").await;
    for i in 0..48 {
        ada.post(
            &format!("/api/books/works/{}/edit-requests", work.id),
            json!({"fields": {"subtitle": format!("Part {i}")}}),
        )
        .await
        .assert_status(201);
    }
    ada.post(
        &format!("/api/books/editions/{}/edit-requests", edition.id),
        json!({"fields": {"publisher": "Self"}}),
    )
    .await
    .assert_status(201);
    ada.post(
        &format!("/api/books/works/{}/merge-requests", work.id),
        json!({"into_work_id": other.id}),
    )
    .await
    .assert_status(201);
    let limited = ada
        .post(
            &format!("/api/books/works/{}/merge-requests", other.id),
            json!({"into_work_id": work.id}),
        )
        .await;
    limited.assert_detail(429, "Slow down a little. Try again later.");
    assert_eq!(request_count(&app).await, 50);
}

/// Waits until a session of this test's database waits for a row lock that the test holds.
async fn until_a_lock_wait(db: &PgPool) {
    for _ in 0..500 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(db)
        .await
        .unwrap();
        if waiting > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("nothing waited for a lock");
}

/// A takedown that runs while a `cover_replace` is approved: the approval locks the picture
/// before any catalog row, waits for the takedown, sees the removal and answers 400 `This
/// picture was removed`; the request stays open and no cover is set.
#[sqlx::test]
async fn a_takedown_during_a_cover_approval_keeps_the_request_open(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let work = app.work("Der Hobbit").await;
    let photo = upload_media(&app, &ada).await;
    let id = cover_request(&app, &ada, work.id, None, photo).await;

    // MED's takedown, half done: the picture is marked removed, not yet cleared.
    let mut takedown = app.db.begin().await.unwrap();
    sqlx::query("UPDATE media SET removed_at = now() WHERE id = $1")
        .bind(photo)
        .execute(&mut *takedown)
        .await
        .unwrap();
    let finish_takedown = async {
        until_a_lock_wait(&app.db).await;
        ingest::clear_media(&mut takedown, photo).await.unwrap();
        takedown.commit().await.unwrap();
    };
    let path = format!("/api/admin/edit-requests/{id}/approve");
    let (res, ()) = tokio::join!(root.post(&path, json!({})), finish_takedown);
    res.assert_detail(400, "This picture was removed");
    let status: String =
        sqlx::query_scalar("SELECT status FROM catalog_edit_requests WHERE id = $1")
            .bind(id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(status, "open");
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM works WHERE id = {} AND cover_media_id IS NULL",
            work.id
        ))
        .await,
        1
    );
}

/// A `cover_replace` whose picture is not a cover (set_cover refuses it) is 400 too, and the
/// request stays open; approving a picture the book already shows is fine.
#[sqlx::test]
async fn a_cover_approval_that_sets_nothing_is_a_400(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let work = app.work("Der Hobbit").await;
    let avatar = upload_media(&app, &ada).await;
    app.exec(&format!(
        "UPDATE media SET kind = 'avatar' WHERE id = {avatar}"
    ))
    .await;
    let id = cover_request(&app, &ada, work.id, None, avatar).await;
    root.post(&format!("/api/admin/edit-requests/{id}/approve"), json!({}))
        .await
        .assert_detail(400, "This picture was removed");
    let open: i64 = app
        .scalar_i64(&format!(
            "SELECT count(*) FROM catalog_edit_requests WHERE id = {id} AND status = 'open'"
        ))
        .await;
    assert_eq!(open, 1);

    let photo = upload_media(&app, &ada).await;
    app.exec(&format!(
        "UPDATE works SET cover_media_id = {photo} WHERE id = {}",
        work.id
    ))
    .await;
    let same = cover_request(&app, &ada, work.id, None, photo).await;
    root.post(
        &format!("/api/admin/edit-requests/{same}/approve"),
        json!({}),
    )
    .await
    .assert_status(200);
}

/// Two admins merge at once. The direct merge checks its pair without a lock; when the other
/// merge commits first, `merge_works` finds a work merged away under its lock, and the answer is
/// the 4xx the pair has become, not a 500: B into A after A into B is 400 `A book cannot be
/// merged into itself`, A into C after A into B is 404 `No such book.`.
#[sqlx::test]
async fn a_merge_that_lost_a_race_is_a_4xx(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    for (round, (into_b_first, expected)) in [
        (true, (400, "A book cannot be merged into itself")),
        (false, (404, "No such book.")),
    ]
    .into_iter()
    .enumerate()
    {
        let a = app.work(&format!("Circe (bare {round})")).await.id;
        let b = app.work(&format!("Circe {round}")).await.id;
        let c = app.work(&format!("Circe (other {round})")).await.id;
        // The other admin's merge of A into B holds both works.
        let mut first = app.db.begin().await.unwrap();
        merge_works(&mut first, a, b).await.unwrap();
        let finish_first = async {
            until_a_lock_wait(&app.db).await;
            first.commit().await.unwrap();
        };
        let (loser, into) = if into_b_first { (b, a) } else { (a, c) };
        let path = format!("/api/admin/works/{loser}/merge");
        let (res, ()) = tokio::join!(
            root.post(&path, json!({"into_work_id": into})),
            finish_first
        );
        res.assert_detail(expected.0, expected.1);
        assert_eq!(
            app.scalar_i64(&format!(
                "SELECT count(*) FROM works WHERE merged_into_id IS NOT NULL AND id IN ({a}, {b}, {c})"
            ))
            .await,
            1,
            "only the first merge happened"
        );
    }
}

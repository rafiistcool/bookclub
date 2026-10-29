//! The admin side of the cover search (#74, docs/phase2/notes/cover.md §5.1): the suggestion
//! list, approve and reject, the books without a cover, "Search now", the direct cover of a
//! work or an edition, and the summary counts.

mod common;
#[path = "common_cover/mod.rs"]
mod cover;

use axum::http::Method;
use bookclub_server::catalog::shared::ingest::{CoverTarget, Source};
use bookclub_server::media::pipeline::MAX_UPLOAD_BYTES;
use bookclub_server::media::service::CoverOutcome;
use bookclub_server::outbound::RetryPolicy;
use common::*;
use cover::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// A work with an open suggestion of `OL21W` (cover 2100) for it.
async fn suggested(app: &TestApp, title: &str) -> (i64, i64) {
    let work = legacy_work(app, &format!("{title} - Band zwei"), "Jeff Kinney").await;
    app.catalog.on_covers(
        title,
        Some("Kinney"),
        answer(vec![doc(21, title, "", "Jeff Kinney", Some(2100), 80)]),
    );
    run_job(app).await;
    let id = app
        .scalar_i64(&format!(
            "SELECT id FROM cover_suggestions WHERE work_id = {work}"
        ))
        .await;
    (work, id)
}

fn picture(bytes: Vec<u8>, edition: Option<i64>) -> Vec<Part> {
    let mut parts = vec![Part::file("cover.jpg", "image/jpeg", bytes)];
    if let Some(edition) = edition {
        parts.push(Part::text("edition_id", &edition.to_string()));
    }
    parts
}

#[sqlx::test]
async fn suggestions_list_open_and_decided(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let (work, id) = suggested(&app, "Gregs Tagebuch").await;
    let page = root.get("/api/admin/cover-suggestions").await;
    page.assert_status(200);
    let item = &page.json()["items"][0];
    assert_eq!(item["id"], id);
    assert_eq!(item["status"], "open");
    assert_eq!(item["work_id"], work);
    assert_eq!(item["book"]["id"], work);
    assert_eq!(item["book"]["cover_url"], Value::Null);
    assert_eq!(item["ol_work_id"], "OL21W");
    assert_eq!(item["title"], "Gregs Tagebuch");
    assert_eq!(item["authors"], "Jeff Kinney");
    assert_eq!(item["reason"], "title_differs");
    let cover_url = item["cover_url"].as_str().unwrap();
    assert!(cover_url.starts_with("/media/") && cover_url.ends_with("/l"));
    assert_eq!(page.json()["has_more"], false);
    assert_eq!(
        root.get("/api/admin/cover-suggestions?status=resolved")
            .await
            .json()["items"],
        json!([])
    );
    root.get("/api/admin/cover-suggestions?status=maybe")
        .await
        .assert_detail(400, "Input should be 'open' or 'resolved'");

    root.post(
        &format!("/api/admin/cover-suggestions/{id}/reject"),
        json!({}),
    )
    .await
    .assert_status(200);
    let decided = root
        .get("/api/admin/cover-suggestions?status=resolved")
        .await
        .json();
    assert_eq!(decided["items"][0]["status"], "rejected");
    assert!(decided["items"][0]["decided_at"].is_string());
}

#[sqlx::test]
async fn approve_and_reject_answer_with_their_errors(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let (work, id) = suggested(&app, "Gregs Tagebuch").await;
    let approve = format!("/api/admin/cover-suggestions/{id}/approve");
    let reject = format!("/api/admin/cover-suggestions/{id}/reject");
    root.post("/api/admin/cover-suggestions/999999/approve", json!({}))
        .await
        .assert_detail(404, "Not found");
    root.post("/api/admin/cover-suggestions/999999/reject", json!({}))
        .await
        .assert_detail(404, "Not found");

    // The book got a cover meanwhile: 409, and the suggestion stays open.
    let photo = upload_media(&app).await;
    set_cover(&app, CoverTarget::Work(work), photo, Source::User).await;
    root.post(&approve, json!({}))
        .await
        .assert_detail(409, "This book has a cover now");
    app.exec(&format!(
        "UPDATE works SET cover_media_id = NULL, field_sources = '{{}}' WHERE id = {work}"
    ))
    .await;

    // Open Library removed the picture: 400.
    app.exec("UPDATE media SET ol_gone_at = now() WHERE ol_cover_id = 2100")
        .await;
    root.post(&approve, json!({}))
        .await
        .assert_detail(400, "This picture was removed");
    app.exec("UPDATE media SET ol_gone_at = NULL WHERE ol_cover_id = 2100")
        .await;

    let res = root.post(&approve, json!({})).await;
    res.assert_status(200);
    assert_eq!(res.json()["status"], "approved");
    assert_eq!(
        res.json()["book"]["cover_public_id"],
        res.json()["cover_public_id"]
    );
    assert_eq!(cover_of(&app, work).await, Some((2100, "admin".into())));
    root.post(&approve, json!({}))
        .await
        .assert_detail(409, "This request was already decided");
    root.post(&reject, json!({}))
        .await
        .assert_detail(409, "This request was already decided");
}

#[sqlx::test]
async fn the_books_without_a_cover_with_their_states(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let reader = app.user("ada").await;
    // In use, never searched.
    let plain = legacy_work(&app, "Zebra", "Anna Autorin").await;
    // In use, a reader's photo waits.
    let waiting = legacy_work(&app, "Apfel", "Anna Autorin").await;
    let photo = upload_media(&app).await;
    app.exec(&format!(
        "INSERT INTO catalog_edit_requests (kind, work_id, media_id, created_by)
         VALUES ('cover_replace', {waiting}, {photo}, {})",
        reader.id()
    ))
    .await;
    // In use, its Open Library picture failed to load.
    let failing = legacy_work(&app, "Mango", "Anna Autorin").await;
    let media = ol_media(&app, 901).await;
    set_cover(&app, CoverTarget::Work(failing), media, Source::OpenLibrary).await;
    app.exec(&format!(
        "UPDATE media SET fetch_failed_at = now() WHERE id = {media}"
    ))
    .await;
    // Not in use: only with scope=all.
    let (unused, _) = app.dnb_work("Birne", "Anna Autorin", "9783161484100").await;
    // A reader's own book and a book with a cover: never listed.
    let readers = app.work("Mein Buch").await;
    use_work(&app, readers.id).await;
    let covered = legacy_work(&app, "Kirsche", "Anna Autorin").await;
    set_cover(
        &app,
        CoverTarget::Work(covered),
        upload_media(&app).await,
        Source::User,
    )
    .await;
    // A suggestion and a search outcome.
    let (suggested_work, suggestion) = suggested(&app, "Gregs Tagebuch").await;

    let page = root.get("/api/admin/missing-covers").await;
    page.assert_status(200);
    let items = page.json()["items"].as_array().unwrap().clone();
    let ids: Vec<i64> = items
        .iter()
        .map(|i| i["book"]["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, [waiting, suggested_work, failing, plain], "by title");
    let by_id = |id: i64| {
        items
            .iter()
            .find(|i| i["book"]["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(by_id(waiting)["state"], "photo_waiting");
    assert_eq!(by_id(failing)["state"], "failing");
    assert!(by_id(failing)["failed_at"].is_string());
    assert_eq!(by_id(plain)["state"], "none");
    assert_eq!(by_id(plain)["imported"], true);
    assert_eq!(by_id(plain)["in_use"], true);
    assert_eq!(by_id(suggested_work)["suggestion_id"], suggestion);
    assert_eq!(by_id(suggested_work)["outcome"], "suggested");
    assert!(by_id(suggested_work)["searched_at"].is_string());

    let all = root.get("/api/admin/missing-covers?scope=all").await.json();
    let ids: Vec<i64> = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["book"]["id"].as_i64().unwrap())
        .collect();
    assert_eq!(
        ids,
        [waiting, suggested_work, failing, plain, unused.id],
        "in use first"
    );
    root.get("/api/admin/missing-covers?scope=mine")
        .await
        .assert_detail(400, "Input should be 'in_use' or 'all'");
    root.get("/api/admin/missing-covers?page=0")
        .await
        .assert_detail(400, "Input should be greater than or equal to 1");
}

#[sqlx::test]
async fn the_missing_list_has_pages(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    for n in 0..51 {
        legacy_work(&app, &format!("Buch {n:02}"), "Anna Autorin").await;
    }
    let first = root.get("/api/admin/missing-covers").await.json();
    assert_eq!(first["items"].as_array().unwrap().len(), 50);
    assert_eq!(first["has_more"], true);
    let second = root.get("/api/admin/missing-covers?page=2").await.json();
    assert_eq!(second["items"].as_array().unwrap().len(), 1);
    assert_eq!(second["items"][0]["book"]["title"], "Buch 50");
    assert_eq!(second["has_more"], false);
}

#[sqlx::test]
async fn search_now_answers_with_its_outcome(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let clear = legacy_work(&app, "Momo", "Michael Ende").await;
    app.catalog.on_covers(
        "Momo",
        Some("Ende"),
        answer(vec![doc(11, "Momo", "", "Michael Ende", Some(1100), 30)]),
    );
    let res = root
        .post(&format!("/api/admin/works/{clear}/cover-search"), json!({}))
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["outcome"], "attached");
    assert!(res.json()["book"]["cover_url"].is_string());
    assert_eq!(res.json()["suggestion"], Value::Null);
    root.post(&format!("/api/admin/works/{clear}/cover-search"), json!({}))
        .await
        .assert_detail(409, "This book has a cover now");

    // A book nobody uses gets a suggestion too when the admin asks.
    let (unused, _) = app
        .dnb_work("Gregs Tagebuch - Band zwei", "Jeff Kinney", "9783161484100")
        .await;
    app.catalog.on_covers(
        "Gregs Tagebuch",
        Some("Kinney"),
        answer(vec![doc(
            21,
            "Gregs Tagebuch",
            "",
            "Jeff Kinney",
            Some(2100),
            80,
        )]),
    );
    let res = root
        .post(
            &format!("/api/admin/works/{}/cover-search", unused.id),
            json!({}),
        )
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["outcome"], "suggested");
    let suggestion = res.json()["suggestion"]["id"].as_i64().unwrap();
    // An open suggestion is returned as it is, without a search.
    let calls = app.catalog.cover_calls().len();
    let res = root
        .post(
            &format!("/api/admin/works/{}/cover-search", unused.id),
            json!({}),
        )
        .await;
    assert_eq!(res.json()["suggestion"]["id"], suggestion);
    assert_eq!(app.catalog.cover_calls().len(), calls);

    let none = legacy_work(&app, "Nichts", "Anna Autorin").await;
    let res = root
        .post(&format!("/api/admin/works/{none}/cover-search"), json!({}))
        .await;
    assert_eq!(res.json()["outcome"], "no_match");
    root.post("/api/admin/works/999999/cover-search", json!({}))
        .await
        .assert_detail(404, "No such book.");

    app.catalog.fail_covers(FakeError::Unavailable, 1);
    root.post(&format!("/api/admin/works/{none}/cover-search"), json!({}))
        .await
        .assert_detail(502, "Could not search the library right now. Try again.");
    app.catalog.fail_covers(FakeError::Throttled, 1);
    root.post(&format!("/api/admin/works/{none}/cover-search"), json!({}))
        .await
        .assert_detail(
            429,
            "The library is busy. Wait a few seconds and try again.",
        );
}

/// "Search now" fetches a failing picture of the book at once: gone, it is cleared and the
/// book searched.
#[sqlx::test]
async fn search_now_rechecks_a_failing_picture(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    serve_cover(&app, 999, 200, jpeg([10, 200, 10])).await;
    let known = ol_media(&app, 999).await;
    assert_eq!(
        app.state
            .media_service
            .ensure_cover(&app.db, known, RetryPolicy::MEDIA)
            .await,
        CoverOutcome::Stored
    );
    let work = legacy_work(&app, "Momo", "Michael Ende").await;
    let failing = ol_media(&app, 301).await;
    set_cover(&app, CoverTarget::Work(work), failing, Source::OpenLibrary).await;
    app.exec(&format!(
        "UPDATE media SET fetch_failed_at = now() WHERE id = {failing}"
    ))
    .await;
    serve_cover(&app, 301, 404, Vec::new()).await;
    app.catalog.on_covers(
        "Momo",
        Some("Ende"),
        answer(vec![doc(11, "Momo", "", "Michael Ende", Some(1100), 30)]),
    );
    let res = root
        .post(&format!("/api/admin/works/{work}/cover-search"), json!({}))
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["outcome"], "attached");
    assert_eq!(cover_of(&app, work).await.unwrap().0, 1100);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM media WHERE id = {failing} AND ol_gone_at IS NOT NULL"
        ))
        .await,
        1
    );
}

/// "Search now" on a book whose picture still fails after the recheck (a body that does not
/// decode; an edition's 404 while the probe fails) answers `failing` with the time and the
/// edition, not "This book has a cover now": readers still see the cloth.
#[sqlx::test]
async fn search_now_says_when_a_picture_still_fails(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let own = legacy_work(&app, "Momo", "Michael Ende").await;
    let media = ol_media(&app, 302).await;
    set_cover(&app, CoverTarget::Work(own), media, Source::OpenLibrary).await;
    app.exec(&format!(
        "UPDATE media SET fetch_failed_at = now() - interval '2 days' WHERE id = {media}"
    ))
    .await;
    serve_cover(&app, 302, 200, b"not a picture".to_vec()).await;
    let res = root
        .post(&format!("/api/admin/works/{own}/cover-search"), json!({}))
        .await;
    res.assert_status(200);
    let out = res.json();
    assert_eq!(out["outcome"], "failing");
    assert_eq!(out["failing_edition_id"], Value::Null);
    assert!(out["failed_at"].is_string());
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM media
             WHERE id = {media} AND fetch_failed_at > now() - interval '1 minute'"
        ))
        .await,
        1,
        "fetched again just now"
    );
    assert!(app.catalog.cover_calls().is_empty(), "nothing searched");

    // A covered book whose edition readers use has a picture Open Library answers 404 for,
    // with no stored picture to probe with: it stays failing.
    let work = app.ol_work("OL31W", "Circe", "Madeline Miller").await;
    use_work(&app, work.id).await;
    set_cover(
        &app,
        CoverTarget::Work(work.id),
        upload_media(&app).await,
        Source::User,
    )
    .await;
    let edition = app.edition(&work, "9780000000019").await;
    let edition_media = ol_media(&app, 303).await;
    set_cover(
        &app,
        CoverTarget::Edition(edition.id),
        edition_media,
        Source::OpenLibrary,
    )
    .await;
    app.exec(&format!(
        "UPDATE media SET fetch_failed_at = now() WHERE id = {edition_media};
         INSERT INTO legacy_work_keys (legacy_key, legacy_book_id, work_id, edition_id, match)
         VALUES ('/works/GBcirce', 777001, {}, {}, 'google_volume')",
        work.id, edition.id
    ))
    .await;
    serve_cover(&app, 303, 404, Vec::new()).await;
    let res = root
        .post(
            &format!("/api/admin/works/{}/cover-search", work.id),
            json!({}),
        )
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["outcome"], "failing");
    assert_eq!(res.json()["failing_edition_id"], edition.id);
    assert!(res.json()["failed_at"].is_string());
}

/// The list names the edition readers use whose picture fails, not a lower-id edition nobody
/// uses; a book under takedown says so.
#[sqlx::test]
async fn the_missing_list_names_the_failing_edition_readers_use(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let work = app.ol_work("OL32W", "Circe", "Madeline Miller").await;
    use_work(&app, work.id).await;
    set_cover(
        &app,
        CoverTarget::Work(work.id),
        upload_media(&app).await,
        Source::User,
    )
    .await;
    let unused = app.edition(&work, "9780000000026").await;
    let used = app.edition(&work, "9780000000033").await;
    assert!(unused.id < used.id);
    for (edition, cover) in [(unused.id, 311), (used.id, 312)] {
        let media = ol_media(&app, cover).await;
        set_cover(
            &app,
            CoverTarget::Edition(edition),
            media,
            Source::OpenLibrary,
        )
        .await;
        app.exec(&format!(
            "UPDATE media SET fetch_failed_at = now() WHERE id = {media}"
        ))
        .await;
    }
    app.exec(&format!(
        "INSERT INTO legacy_work_keys (legacy_key, legacy_book_id, work_id, edition_id, match)
         VALUES ('/works/GBused', 777002, {}, {}, 'google_volume')",
        work.id, used.id
    ))
    .await;
    // A book whose cover a takedown cleared.
    let taken = legacy_work(&app, "Apfel", "Anna Autorin").await;
    app.exec(&format!(
        "INSERT INTO cover_search (work_id, takedown_at) VALUES ({taken}, now())"
    ))
    .await;

    let items = root.get("/api/admin/missing-covers").await.json()["items"]
        .as_array()
        .unwrap()
        .clone();
    let row = |id: i64| {
        items
            .iter()
            .find(|i| i["book"]["id"] == id)
            .unwrap_or_else(|| panic!("{id} is listed"))
            .clone()
    };
    assert_eq!(row(work.id)["state"], "failing");
    assert_eq!(row(work.id)["failing_edition_id"], used.id);
    assert_eq!(row(work.id)["after_takedown"], false);
    assert_eq!(row(taken)["state"], "none");
    assert_eq!(row(taken)["after_takedown"], true);
    // The count beside the list is the list's.
    let summary = root.get("/api/admin/summary").await.json();
    assert_eq!(summary["covers"]["missing"], items.len());
}

/// The scans of the job, the list and the summary use the partial index on works without a
/// cover, and call no function per work (`bc_cover_missing` is not inlined by Postgres).
#[sqlx::test]
async fn the_scans_use_the_index_of_works_without_a_cover(db: PgPool) {
    let app = TestApp::new(db).await;
    let mut conn = app.db.acquire().await.unwrap();
    sqlx::raw_sql("SET enable_seqscan = off")
        .execute(&mut *conn)
        .await
        .unwrap();
    for sql in [
        "EXPLAIN SELECT work_id FROM cover_missing",
        "EXPLAIN SELECT count(*) FROM cover_gaps WHERE in_use",
        "EXPLAIN SELECT work_id FROM cover_gaps ORDER BY in_use DESC, title, work_id",
    ] {
        let plan: Vec<String> = sqlx::query_scalar(sql).fetch_all(&mut *conn).await.unwrap();
        let plan = plan.join("\n");
        assert!(plan.contains("works_cover_missing_idx"), "{sql}:\n{plan}");
        assert!(!plan.contains("bc_cover_"), "{sql}:\n{plan}");
    }
}

#[sqlx::test]
async fn an_admin_sets_any_cover_directly(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    // Replaces a reader's cover at once.
    let work = legacy_work(&app, "Momo", "Michael Ende").await;
    set_cover(
        &app,
        CoverTarget::Work(work),
        upload_media(&app).await,
        Source::User,
    )
    .await;
    let res = root
        .put_multipart(
            &format!("/api/admin/works/{work}/cover"),
            picture(jpeg([200, 10, 10]), None),
        )
        .await;
    res.assert_status(200);
    assert!(res.json()["book"]["cover_url"].is_string());
    let (cover, by) = cover_of(&app, work).await.unwrap();
    assert_eq!((cover, by.as_str()), (0, "admin"));

    // An edition's cover, also of a reader's own book.
    let readers = app.work("Mein Buch").await;
    let edition = app.edition(&readers, "9780316556347").await;
    let res = root
        .put_multipart(
            &format!("/api/admin/works/{}/cover", readers.id),
            picture(jpeg([10, 200, 10]), Some(edition.id)),
        )
        .await;
    res.assert_status(200);
    let edition_cover: Option<String> =
        sqlx::query_scalar("SELECT field_sources ->> 'cover_media_id' FROM editions WHERE id = $1")
            .bind(edition.id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(edition_cover.as_deref(), Some("admin"));
    assert!(res.json()["book"]["cover_url"].is_string());

    // A taken-down picture is refused.
    let public_id = res.json()["book"]["cover_public_id"]
        .as_str()
        .unwrap()
        .to_owned();
    root.delete(&format!("/api/admin/media/{public_id}"))
        .await
        .assert_status(204);
    root.put_multipart(
        &format!("/api/admin/works/{}/cover", readers.id),
        picture(jpeg([10, 200, 10]), Some(edition.id)),
    )
    .await
    .assert_detail(400, "This picture was removed");

    // The upload checks and limits of a reader's photo.
    root.put_multipart(
        "/api/admin/works/999999/cover",
        picture(jpeg([1, 1, 1]), None),
    )
    .await
    .assert_detail(404, "No such book.");
    root.put_multipart(
        &format!("/api/admin/works/{work}/cover"),
        vec![Part::file("cover.txt", "text/plain", b"hello".to_vec())],
    )
    .await
    .assert_detail(400, "Use a JPEG, PNG, WebP, or GIF");
    let res = root
        .clone()
        .with_header("content-length", &(MAX_UPLOAD_BYTES + 4096 + 1).to_string())
        .request(
            Method::PUT,
            &format!("/api/admin/works/{work}/cover"),
            Some("multipart/form-data; boundary=x"),
            b"ignored".to_vec(),
        )
        .await;
    res.assert_detail(413, "That picture is too large (50 MB max)");
    let other = legacy_work(&app, "Circe", "Madeline Miller").await;
    root.put_multipart(
        &format!("/api/admin/works/{other}/cover"),
        picture(jpeg([3, 3, 3]), Some(edition.id)),
    )
    .await
    .assert_detail(400, "That edition belongs to another book");
}

/// A private quote or diary entry (`only_me`) does not make a book "in use": an instance admin
/// sees that flag, and it must not tell them that someone reads the book (conventions §6.2).
/// The same items with `audience` do count.
#[sqlx::test]
async fn private_quotes_and_diary_entries_do_not_mark_a_book_in_use(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    app.user("bea").await;
    let bea = app
        .scalar_i64("SELECT id FROM users WHERE username = 'bea'")
        .await;
    let (quoted, _) = app.dnb_work("Birne", "Anna Autorin", "9783161484100").await;
    let (noted, _) = app.dnb_work("Apfel", "Anna Autorin", "9780306406157").await;
    let (shared, _) = app.dnb_work("Quitte", "Anna Autorin", "9781861972712").await;
    app.exec(&format!(
        "INSERT INTO quotes (user_id, work_id, body, visibility)
         VALUES ({bea}, {}, 'Nur für mich', 'only_me'), ({bea}, {}, 'Für alle', 'audience')",
        quoted.id, shared.id
    ))
    .await;
    let root_post = app
        .scalar_i64(&format!(
            "INSERT INTO posts (work_id, author_id, body, visibility)
             VALUES ({}, {bea}, 'Tagebuch', 'only_me') RETURNING id",
            noted.id
        ))
        .await;
    app.exec(&format!(
        "INSERT INTO posts (work_id, author_id, parent_id, body)
         VALUES ({}, {bea}, {root_post}, 'Nachtrag')",
        noted.id
    ))
    .await;

    let in_use: Vec<i64> = root.get("/api/admin/missing-covers").await.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["book"]["id"].as_i64().unwrap())
        .collect();
    assert_eq!(in_use, [shared.id]);
    let all = root.get("/api/admin/missing-covers?scope=all").await.json();
    for item in all["items"].as_array().unwrap() {
        let id = item["book"]["id"].as_i64().unwrap();
        assert_eq!(item["in_use"], id == shared.id, "{item}");
    }
    let summary = root.get("/api/admin/summary").await.json();
    assert_eq!(summary["covers"]["missing"], 1);
}

#[sqlx::test]
async fn the_summary_counts_suggestions_and_books_without_a_cover(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    legacy_work(&app, "Momo", "Michael Ende").await;
    app.dnb_work("Birne", "Anna Autorin", "9783161484100").await;
    suggested(&app, "Gregs Tagebuch").await;
    let summary = root.get("/api/admin/summary").await.json();
    assert_eq!(summary["covers"], json!({"suggestions": 1, "missing": 2}));
    // A book readers use whose picture fails to load is on the list, so it is counted.
    let failing = legacy_work(&app, "Mango", "Anna Autorin").await;
    let media = ol_media(&app, 901).await;
    set_cover(&app, CoverTarget::Work(failing), media, Source::OpenLibrary).await;
    app.exec(&format!(
        "UPDATE media SET fetch_failed_at = now() WHERE id = {media}"
    ))
    .await;
    let summary = root.get("/api/admin/summary").await.json();
    let listed = root.get("/api/admin/missing-covers").await.json()["items"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(summary["covers"]["missing"], 3);
    assert_eq!(listed, 3);
}

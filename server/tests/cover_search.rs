//! The cover search (#74, docs/phase2/notes/cover.md §1, §4): a clear match attaches another
//! edition's cover, an uncertain one waits as a suggestion, readers' and admins' covers and
//! readers' own books are never touched, taken-down and gone pictures never come back, and the
//! background job's schedule, tiers, backoff and stops. Open Library is the `FakeCatalog`
//! (the tests with the real client say so).

mod common;
#[path = "common_cover/mod.rs"]
mod cover;

use std::sync::Arc;
use std::time::{Duration, Instant};

use bookclub_server::catalog::covers::dead;
use bookclub_server::catalog::covers::job::{self, Breaker, JobSettings};
use bookclub_server::catalog::covers::search::Outcome;
use bookclub_server::catalog::merge_works;
use bookclub_server::catalog::shared::books::{BookRef, book_out};
use bookclub_server::catalog::shared::ingest::{self, CoverTarget, Mode, Source, WorkRecord};
use bookclub_server::jobs::lock_ids;
use bookclub_server::media::service::CoverOutcome;
use bookclub_server::media::shared::MediaStore;
use bookclub_server::outbound::RetryPolicy;
use common::*;
use cover::*;
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::Barrier;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

async fn public_id(app: &TestApp, ol_cover_id: i64) -> String {
    sqlx::query_scalar("SELECT public_id FROM media WHERE ol_cover_id = $1")
        .bind(ol_cover_id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// #74 AC 1: a legacy, a DNB and an Open Library work without a cover each get another
/// edition's cover on a clear match, with provenance `openlibrary` on the work and none on our
/// editions. The DNB work's ISBN check runs first and finds nothing, so the title search
/// follows in the same attempt, in the book's language, and takes the German edition's cover
/// over the work's.
#[sqlx::test]
async fn clear_matches_attach_another_editions_cover(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = legacy_work(&app, "Momo", "Michael Ende").await;
    let (dnb, _) = app
        .dnb_work("Der Schwarm", "Frank Schätzing", "9783161484100")
        .await;
    use_work(&app, dnb.id).await;
    let ol = app.ol_work("OL5W", "Circe", "Madeline Miller").await;
    use_work(&app, ol.id).await;
    app.catalog.on_covers(
        "Momo",
        Some("Ende"),
        answer(vec![doc(11, "Momo", "", "Michael Ende", Some(1100), 30)]),
    );
    app.catalog.on_covers(
        "Der Schwarm",
        Some("Schätzing"),
        answer(vec![with_edition(
            doc(12, "The Swarm", "", "Frank Schätzing", Some(1200), 20),
            12,
            "Der Schwarm",
            "Roman",
            Some(1201),
            &["ger"],
        )]),
    );
    app.catalog.on_covers(
        "Circe",
        Some("Miller"),
        answer(vec![doc(
            13,
            "Circe",
            "",
            "Madeline Miller",
            Some(1300),
            50,
        )]),
    );

    let report = run_job(&app).await;
    assert_eq!(report.attached, 3, "{report:?}");
    assert_eq!(
        cover_of(&app, legacy).await,
        Some((1100, "openlibrary".into()))
    );
    assert_eq!(
        cover_of(&app, dnb.id).await,
        Some((1201, "openlibrary".into())),
        "the German edition's cover"
    );
    assert_eq!(
        cover_of(&app, ol.id).await,
        Some((1300, "openlibrary".into()))
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM editions WHERE cover_media_id IS NOT NULL")
            .await,
        0,
        "the cover shows another edition: never one of ours"
    );
    let row = search_row(&app, dnb.id).await.unwrap();
    assert_eq!(row.outcome.as_deref(), Some("attached"));
    assert_eq!(row.attached_cover, Some(1201));
    let calls = app.catalog.cover_calls();
    let isbn = calls.iter().position(|c| c == "isbn_cover:9783161484100");
    let title = calls
        .iter()
        .position(|c| c == "cover:Der Schwarm|Schätzing|de");
    assert!(isbn.unwrap() < title.unwrap(), "{calls:?}");
}

/// A harvested DNB work gets its exact ISBN cover on its edition, without a title search.
#[sqlx::test]
async fn a_dnb_work_gets_its_isbn_cover_first(db: PgPool) {
    let app = TestApp::new(db).await;
    let (work, edition) = app
        .dnb_work("Tannöd", "Andrea Maria Schenkel", "9783161484100")
        .await;
    use_work(&app, work.id).await;
    app.catalog.on_isbn_cover("9783161484100", Some(4242));
    let report = run_job(&app).await;
    assert_eq!(report.isbn, 1);
    assert_eq!(cover_of(&app, work.id).await.unwrap().0, 4242);
    let on_edition: Option<i64> = sqlx::query_scalar(
        "SELECT m.ol_cover_id FROM editions e JOIN media m ON m.id = e.cover_media_id
         WHERE e.id = $1",
    )
    .bind(edition.id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        on_edition,
        Some(4242),
        "the exact picture is the edition's own"
    );
    assert_eq!(app.catalog.cover_calls(), ["isbn_cover:9783161484100"]);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM works WHERE id = {} AND details_fetched_at IS NOT NULL",
            work.id
        ))
        .await,
        1,
        "the first detail view finds the check done"
    );
}

/// #74 AC 2: an uncertain match becomes one suggestion; approving it attaches the picture with
/// provenance `admin`.
#[sqlx::test]
async fn an_uncertain_match_waits_as_one_suggestion(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = legacy_work(&app, "Gregs Tagebuch - Jetzt reicht's!", "Jeff Kinney").await;
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
    let report = run_job(&app).await;
    assert_eq!(report.suggested, 1);
    assert_eq!(
        suggestions(&app, work).await,
        [("OL21W".into(), 2100, "title_differs".into(), "open".into())]
    );
    assert_eq!(cover_of(&app, work).await, None);
    // The open suggestion keeps the book out of the next runs.
    run_job(&app).await;
    assert_eq!(app.catalog.cover_calls().len(), 1);
    assert_eq!(suggestions(&app, work).await.len(), 1);

    let root = app.admin("root").await;
    let id = app.scalar_i64("SELECT id FROM cover_suggestions").await;
    let res = root
        .post(
            &format!("/api/admin/cover-suggestions/{id}/approve"),
            json!({}),
        )
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["status"], "approved");
    assert_eq!(cover_of(&app, work).await, Some((2100, "admin".into())));
    assert_eq!(
        search_row(&app, work).await.unwrap().attached_cover,
        Some(2100)
    );
}

/// #74 AC 2: rejecting keeps that Open Library work out for good: on the next run, on "Search
/// now", and after the book is merged into another, whose open suggestion for the same Open
/// Library work is closed.
#[sqlx::test]
async fn a_rejection_keeps_that_open_library_work_out(db: PgPool) {
    let app = TestApp::new(db).await;
    let first = legacy_work(&app, "Gregs Tagebuch - Jetzt reicht's!", "Jeff Kinney").await;
    let second = legacy_work(&app, "Gregs Tagebuch: Jetzt reicht's", "Jeff Kinney").await;
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
    run_job(&app).await;
    assert_eq!(suggestions(&app, first).await.len(), 1);
    assert_eq!(suggestions(&app, second).await.len(), 1);
    let root = app.admin("root").await;
    let id = app
        .scalar_i64(&format!(
            "SELECT id FROM cover_suggestions WHERE work_id = {first}"
        ))
        .await;
    root.post(
        &format!("/api/admin/cover-suggestions/{id}/reject"),
        json!({}),
    )
    .await
    .assert_status(200);
    assert_eq!(
        search_row(&app, first).await.unwrap().next_hours,
        None,
        "due at once"
    );

    let report = run_job(&app).await;
    assert_eq!(report.works, [first]);
    assert_eq!(report.no_match, 1);
    assert_eq!(suggestions(&app, first).await.len(), 1, "nothing new");
    let res = root
        .post(&format!("/api/admin/works/{first}/cover-search"), json!({}))
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["outcome"], "no_match");

    // Merged into the second book: its open suggestion of the same Open Library work closes,
    // and it is not suggested again.
    let mut conn = app.db.acquire().await.unwrap();
    merge_works(&mut conn, first, second).await.unwrap();
    let report = run_job(&app).await;
    assert_eq!(report.closed, 1);
    assert_eq!(
        suggestions(&app, second).await,
        [(
            "OL21W".into(),
            2100,
            "title_differs".into(),
            "closed".into()
        )]
    );
    assert_eq!(report.works, [second]);
    assert_eq!(report.no_match, 1);
    assert_eq!(cover_of(&app, second).await, None);
}

/// #74 AC 3: a reader's cover, an admin's cover and a reader's own book are never touched
/// (not even searched).
#[sqlx::test]
async fn readers_and_admins_covers_and_readers_books_are_left_alone(db: PgPool) {
    let app = TestApp::new(db).await;
    let users = legacy_work(&app, "Momo", "Michael Ende").await;
    let photo = upload_media(&app).await;
    set_cover(&app, CoverTarget::Work(users), photo, Source::User).await;
    let admins = legacy_work(&app, "Circe", "Madeline Miller").await;
    let picture = upload_media(&app).await;
    set_cover(&app, CoverTarget::Work(admins), picture, Source::Admin).await;
    let readers = app.work("Mein eigenes Buch").await;
    use_work(&app, readers.id).await;
    for (title, author) in [
        ("Momo", "Ende"),
        ("Circe", "Miller"),
        ("Mein eigenes Buch", ""),
    ] {
        let author = (!author.is_empty()).then_some(author);
        app.catalog.on_covers(
            title,
            author,
            answer(vec![doc(31, title, "", "Someone", Some(3100), 5)]),
        );
    }
    let report = run_job(&app).await;
    assert_eq!(report.searched, 0);
    assert!(app.catalog.cover_calls().is_empty());
    assert_eq!(cover_of(&app, users).await, Some((0, "user".into())));
    assert_eq!(cover_of(&app, admins).await, Some((0, "admin".into())));
    assert_eq!(cover_of(&app, readers.id).await, None);
    // Not on an admin's "Search now" either: 404 for a reader's book, 409 for a covered one.
    let root = app.admin("root").await;
    root.post(
        &format!("/api/admin/works/{}/cover-search", readers.id),
        json!({}),
    )
    .await
    .assert_detail(404, "No such book.");
    root.post(&format!("/api/admin/works/{users}/cover-search"), json!({}))
        .await
        .assert_detail(409, "This book has a cover now");
    assert!(app.catalog.cover_calls().is_empty());
}

/// #74 AC 3: a reader's photo of an edition that lands between the remote step and the apply
/// is inherited by the work; the search's picture is not attached.
#[sqlx::test]
async fn a_readers_upload_during_the_search_wins(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = legacy_work(&app, "Momo", "Michael Ende").await;
    let edition = app
        .edition(
            &TestWork {
                id: work,
                title: "Momo".into(),
            },
            "9780316556347",
        )
        .await;
    let photo = upload_media(&app).await;
    let hook = Hook {
        db: app.db.clone(),
        sql: Some(format!(
            "UPDATE editions SET cover_media_id = {photo},
                    field_sources = field_sources || '{{\"cover_media_id\": \"user\"}}'
             WHERE id = {}",
            edition.id
        )),
        barrier: None,
        answer: answer(vec![doc(11, "Momo", "", "Michael Ende", Some(1100), 30)]),
    };
    let outcome = search_with(&app.db, &hook, work, false).await;
    assert_eq!(outcome, Outcome::Inherited);
    assert_eq!(cover_of(&app, work).await, Some((0, "user".into())));
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM media WHERE ol_cover_id = 1100")
            .await,
        1,
        "the picture's row exists, but nothing shows it"
    );
}

/// A taken-down cover id never comes back, and after the takedown of a book's cover the
/// search only suggests for it.
#[sqlx::test]
async fn after_a_takedown_the_search_only_suggests(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = legacy_work(&app, "Momo", "Michael Ende").await;
    app.catalog.on_covers(
        "Momo",
        Some("Ende"),
        answer(vec![with_edition(
            doc(31, "Momo", "", "Michael Ende", Some(3100), 30),
            31,
            "Momo",
            "",
            Some(3101),
            &["eng"],
        )]),
    );
    run_job(&app).await;
    assert_eq!(cover_of(&app, work).await.unwrap().0, 3101);
    let root = app.admin("root").await;
    let taken = public_id(&app, 3101).await;
    root.delete(&format!("/api/admin/media/{taken}"))
        .await
        .assert_status(204);
    assert_eq!(cover_of(&app, work).await, None);
    assert!(search_row(&app, work).await.unwrap().takedown);

    // The same clear match: the taken-down id is out, the other picture of the same record
    // is only suggested.
    let report = run_job(&app).await;
    assert_eq!(report.suggested, 1, "{report:?}");
    assert_eq!(
        suggestions(&app, work).await,
        [("OL31W".into(), 3100, "after_takedown".into(), "open".into())]
    );
    assert_eq!(cover_of(&app, work).await, None);
}

/// A takedown that lands during the remote step turns an attach into a suggestion.
#[sqlx::test]
async fn a_takedown_during_the_search_turns_it_into_a_suggestion(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = legacy_work(&app, "Momo", "Michael Ende").await;
    let hook = Hook {
        db: app.db.clone(),
        sql: Some(format!(
            "INSERT INTO cover_search (work_id, takedown_at) VALUES ({work}, now())"
        )),
        barrier: None,
        answer: answer(vec![doc(11, "Momo", "", "Michael Ende", Some(1100), 30)]),
    };
    let outcome = search_with(&app.db, &hook, work, false).await;
    assert!(matches!(outcome, Outcome::Suggested { .. }), "{outcome:?}");
    assert_eq!(
        suggestions(&app, work).await,
        [("OL11W".into(), 1100, "after_takedown".into(), "open".into())]
    );
    assert_eq!(cover_of(&app, work).await, None);
}

/// Bytes identical to a taken-down picture under another Open Library cover id are never
/// stored or served, and the works that named it get the takedown flag.
#[sqlx::test]
async fn a_taken_down_picture_never_comes_back_under_another_cover_id(db: PgPool) {
    let app = TestApp::new(db).await;
    let bytes = jpeg([150, 40, 40]);
    serve_cover(&app, 111, 200, bytes.clone()).await;
    serve_cover(&app, 222, 200, bytes).await;
    let first = legacy_work(&app, "Momo", "Michael Ende").await;
    let shown = ol_media(&app, 111).await;
    set_cover(&app, CoverTarget::Work(first), shown, Source::OpenLibrary).await;
    let service = &app.state.media_service;
    assert_eq!(
        service
            .ensure_cover(&app.db, shown, RetryPolicy::MEDIA)
            .await,
        CoverOutcome::Stored
    );
    let root = app.admin("root").await;
    root.delete(&format!("/api/admin/media/{}", public_id(&app, 111).await))
        .await
        .assert_status(204);

    let second = legacy_work(&app, "Momo und die Zeitdiebe", "Michael Ende").await;
    let other = ol_media(&app, 222).await;
    set_cover(&app, CoverTarget::Work(second), other, Source::OpenLibrary).await;
    assert_eq!(
        service
            .ensure_cover(&app.db, other, RetryPolicy::MEDIA)
            .await,
        CoverOutcome::Removed
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM media
             WHERE id = {other} AND removed_at IS NOT NULL AND storage_key IS NULL"
        ))
        .await,
        1
    );
    assert_eq!(cover_of(&app, second).await, None);
    assert!(search_row(&app, second).await.unwrap().takedown);
    let other_id = public_id(&app, 222).await;
    app.anon()
        .get(&format!("/media/{other_id}/l"))
        .await
        .assert_status(410);
    assert!(
        app.media
            .get(&format!("cover/{other_id}/l"))
            .await
            .unwrap()
            .is_none()
    );
}

/// Uploads `bytes` as a reader's cover of `work` (for `edition`), and returns the media id.
async fn upload_cover(
    app: &TestApp,
    reader: &TestClient,
    work: i64,
    edition: Option<i64>,
    bytes: Vec<u8>,
) -> i64 {
    let mut parts = vec![Part::file("cover.jpg", "image/jpeg", bytes)];
    if let Some(edition) = edition {
        parts.push(Part::text("edition_id", &edition.to_string()));
    }
    reader
        .post_multipart(&format!("/api/books/works/{work}/cover"), parts)
        .await
        .assert_status(200);
    app.scalar_i64("SELECT max(id) FROM media WHERE source = 'upload'")
        .await
}

/// Reports a cover for `reason` and has an admin remove it.
async fn report_and_remove(app: &TestApp, media_id: i64, reason: &str) {
    let reporter = app.user(&format!("reporter{media_id}")).await;
    let public_id: String = sqlx::query_scalar("SELECT public_id FROM media WHERE id = $1")
        .bind(media_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    let res = reporter
        .post(
            &format!("/api/media/{public_id}/reports"),
            json!({"reason": reason}),
        )
        .await;
    res.assert_status(201);
    let report = res.json()["id"].as_i64().unwrap();
    app.admin(&format!("admin{media_id}"))
        .await
        .post(
            &format!("/api/admin/reports/{report}/resolve"),
            json!({"action": "remove"}),
        )
        .await
        .assert_status(200);
}

async fn removed(app: &TestApp, media_id: i64) -> bool {
    app.scalar_i64(&format!(
        "SELECT count(*) FROM media WHERE id = {media_id} AND removed_at IS NOT NULL"
    ))
    .await
        == 1
}

/// A reader uploads one file for two editions of a book; its cover is reported and taken down.
/// The copy on the other edition goes with it, so the search cannot put it back on the book
/// (security review: the inherit step brought a live copy of the taken-down picture back).
#[sqlx::test]
async fn a_takedown_takes_the_readers_copy_on_another_edition_with_it(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = app.ol_work("OL5W", "Circe", "Madeline Miller").await;
    use_work(&app, work.id).await;
    let first = app.edition(&work, "9783161484100").await;
    let second = app.edition(&work, "9780306406157").await;
    let reader = app.user("rita").await;
    let bytes = jpeg([200, 0, 120]);
    let shown = upload_cover(&app, &reader, work.id, Some(first.id), bytes.clone()).await;
    let copy = upload_cover(&app, &reader, work.id, Some(second.id), bytes).await;
    assert_ne!(shown, copy);
    assert_eq!(cover_of(&app, work.id).await, Some((0, "user".into())));

    report_and_remove(&app, shown, "offensive").await;
    assert!(removed(&app, copy).await, "the copy went with it");
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM editions WHERE work_id = {} AND cover_media_id IS NOT NULL",
            work.id
        ))
        .await,
        0
    );
    let report = run_job(&app).await;
    assert_eq!(report.inherited, 0, "{report:?}");
    assert_eq!(cover_of(&app, work.id).await, None);
}

/// A takedown that only said the cover shows another book leaves the picture to its own book:
/// the same bytes as another book's Open Library cover are still fetched and shown there. The
/// reader's copy on the same book goes with it.
#[sqlx::test]
async fn a_wrong_book_takedown_leaves_the_picture_to_its_own_book(db: PgPool) {
    let app = TestApp::new(db).await;
    let bytes = jpeg([30, 90, 160]);
    serve_cover(&app, 611, 200, bytes.clone()).await;
    let own = legacy_work(&app, "Momo", "Michael Ende").await;
    let genuine = ol_media(&app, 611).await;
    set_cover(&app, CoverTarget::Work(own), genuine, Source::OpenLibrary).await;

    let wrong = app.ol_work("OL6W", "Circe", "Madeline Miller").await;
    use_work(&app, wrong.id).await;
    let first = app.edition(&wrong, "9783161484100").await;
    let second = app.edition(&wrong, "9780306406157").await;
    let reader = app.user("rita").await;
    let shown = upload_cover(&app, &reader, wrong.id, Some(first.id), bytes.clone()).await;
    let copy = upload_cover(&app, &reader, wrong.id, Some(second.id), bytes).await;
    report_and_remove(&app, shown, "wrong_book").await;
    assert!(removed(&app, copy).await, "the copy on the same book went with it");
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM media_removed_hashes WHERE blocks_covers")
            .await,
        0
    );

    assert_eq!(
        app.state
            .media_service
            .ensure_cover(&app.db, genuine, RetryPolicy::MEDIA)
            .await,
        CoverOutcome::Stored
    );
    assert_eq!(cover_of(&app, own).await, Some((611, "openlibrary".into())));
    assert!(search_row(&app, own).await.is_none_or(|row| !row.takedown));
}

/// A profile picture's takedown blocks the same file as a profile picture or an upload, but not
/// as a book's Open Library cover.
#[sqlx::test]
async fn a_profile_pictures_takedown_leaves_book_covers_alone(db: PgPool) {
    let app = TestApp::new(db).await;
    let bytes = jpeg([10, 160, 60]);
    serve_cover(&app, 621, 200, bytes.clone()).await;
    let work = legacy_work(&app, "Momo", "Michael Ende").await;
    let genuine = ol_media(&app, 621).await;
    set_cover(&app, CoverTarget::Work(work), genuine, Source::OpenLibrary).await;
    let reader = app.user("rita").await;
    let res = reader
        .put_multipart(
            "/api/auth/me/avatar",
            vec![Part::file("me.jpg", "image/jpeg", bytes)],
        )
        .await;
    res.assert_status(200);
    let avatar = res.json()["avatar_url"]
        .as_str()
        .unwrap()
        .strip_prefix("/media/")
        .and_then(|rest| rest.strip_suffix("/a"))
        .unwrap()
        .to_owned();
    app.admin("root")
        .await
        .delete(&format!("/api/admin/media/{avatar}"))
        .await
        .assert_status(204);

    assert_eq!(
        app.state
            .media_service
            .ensure_cover(&app.db, genuine, RetryPolicy::MEDIA)
            .await,
        CoverOutcome::Stored
    );
    assert_eq!(cover_of(&app, work).await, Some((621, "openlibrary".into())));
}

/// The dead check: a failing picture a day old that answers 404 again is gone, while the
/// cover server serves a picture we know. It is cleared from the work and from an edition,
/// which then shows the work's new cover; a refresh of the Open Library record does not bring
/// it back.
#[sqlx::test]
async fn a_picture_open_library_no_longer_has_is_gone(db: PgPool) {
    let app = TestApp::new(db).await;
    // A picture we have bytes for, for the probe.
    serve_cover(&app, 999, 200, jpeg([10, 200, 10])).await;
    let known = ol_media(&app, 999).await;
    assert_eq!(
        app.state
            .media_service
            .ensure_cover(&app.db, known, RetryPolicy::MEDIA)
            .await,
        CoverOutcome::Stored
    );
    let work = app.ol_work("OL9W", "Momo", "Michael Ende").await;
    use_work(&app, work.id).await;
    let edition = app.edition(&work, "9780316556347").await;
    let dead_media = ol_media(&app, 301).await;
    set_cover(
        &app,
        CoverTarget::Work(work.id),
        dead_media,
        Source::OpenLibrary,
    )
    .await;
    set_cover(
        &app,
        CoverTarget::Edition(edition.id),
        dead_media,
        Source::OpenLibrary,
    )
    .await;
    app.exec(&format!(
        "UPDATE media SET fetch_failed_at = now() - interval '25 hours' WHERE id = {dead_media}"
    ))
    .await;
    serve_cover(&app, 301, 404, Vec::new()).await;
    app.catalog.on_covers(
        "Momo",
        Some("Ende"),
        answer(vec![doc(41, "Momo", "", "Michael Ende", Some(4100), 30)]),
    );

    let report = run_job(&app).await;
    assert_eq!(report.dead.gone, 1, "{report:?}");
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM media WHERE id = {dead_media} AND ol_gone_at IS NOT NULL"
        ))
        .await,
        1
    );
    assert_eq!(
        cover_of(&app, work.id).await,
        Some((4100, "openlibrary".into()))
    );
    let mut conn = app.db.acquire().await.unwrap();
    let book = book_out(
        &mut conn,
        BookRef {
            work_id: work.id,
            edition_id: Some(edition.id),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(book.cover_public_id, Some(public_id(&app, 4100).await));

    // A refresh of the Open Library record names the gone picture again: it stays out.
    let record = WorkRecord {
        ol_work_id: Some("OL9W".into()),
        ol_cover_id: Some(301),
        ..WorkRecord::new(Source::OpenLibrary, "Momo")
    };
    ingest::upsert_work(&mut conn, &record, Mode::Refresh)
        .await
        .unwrap();
    assert_eq!(cover_of(&app, work.id).await.unwrap().0, 4100);
}

/// Without a working cover server (the probe fails), nothing is marked gone; a refused or
/// undecodable picture stays failing; a picture that loads again is stored.
#[sqlx::test]
async fn the_dead_check_marks_nothing_gone_on_a_broken_cover_server(db: PgPool) {
    let app = TestApp::new(db).await;
    let mut ids = Vec::new();
    for (cover, status, body) in [
        (301, 404, Vec::new()),
        (302, 200, b"not a picture".to_vec()),
        (303, 200, jpeg([1, 2, 3])),
    ] {
        let work = legacy_work(&app, &format!("Buch {cover}"), "Anna Autorin").await;
        let media = ol_media(&app, cover).await;
        set_cover(&app, CoverTarget::Work(work), media, Source::OpenLibrary).await;
        serve_cover(&app, cover, status, body).await;
        ids.push(media);
    }
    app.exec("UPDATE media SET fetch_failed_at = now() - interval '25 hours'")
        .await;
    let report = run_job(&app).await;
    // No stored picture to probe with: nothing is gone. 303 is stored now; then the probe
    // could pass, but this run asked it already.
    assert_eq!(report.dead.gone, 0, "{report:?}");
    assert_eq!(report.dead.unconfirmed, 1);
    assert_eq!(report.dead.unusable, 1);
    assert_eq!(report.dead.stored, 1);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM media WHERE ol_gone_at IS NOT NULL")
            .await,
        0
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM media WHERE id = ANY('{{{},{}}}')
               AND fetch_failed_at > now() - interval '1 minute'",
            ids[0], ids[1]
        ))
        .await,
        2,
        "both wait another day"
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM media WHERE id = {} AND storage_key IS NOT NULL",
            ids[2]
        ))
        .await,
        1
    );
}

/// A picture that fails for its own reason on every recheck (here a 403) goes to the back of
/// the queue instead of stopping the check, so the pictures after it are still settled; three
/// failures in a row stop the check (the cover server itself is down then).
#[sqlx::test]
async fn one_picture_that_keeps_failing_does_not_hold_up_the_dead_check(db: PgPool) {
    let app = TestApp::new(db).await;
    let mut ids = Vec::new();
    for (cover, status, body, hours) in [
        (311, 403, Vec::new(), 50),
        (312, 200, jpeg([4, 5, 6]), 25),
    ] {
        let work = legacy_work(&app, &format!("Buch {cover}"), "Anna Autorin").await;
        let media = ol_media(&app, cover).await;
        set_cover(&app, CoverTarget::Work(work), media, Source::OpenLibrary).await;
        serve_cover(&app, cover, status, body).await;
        app.exec(&format!(
            "UPDATE media SET fetch_failed_at = now() - interval '{hours} hours' WHERE id = {media}"
        ))
        .await;
        ids.push(media);
    }
    let report = run_job(&app).await;
    assert_eq!(
        (report.dead.failed, report.dead.stored, report.dead.stopped),
        (1, 1, false),
        "{report:?}"
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM media WHERE id = {}
               AND fetch_failed_at > now() - interval '1 minute' AND storage_key IS NULL",
            ids[0]
        ))
        .await,
        1,
        "the failing one waits another day, at the back"
    );

    // Three failures in a row stop the check.
    let mut failing = Vec::new();
    for cover in 321..=324 {
        let work = legacy_work(&app, &format!("Buch {cover}"), "Anna Autorin").await;
        let media = ol_media(&app, cover).await;
        set_cover(&app, CoverTarget::Work(work), media, Source::OpenLibrary).await;
        serve_cover(&app, cover, 500, Vec::new()).await;
        failing.push(media);
    }
    app.exec(&format!(
        "UPDATE media SET fetch_failed_at = now() - interval '30 hours'
         WHERE id = ANY('{{{}}}')",
        failing
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    ))
    .await;
    let report = run_job(&app).await;
    assert_eq!(
        (report.dead.failed, report.dead.rechecked, report.dead.stopped),
        (3, 3, true),
        "{report:?}"
    );
}

/// Never-fetched pictures of books readers use are queued for the prefetch; others are not.
#[sqlx::test]
async fn unfetched_pictures_of_books_in_use_are_prefetched(db: PgPool) {
    let app = TestApp::new(db).await;
    let used = legacy_work(&app, "Momo", "Michael Ende").await;
    let media = ol_media(&app, 501).await;
    set_cover(&app, CoverTarget::Work(used), media, Source::OpenLibrary).await;
    let unused = app.ol_work("OL7W", "Circe", "Madeline Miller").await;
    let other = ol_media(&app, 502).await;
    set_cover(
        &app,
        CoverTarget::Work(unused.id),
        other,
        Source::OpenLibrary,
    )
    .await;
    let report = run_job(&app).await;
    assert_eq!(report.prefetched, 1);
}

/// A work takes its edition's cover without asking Open Library. A reader's photo waiting for
/// the work keeps it out; one waiting for an edition does not.
#[sqlx::test]
async fn editions_covers_and_waiting_photos(db: PgPool) {
    let app = TestApp::new(db).await;
    let reader = app.user("ada").await;
    let inherits = legacy_work(&app, "Momo", "Michael Ende").await;
    let edition = app
        .edition(
            &TestWork {
                id: inherits,
                title: "Momo".into(),
            },
            "9780316556347",
        )
        .await;
    let photo = upload_media(&app).await;
    app.exec(&format!(
        "UPDATE editions SET cover_media_id = {photo},
                field_sources = '{{\"cover_media_id\": \"user\"}}' WHERE id = {}",
        edition.id
    ))
    .await;
    let waiting = legacy_work(&app, "Circe", "Madeline Miller").await;
    let request = upload_media(&app).await;
    app.exec(&format!(
        "INSERT INTO catalog_edit_requests (kind, work_id, media_id, created_by)
         VALUES ('cover_replace', {waiting}, {request}, {})",
        reader.id()
    ))
    .await;
    let edition_waiting = legacy_work(&app, "Dune", "Frank Herbert").await;
    let its_edition = app
        .edition(
            &TestWork {
                id: edition_waiting,
                title: "Dune".into(),
            },
            "9780441172719",
        )
        .await;
    let request = upload_media(&app).await;
    app.exec(&format!(
        "INSERT INTO catalog_edit_requests (kind, work_id, edition_id, media_id, created_by)
         VALUES ('cover_replace', {edition_waiting}, {}, {request}, {})",
        its_edition.id,
        reader.id()
    ))
    .await;

    let report = run_job(&app).await;
    assert_eq!(report.inherited, 1);
    assert_eq!(cover_of(&app, inherits).await, Some((0, "user".into())));
    assert_eq!(report.works, [inherits, edition_waiting]);
    assert_eq!(app.catalog.cover_calls(), ["cover:Dune|Herbert|en"]);
}

/// #74 AC 4: throttled ends the run with nothing recorded, and the next run resumes.
#[sqlx::test]
async fn a_throttled_run_records_nothing_and_the_next_resumes(db: PgPool) {
    let app = TestApp::new(db).await;
    let first = legacy_work(&app, "Momo", "Michael Ende").await;
    let second = legacy_work(&app, "Circe", "Madeline Miller").await;
    app.catalog.on_covers(
        "Momo",
        Some("Ende"),
        answer(vec![doc(11, "Momo", "", "Michael Ende", Some(1100), 30)]),
    );
    app.catalog.on_covers(
        "Circe",
        Some("Miller"),
        answer(vec![doc(
            13,
            "Circe",
            "",
            "Madeline Miller",
            Some(1300),
            50,
        )]),
    );
    app.catalog.fail_covers(FakeError::Throttled, 1);
    let report = run_job(&app).await;
    assert!(report.throttled);
    assert_eq!(report.searched, 0);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM cover_search").await, 0);
    let report = run_job(&app).await;
    assert_eq!(report.attached, 2);
    assert!(cover_of(&app, first).await.is_some() && cover_of(&app, second).await.is_some());
}

/// #74 AC 4: unavailable backs the work off (1 hour, then 2), ends the run and opens the
/// breaker; the next answer closes it.
#[sqlx::test]
async fn an_outage_backs_off_and_opens_the_breaker(db: PgPool) {
    let app = TestApp::new(db).await;
    let first = legacy_work(&app, "Momo", "Michael Ende").await;
    let second = legacy_work(&app, "Circe", "Madeline Miller").await;
    app.catalog.fail_covers(FakeError::Unavailable, 1);
    let mut breaker = Breaker::default();
    let report = run_job(&app).await;
    assert!(report.unavailable);
    assert_eq!(report.works, [first]);
    let row = search_row(&app, first).await.unwrap();
    assert_eq!(
        (row.failures, row.next_hours, row.searched),
        (1, Some(1), false)
    );
    assert_eq!(search_row(&app, second).await, None, "the run ended");
    breaker.record(&report, tokio::time::Instant::now());
    assert!(!breaker.allows(tokio::time::Instant::now()));

    let report = run_job(&app).await;
    assert_eq!(report.works, [second], "the first waits its hour");
    assert!(report.answered);
    breaker.record(&report, tokio::time::Instant::now());
    assert!(breaker.allows(tokio::time::Instant::now()));
    assert_eq!(breaker.outages(), 0);

    app.exec(&format!(
        "UPDATE cover_search SET next_search_at = now() WHERE work_id = {first}"
    ))
    .await;
    app.catalog.fail_covers(FakeError::Unavailable, 1);
    run_job(&app).await;
    let row = search_row(&app, first).await.unwrap();
    assert_eq!((row.failures, row.next_hours), (2, Some(2)));
}

/// Makes the work due and runs the job with its search failing.
async fn fail_again(app: &TestApp, work_id: i64) -> job::RunReport {
    app.exec(&format!(
        "UPDATE cover_search SET next_search_at = now() WHERE work_id = {work_id}"
    ))
    .await;
    app.catalog.fail_covers(FakeError::Unavailable, 1);
    run_job(app).await
}

/// A book Open Library fails on every time, while it answers the others, counts a miss from the
/// 5th failure in a row on: it gets the miss backoff and the tier-2 stop instead of a request
/// every day for good. Without another answer in the last day (an outage) the failures only add
/// up.
#[sqlx::test]
async fn a_book_open_library_always_fails_on_counts_a_miss(db: PgPool) {
    let app = TestApp::new(db).await;
    let answered = legacy_work(&app, "Momo", "Michael Ende").await;
    assert!(run_job(&app).await.answered);
    let (failing, _) = app
        .dnb_work("Der Schwarm", "Frank Schätzing", "9783161484100")
        .await;
    for failures in 1..=4 {
        let report = fail_again(&app, failing.id).await;
        assert_eq!(report.works, [failing.id], "{report:?}");
        let row = search_row(&app, failing.id).await.unwrap();
        assert_eq!((row.failures, row.misses), (failures, 0));
    }
    fail_again(&app, failing.id).await;
    let row = search_row(&app, failing.id).await.unwrap();
    assert_eq!(row.outcome.as_deref(), Some("no_match"));
    assert_eq!((row.failures, row.misses, row.searched), (5, 1, false));
    assert_eq!(row.next_hours, Some(30 * 24));
    assert!(row.last_error.unwrap().contains("5 times"));
    // The next failure is the second miss, and a tier-2 book then stops.
    fail_again(&app, failing.id).await;
    assert_eq!(search_row(&app, failing.id).await.unwrap().misses, 2);
    app.exec(&format!(
        "UPDATE cover_search SET next_search_at = now() WHERE work_id = {}",
        failing.id
    ))
    .await;
    assert!(run_job(&app).await.works.is_empty());

    // An outage: nothing else answered in the last day, so the failures only add up.
    app.exec(&format!(
        "UPDATE cover_search SET searched_at = now() - interval '2 days' WHERE work_id = {answered}"
    ))
    .await;
    let other = legacy_work(&app, "Circe", "Madeline Miller").await;
    for _ in 0..6 {
        fail_again(&app, other).await;
    }
    let row = search_row(&app, other).await.unwrap();
    assert_eq!((row.failures, row.misses, row.searched), (6, 0, false));
}

/// With the real client: a payload without `docs` is an outage that ends the run; a 400 is a
/// no match that names the status and does not.
#[sqlx::test]
async fn open_library_error_answers_get_their_own_outcomes(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    let refused = legacy_work(&app, "Momo", "Michael Ende").await;
    let broken = legacy_work(&app, "Circe", "Madeline Miller").await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", "title:(Momo) author:(Ende)"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "bad"})))
        .mount(&app.upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", "title:(Circe) author:(Miller)"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"numFound": 0})))
        .mount(&app.upstream)
        .await;
    let report = run_job(&app).await;
    assert_eq!(report.works, [refused, broken]);
    assert_eq!(report.no_match, 1);
    assert!(report.unavailable);
    let row = search_row(&app, refused).await.unwrap();
    assert_eq!(row.outcome.as_deref(), Some("no_match"));
    assert!(row.last_error.unwrap().contains("400"));
    assert_eq!(search_row(&app, broken).await.unwrap().failures, 1);
}

/// No match waits 30 days, then 60; a changed title or first author is due at once with the
/// misses reset.
#[sqlx::test]
async fn misses_back_off_and_a_corrected_title_is_due_at_once(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = legacy_work(&app, "Momo", "Michael Ende").await;
    run_job(&app).await;
    let row = search_row(&app, work).await.unwrap();
    assert_eq!((row.misses, row.next_hours), (1, Some(30 * 24)));
    run_job(&app).await;
    assert_eq!(app.catalog.cover_calls().len(), 1, "not due");
    app.exec(&format!(
        "UPDATE cover_search SET next_search_at = now() WHERE work_id = {work}"
    ))
    .await;
    run_job(&app).await;
    let row = search_row(&app, work).await.unwrap();
    assert_eq!((row.misses, row.next_hours), (2, Some(60 * 24)));

    let root = app.admin("root").await;
    root.patch(
        &format!("/api/admin/works/{work}"),
        json!({"fields": {"title": "Momo oder Die seltsame Geschichte"}}),
    )
    .await
    .assert_status(200);
    let report = run_job(&app).await;
    assert_eq!(report.works, [work]);
    let row = search_row(&app, work).await.unwrap();
    assert_eq!((row.misses, row.next_hours), (1, Some(30 * 24)));
}

/// A book that loses its cover (a takedown, or the gone step) is due for the search at once,
/// whatever miss backoff it had before it got that cover; also a book nobody uses that had
/// stopped after two misses.
#[sqlx::test]
async fn a_book_that_loses_its_cover_is_due_at_once(db: PgPool) {
    let app = TestApp::new(db).await;
    let used = legacy_work(&app, "Momo", "Michael Ende").await;
    let (unused, _) = app
        .dnb_work("Der Schwarm", "Frank Schätzing", "9783161484100")
        .await;
    app.exec(&format!(
        "UPDATE works SET details_fetched_at = now() WHERE id = {}",
        unused.id
    ))
    .await;
    run_job(&app).await;
    app.exec(&format!(
        "UPDATE cover_search SET misses = 2, next_search_at = now() + interval '60 days'
         WHERE work_id = {}",
        unused.id
    ))
    .await;
    assert_eq!(search_row(&app, used).await.unwrap().misses, 1);
    let mut conn = app.db.acquire().await.unwrap();
    assert!(job::due_works(&mut conn, 30).await.unwrap().is_empty());

    // A reader's photo, taken down; an Open Library picture, gone.
    let photo = upload_media(&app).await;
    set_cover(&app, CoverTarget::Work(used), photo, Source::User).await;
    let picture = ol_media(&app, 321).await;
    set_cover(
        &app,
        CoverTarget::Work(unused.id),
        picture,
        Source::OpenLibrary,
    )
    .await;
    ingest::clear_media(&mut conn, photo).await.unwrap();
    assert!(dead::mark_gone(&app.db, picture).await.unwrap());

    let mut due = job::due_works(&mut conn, 30).await.unwrap();
    due.sort_unstable();
    assert_eq!(due, [used, unused.id]);
    let row = search_row(&app, used).await.unwrap();
    assert_eq!((row.misses, row.next_hours, row.takedown), (0, None, true));
    let row = search_row(&app, unused.id).await.unwrap();
    assert_eq!((row.misses, row.next_hours, row.takedown), (0, None, false));
}

/// The gone probe tries an older stored picture when Open Library no longer has the newest
/// one: a single deleted picture does not stop every 404 from being confirmed.
#[sqlx::test]
async fn the_probe_tries_an_older_picture(db: PgPool) {
    let app = TestApp::new(db).await;
    // Two pictures we have bytes for; Open Library deleted the newer one.
    for cover in [998, 999] {
        let media = ol_media(&app, cover).await;
        app.exec(&format!(
            "UPDATE media SET storage_key = 'cover/probe{cover}', content_type = 'image/webp'
             WHERE id = {media}"
        ))
        .await;
    }
    serve_cover(&app, 998, 200, jpeg([10, 200, 10])).await;
    serve_cover(&app, 999, 404, Vec::new()).await;
    let work = legacy_work(&app, "Momo", "Michael Ende").await;
    let dead_media = ol_media(&app, 301).await;
    set_cover(
        &app,
        CoverTarget::Work(work),
        dead_media,
        Source::OpenLibrary,
    )
    .await;
    app.exec(&format!(
        "UPDATE media SET fetch_failed_at = now() - interval '25 hours' WHERE id = {dead_media}"
    ))
    .await;
    serve_cover(&app, 301, 404, Vec::new()).await;
    let report = run_job(&app).await;
    assert_eq!(
        (report.dead.gone, report.dead.unconfirmed),
        (1, 0),
        "{report:?}"
    );

    // A failing cover server (5xx on the newest) confirms nothing.
    let other = legacy_work(&app, "Circe", "Madeline Miller").await;
    let failing = ol_media(&app, 302).await;
    set_cover(&app, CoverTarget::Work(other), failing, Source::OpenLibrary).await;
    app.exec(&format!(
        "UPDATE media SET fetch_failed_at = now() - interval '25 hours' WHERE id = {failing}"
    ))
    .await;
    serve_cover(&app, 302, 404, Vec::new()).await;
    let newest = ol_media(&app, 1000).await;
    app.exec(&format!(
        "UPDATE media SET storage_key = 'cover/probe1000', content_type = 'image/webp'
         WHERE id = {newest}"
    ))
    .await;
    serve_cover(&app, 1000, 503, Vec::new()).await;
    let report = run_job(&app).await;
    assert_eq!(
        (report.dead.gone, report.dead.unconfirmed),
        (0, 1),
        "{report:?}"
    );
}

/// Tier 1 (books in use) before tier 2; tier 2 never suggests, stops after two misses and is
/// due again once readers use it. DNB works still to come and Open Library works nobody uses
/// are not searched.
#[sqlx::test]
async fn books_in_use_first_and_the_rest_with_restraint(db: PgPool) {
    let app = TestApp::new(db).await;
    let (unused, _) = app
        .dnb_work("Der Schwarm", "Frank Schätzing", "9783161484100")
        .await;
    let used = legacy_work(&app, "Gregs Tagebuch - Jetzt reicht's!", "Jeff Kinney").await;
    let (planned, _) = app
        .dnb_work("Das kommende Buch", "Anna Autorin", "9783161484117")
        .await;
    app.exec(&format!(
        "UPDATE editions SET planned_publication_date = current_date + 60
         WHERE work_id = {}",
        planned.id
    ))
    .await;
    app.ol_work("OL7W", "Circe", "Madeline Miller").await;
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
    app.catalog.on_covers(
        "Der Schwarm",
        Some("Schätzing"),
        answer(vec![doc(
            22,
            "Der Schwarm 2",
            "",
            "Frank Schätzing",
            Some(2200),
            8,
        )]),
    );
    let one = JobSettings {
        batch: 1,
        ..settings()
    };
    let report = run_job_with(&app, &one).await;
    assert_eq!(report.works, [used]);
    assert_eq!(report.suggested, 1);
    let report = run_job(&app).await;
    assert_eq!(report.works, [unused.id]);
    assert_eq!(report.uncertain, 1);
    assert!(
        suggestions(&app, unused.id).await.is_empty(),
        "tier 2 never suggests"
    );
    for misses in [2, 2] {
        app.exec(&format!(
            "UPDATE cover_search SET next_search_at = now() WHERE work_id = {}",
            unused.id
        ))
        .await;
        run_job(&app).await;
        assert_eq!(search_row(&app, unused.id).await.unwrap().misses, misses);
    }
    let searched: Vec<String> = app.catalog.cover_calls();
    assert!(
        !searched
            .iter()
            .any(|c| c.contains("Circe") || c.contains("kommende"))
    );

    use_work(&app, unused.id).await;
    let report = run_job(&app).await;
    assert_eq!(report.works, [unused.id], "uncertain and in use: due");
    assert_eq!(suggestions(&app, unused.id).await.len(), 1);
}

/// Suggestions nobody needs to decide any more are closed: the book got a cover, or the
/// picture is gone.
#[sqlx::test]
async fn moot_suggestions_close(db: PgPool) {
    let app = TestApp::new(db).await;
    let covered = legacy_work(&app, "Gregs Tagebuch - Jetzt reicht's!", "Jeff Kinney").await;
    let gone = legacy_work(&app, "Dune: House Harkonnen", "Brian Herbert").await;
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
    app.catalog.on_covers(
        "Dune",
        Some("Herbert"),
        answer(vec![doc(22, "Dune", "", "Brian Herbert", Some(2200), 80)]),
    );
    assert_eq!(run_job(&app).await.suggested, 2);
    let photo = upload_media(&app).await;
    set_cover(&app, CoverTarget::Work(covered), photo, Source::User).await;
    app.exec("UPDATE media SET ol_gone_at = now() WHERE ol_cover_id = 2200")
        .await;
    let root = app.admin("root").await;
    let list = root.get("/api/admin/cover-suggestions").await.json();
    assert_eq!(list["items"], json!([]), "hidden before they are closed");
    let report = run_job(&app).await;
    assert_eq!(report.closed, 2);
    assert_eq!(suggestions(&app, covered).await[0].3, "closed");
    assert_eq!(suggestions(&app, gone).await[0].3, "closed");
}

/// A second run while another session holds the job's lock is skipped.
#[sqlx::test]
async fn a_second_run_is_skipped_while_the_lock_is_held(db: PgPool) {
    let app = TestApp::new(db).await;
    legacy_work(&app, "Momo", "Michael Ende").await;
    let mut holder = app.db.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(lock_ids::COVER_SEARCH)
        .execute(&mut *holder)
        .await
        .unwrap();
    let token = CancellationToken::new();
    assert_eq!(job::run_once(&app.state, &token, &settings()).await, None);
    assert!(app.catalog.cover_calls().is_empty());
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(lock_ids::COVER_SEARCH)
        .execute(&mut *holder)
        .await
        .unwrap();
    let report = job::run_once(&app.state, &token, &settings())
        .await
        .unwrap();
    assert_eq!(report.searched, 1);
}

/// #74 AC 4: a stop ends the run between works, during the pace, well within the grace period.
#[sqlx::test]
async fn a_cancelled_token_ends_the_run_between_works(db: PgPool) {
    let app = TestApp::new(db).await;
    legacy_work(&app, "Momo", "Michael Ende").await;
    legacy_work(&app, "Circe", "Madeline Miller").await;
    let token = CancellationToken::new();
    let cancel = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();
    });
    let paced = JobSettings {
        pace: Duration::from_secs(20),
        ..settings()
    };
    let started = Instant::now();
    let mut conn = app.db.acquire().await.unwrap();
    let report = job::run(&app.state, &mut conn, &token, &paced)
        .await
        .unwrap();
    assert!(report.stopped);
    assert_eq!(report.searched, 1);
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// Two searches of one book at once record one result and count one miss.
#[sqlx::test]
async fn two_searches_at_once_count_one_miss(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = legacy_work(&app, "Momo", "Michael Ende").await;
    let hook = Hook {
        db: app.db.clone(),
        sql: None,
        barrier: Some(Arc::new(Barrier::new(2))),
        answer: answer(Vec::new()),
    };
    let (a, b) = tokio::join!(
        search_with(&app.db, &hook, work, false),
        search_with(&app.db, &hook, work, false)
    );
    assert_eq!((a, b), (Outcome::NoMatch, Outcome::NoMatch));
    assert_eq!(search_row(&app, work).await.unwrap().misses, 1);
}

/// Race: a takedown of the picture during an attach never deadlocks, and the picture never
/// stays on the book.
#[sqlx::test]
async fn a_takedown_racing_an_attach(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    for round in 0..4u32 {
        let title = format!("Momo {round}");
        let work = legacy_work(&app, &title, "Michael Ende").await;
        let cover = 6000 + i64::from(round);
        ol_media(&app, cover).await;
        app.catalog.on_covers(
            &title,
            Some("Ende"),
            answer(vec![doc(
                60 + round,
                &title,
                "",
                "Michael Ende",
                Some(cover),
                9,
            )]),
        );
        let takedown = format!("/api/admin/media/{}", public_id(&app, cover).await);
        let lookup = app.state.lookup.clone();
        let (outcome, res) = tokio::join!(
            search_with(&app.db, lookup.as_ref(), work, false),
            root.delete(&takedown)
        );
        res.assert_status(204);
        assert!(
            matches!(outcome, Outcome::Attached { .. } | Outcome::NoMatch),
            "{outcome:?}"
        );
        assert_eq!(cover_of(&app, work).await, None, "round {round}");
    }
}

/// Race: the gone step during an attach never deadlocks, and the gone picture never stays.
#[sqlx::test]
async fn the_gone_step_racing_an_attach(db: PgPool) {
    let app = TestApp::new(db).await;
    for round in 0..4u32 {
        let title = format!("Circe {round}");
        let work = legacy_work(&app, &title, "Madeline Miller").await;
        let cover = 7000 + i64::from(round);
        let media = ol_media(&app, cover).await;
        app.exec(&format!(
            "UPDATE media SET fetch_failed_at = now() - interval '2 days' WHERE id = {media}"
        ))
        .await;
        app.catalog.on_covers(
            &title,
            Some("Miller"),
            answer(vec![doc(
                70 + round,
                &title,
                "",
                "Madeline Miller",
                Some(cover),
                9,
            )]),
        );
        let lookup = app.state.lookup.clone();
        let (outcome, marked) = tokio::join!(
            search_with(&app.db, lookup.as_ref(), work, false),
            dead::mark_gone(&app.db, media)
        );
        assert!(marked.unwrap());
        assert!(
            matches!(outcome, Outcome::Attached { .. } | Outcome::NoMatch),
            "{outcome:?}"
        );
        assert_eq!(cover_of(&app, work).await, None, "round {round}");
    }
}

/// Race: the job against an approval of the same book: no deadlock, the admin's picture wins.
#[sqlx::test]
async fn the_job_racing_an_approval(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    for round in 0..3u32 {
        let title = format!("Gregs Tagebuch {round} - Jetzt reicht's!");
        let work = legacy_work(&app, &title, "Jeff Kinney").await;
        let main = format!("Gregs Tagebuch {round}");
        let cover = 8000 + i64::from(round);
        app.catalog.on_covers(
            &main,
            Some("Kinney"),
            answer(vec![doc(
                80 + round,
                &main,
                "",
                "Jeff Kinney",
                Some(cover),
                9,
            )]),
        );
        run_job(&app).await;
        let id = app
            .scalar_i64(&format!(
                "SELECT id FROM cover_suggestions WHERE work_id = {work}"
            ))
            .await;
        let approve = format!("/api/admin/cover-suggestions/{id}/approve");
        let (res, report) = tokio::join!(root.post(&approve, json!({})), run_job(&app));
        res.assert_status(200);
        assert!(!report.works.contains(&work));
        assert_eq!(cover_of(&app, work).await, Some((cover, "admin".into())));
    }
}

//! Legacy diary entries, pick notes, reactions and events (docs/phase1/legacy-mapping.md §9,
//! §12; §17 AC 7, 8 and 13), on tests/fixtures/legacy.

mod common;
#[path = "fixtures/legacy/harness.rs"]
mod legacy;

use bookclub_server::catalog::shared::ingest::Source;
use bookclub_server::importer::Report;
use common::*;
use legacy::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn merged_import(app: &TestApp, legacy: &Legacy) -> Report {
    app.catalog.on_isbn(
        ISBN_MATCH,
        FakeHit::New {
            title: "Middlemarch".into(),
            authors: "George Eliot".into(),
            ol_work_id: Some("OL45883W".into()),
            source: Source::OpenLibrary,
        },
    );
    import_ok(app, legacy.args("demo")).await
}

/// id, club_id, kind, pick_id, payload.
type EventRow = (i64, Option<i64>, String, Option<i64>, Value);

/// A post's scope and content.
#[derive(Debug, sqlx::FromRow, PartialEq)]
struct Post {
    id: i64,
    work_id: i64,
    author_id: Option<i64>,
    parent_id: Option<i64>,
    club_id: Option<i64>,
    pick_id: Option<i64>,
    milestone_id: Option<i64>,
    visibility: String,
    body: String,
    spoiler_upto: Option<i32>,
    progress_at: Option<i32>,
    status_at: Option<String>,
    deleted: bool,
}

async fn posts(app: &TestApp) -> Vec<Post> {
    sqlx::query_as(
        "SELECT id, work_id, author_id, parent_id, club_id, pick_id, milestone_id, visibility,
                body, spoiler_upto, progress_at, status_at, deleted_at IS NOT NULL AS deleted
         FROM posts ORDER BY id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap()
}

fn post(all: &[Post], id: i64) -> &Post {
    all.iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("no post {id}"))
}

async fn reactions(app: &TestApp, post: i64) -> Vec<(i64, String)> {
    sqlx::query_as("SELECT user_id, emoji FROM post_reactions WHERE post_id = $1 ORDER BY user_id")
        .bind(post)
        .fetch_all(&app.db)
        .await
        .unwrap()
}

/// #38 AC 7: a mirrored note is not duplicated and keeps only its diary reactions (one
/// removed after mirroring stays removed, one added to the old note is not brought in); a new
/// note becomes a club post with an id above every kept one, with its reactions; a note whose
/// diary copy was deleted is skipped and listed; the milestone link is carried across.
#[sqlx::test]
async fn legacy_pick_notes_are_neither_lost_nor_duplicated(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let report = merged_import(&app, &legacy).await;
    let all = posts(&app).await;
    let club = app.scalar_i64("SELECT id FROM clubs").await;
    let count = |body: &str| all.iter().filter(|p| p.body == body).count();

    // Note 1, mirrored as book_posts 6.
    assert_eq!(count("Alte Notiz zur letzten Lektüre."), 1);
    let p6 = post(&all, 6);
    assert_eq!(
        (p6.club_id, p6.pick_id, p6.author_id, p6.spoiler_upto),
        (Some(club), Some(1), Some(2), Some(10))
    );
    // demo's 🔥 is on the diary copy; max's 👍 was removed there after mirroring, lena's 😂 was
    // only ever on the old note.
    assert_eq!(reactions(&app, 6).await, vec![(1, "🔥".to_owned())]);

    // Note 3, mirrored as book_posts 7, with its milestone and a reply.
    assert_eq!(count("Notiz zum ersten Abschnitt."), 1);
    let p7 = post(&all, 7);
    assert_eq!(
        (p7.club_id, p7.pick_id, p7.milestone_id),
        (Some(club), Some(2), Some(1))
    );
    assert_eq!(reactions(&app, 7).await, vec![(2, "❤️".to_owned())]);

    // Note 4: written after the last mirroring, a new club post above every kept id.
    let max_kept = 13;
    let new: Vec<&Post> = all
        .iter()
        .filter(|p| p.body == "Neue Notiz nach dem letzten Start.")
        .collect();
    assert_eq!(new.len(), 1);
    let new = new[0];
    assert!(new.id > max_kept, "{}", new.id);
    let book1 = report.books[&book_id(&legacy, "/works/BC0000000001")].work_id;
    assert_eq!(
        (
            new.work_id,
            new.club_id,
            new.pick_id,
            new.author_id,
            new.spoiler_upto,
            new.progress_at,
            new.status_at.as_deref()
        ),
        (book1, Some(club), Some(2), Some(4), Some(50), None, None)
    );
    assert_eq!(reactions(&app, new.id).await, vec![(1, "❤️".to_owned())]);

    // Note 2: its diary copy was deleted; skipped and listed.
    assert_eq!(count("Diese Kopie löscht der Autor im Tagebuch."), 0);
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.starts_with("club_pick_posts 2: not mirrored")),
        "{:?}",
        report.notes
    );
    let line = report.entity("club_pick_posts").unwrap();
    assert_eq!((line.source, line.target), (Some(4), Some(1)));
    assert!(line.detail.contains("1 skipped (2)"), "{}", line.detail);
    let line = report.entity("post_reactions").unwrap();
    assert_eq!(
        (line.source, line.target, line.detail.as_str()),
        (
            Some(5),
            Some(1),
            "(2 of mirrored notes already on the diary copy, 2 of mirrored notes not on the \
             diary copy, dropped)"
        )
    );
    assert_eq!(
        app.scalar_i64("SELECT nextval(pg_get_serial_sequence('posts', 'id'))")
            .await,
        new.id + 1
    );
}

/// #38 AC 8: diary entries the legacy app stamped with the current pick stay diary entries
/// (with their replies); a mirror copy becomes a club post with its replies.
#[sqlx::test]
async fn only_real_pick_notes_become_club_posts(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let report = merged_import(&app, &legacy).await;
    let all = posts(&app).await;
    let club = app.scalar_i64("SELECT id FROM clubs").await;
    for id in [1, 2, 3, 4, 5] {
        let p = post(&all, id);
        assert_eq!((p.club_id, p.pick_id), (None, None), "post {id}");
        assert_eq!(p.visibility, "audience");
    }
    assert_eq!(post(&all, 5).parent_id, Some(1));
    let reply = post(&all, 9);
    assert_eq!(
        (
            reply.parent_id,
            reply.club_id,
            reply.pick_id,
            reply.milestone_id
        ),
        (Some(7), Some(club), Some(2), None)
    );
    assert_eq!(report.normalised("book_posts.pick_id"), 5);
    let line = report.entity("book_posts").unwrap();
    assert_eq!(
        (line.source, line.target, line.detail.as_str()),
        (
            Some(12),
            Some(12),
            "(9 diary entries, 3 club posts; 5 stamped pick ids dropped from diary entries)"
        )
    );
}

/// §9.1: tombstones stay tombstones with their replies; out-of-range positions become NULL;
/// a diary entry on a merged book moves to the merged work; a reply to a reply becomes a
/// top-level post (counted).
#[sqlx::test]
async fn diary_entries_keep_their_threads(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    legacy.exec(
        "INSERT INTO book_posts (id, book_id, author_id, parent_id, body, created_at)
         VALUES (20, 1, 3, 5, 'Antwort auf eine Antwort.', '2026-09-01 11:00:00');",
    );
    let report = merged_import(&app, &legacy).await;
    let all = posts(&app).await;
    let tomb = post(&all, 10);
    assert_eq!(
        (
            tomb.deleted,
            tomb.body.as_str(),
            tomb.spoiler_upto,
            tomb.author_id
        ),
        (true, "", None, Some(3))
    );
    let reply = post(&all, 11);
    assert_eq!(
        (reply.parent_id, reply.deleted, reply.club_id),
        (Some(10), false, None)
    );
    let p13 = post(&all, 13);
    assert_eq!((p13.spoiler_upto, p13.progress_at), (None, None));
    let ol = report.books[&book_id(&legacy, "/works/OL45883W")].work_id;
    let p12 = post(&all, 12);
    assert_eq!(
        (p12.work_id, p12.progress_at, p12.status_at.as_deref()),
        (ol, Some(30), Some("currently_reading"))
    );
    assert_eq!(post(&all, 20).parent_id, None);
    assert_eq!(report.normalised("book_posts.parent_id"), 1);
    assert_eq!(report.normalised("book_posts.spoiler_upto"), 1);
    assert_eq!(report.normalised("book_posts.progress_at"), 1);
}

/// A legacy pick note is imported once, with its reactions, whether it was mirrored into the
/// diary (the copy is canonical) or never mirrored (it becomes a new club post).
/// parity: test_diary.py::test_legacy_pick_notes_are_imported_once_with_reactions
#[sqlx::test]
async fn legacy_pick_notes_are_imported_once_with_reactions(db: PgPool) {
    // migrated.sql: note 1 of bea was mirrored as book_posts 1 at the boot. ada reacted 🔥 on
    // the old note before the boot, so both reaction tables have it.
    let app = TestApp::new(db.clone()).await;
    let legacy = load("migrated.sql");
    legacy.exec(
        "UPDATE club_pick_posts SET spoiler_upto = 30 WHERE id = 1;
         UPDATE book_posts SET spoiler_upto = 30 WHERE id = 1;
         INSERT INTO post_reactions VALUES (1, 1, 1, '🔥', '2024-01-07 08:00:00');
         INSERT INTO book_post_reactions VALUES (1, 1, 1, '🔥', '2024-01-07 08:00:00');",
    );
    import_ok(&app, legacy.args("ada")).await;
    let all = posts(&app).await;
    assert_eq!(all.len(), 1);
    let club = app.scalar_i64("SELECT id FROM clubs").await;
    assert_eq!(
        (
            all[0].id,
            all[0].body.as_str(),
            all[0].spoiler_upto,
            all[0].author_id,
            all[0].club_id,
            all[0].pick_id
        ),
        (
            1,
            "Written before the diary existed",
            Some(30),
            Some(2),
            Some(club),
            Some(1)
        )
    );
    assert_eq!(reactions(&app, 1).await, vec![(1, "🔥".to_owned())]);

    // The same note never mirrored: a new club post, with the old note's reaction.
    let legacy = load("migrated.sql");
    legacy.exec(
        "DELETE FROM book_posts;
         UPDATE club_pick_posts SET spoiler_upto = 30 WHERE id = 1;
         INSERT INTO post_reactions VALUES (1, 1, 1, '🔥', '2024-01-07 08:00:00');",
    );
    app.exec("TRUNCATE users, clubs, works, editions, media CASCADE")
        .await;
    import_ok(&app, legacy.args("ada")).await;
    let all = posts(&app).await;
    assert_eq!(all.len(), 1);
    assert_eq!(
        (
            all[0].body.as_str(),
            all[0].spoiler_upto,
            all[0].author_id,
            all[0].pick_id
        ),
        (
            "Written before the diary existed",
            Some(30),
            Some(2),
            Some(1)
        )
    );
    assert_eq!(reactions(&app, all[0].id).await, vec![(1, "🔥".to_owned())]);
}

/// An emoji outside the six aborts the import (impossible through the API), naming the row.
#[sqlx::test]
async fn an_unknown_reaction_aborts(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    legacy.exec("UPDATE book_post_reactions SET emoji = '🙈' WHERE id = 1;");
    let error = import(&app, legacy.args("demo")).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("book_post_reactions id 1, column emoji"),
        "{error}"
    );
    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 0);
}

/// #38 AC 13 and §12: club kinds join the club and keep their pick, personal kinds lose it;
/// `note_posted` and `reaction` carry the post that stands for the legacy note (none for a
/// skipped one), `reaction` an `author_id` instead of `author`, `quote_added` a `quote_id`
/// (also for a quote edited later; none for a deleted one, listed); broken payloads become {}.
#[sqlx::test]
async fn event_payloads_point_at_the_imported_rows(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    let report = merged_import(&app, &legacy).await;
    let club = app.scalar_i64("SELECT id FROM clubs").await;
    let new_note = app
        .scalar_i64("SELECT id FROM posts WHERE body = 'Neue Notiz nach dem letzten Start.'")
        .await;
    let events: Vec<EventRow> =
        sqlx::query_as("SELECT id, club_id, kind, pick_id, payload FROM events ORDER BY id")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(events.len(), 48);
    let ev = |id: i64| {
        events
            .iter()
            .find(|e| e.0 == id)
            .unwrap_or_else(|| panic!("no event {id}"))
    };
    for e in &events {
        let personal = matches!(
            e.2.as_str(),
            "shelf_added"
                | "shelf_moved"
                | "shelf_finished"
                | "shelf_dnf"
                | "shelf_removed"
                | "progress"
                | "quote_added"
        );
        assert_eq!(e.1.is_none(), personal, "event {} ({})", e.0, e.2);
    }
    // note_posted: notes 1, 3, 4 map to their posts; note 2 was skipped.
    assert_eq!(ev(25).4, json!({"post_id": 6}));
    assert_eq!(ev(26).4, json!({"post_id": 7}));
    assert_eq!(ev(27).4, json!({}));
    assert_eq!(ev(29).4, json!({"post_id": new_note}));
    assert_eq!((ev(25).1, ev(25).3), (Some(club), Some(1)));
    // reaction: author → author_id (an unknown name is dropped).
    assert_eq!(
        ev(28).4,
        json!({"emoji": "🔥", "post_id": 6, "author_id": 2})
    );
    assert_eq!(
        ev(30).4,
        json!({"emoji": "❤️", "post_id": new_note, "author_id": 4})
    );
    assert_eq!(ev(31).4, json!({"emoji": "👍"}));
    // quote_added: the seed quote, the edited quote, the deleted one.
    assert_eq!(ev(32).4["quote_id"], json!(1));
    assert_eq!(
        ev(33).4,
        json!({"page": 12, "excerpt": "Ursprünglicher Satz.", "quote_id": 2})
    );
    assert_eq!(ev(34).4.get("quote_id"), None);
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.starts_with("events 34: quote_added")),
        "{:?}",
        report.notes
    );
    // A personal kind loses the pick the legacy app stamped on it.
    assert_eq!((ev(46).1, ev(46).3), (None, None));
    assert_eq!(report.normalised("events.pick_id"), 1);
    // Payloads that are not a JSON object.
    assert_eq!(ev(47).4, json!({}));
    assert_eq!(ev(48).4, json!({}));
    assert_eq!(report.normalised("events.payload"), 2);
    // The seed's shelf_added with a rating is kept as it is.
    assert_eq!(ev(6).4, json!({"status": "finished", "rating": 5}));
    // Club kinds without a pick (votes) still belong to the club.
    assert_eq!((ev(43).1, ev(43).3), (Some(club), None));
    let line = report.entity("events").unwrap();
    assert_eq!(
        (line.source, line.target, line.detail.as_str()),
        (
            Some(48),
            Some(48),
            "(19 club, 29 personal; 1 quote_added without their quote)"
        )
    );
    assert_eq!(
        app.scalar_i64("SELECT nextval(pg_get_serial_sequence('events', 'id'))")
            .await,
        49
    );
}

/// An unknown event kind aborts the import, naming the row.
#[sqlx::test]
async fn an_unknown_event_kind_aborts(db: PgPool) {
    let app = TestApp::new(db).await;
    let legacy = load("fresh.sql");
    legacy.exec("UPDATE events SET kind = 'book_burned' WHERE id = 5;");
    let error = import(&app, legacy.args("demo")).await.unwrap_err();
    assert!(
        error.to_string().contains("events id 5, column kind"),
        "{error}"
    );
    assert_eq!(app.scalar_i64("SELECT count(*) FROM users").await, 0);
}

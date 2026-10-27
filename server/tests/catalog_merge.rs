//! `catalog::merge_works` (#41, conventions §7.6): two works becoming one on live data, with
//! the collision rules of `merge_rules` for shelves, favourites, layouts and nominations, every
//! reference re-pointed, and the catalog fields merged under the provenance ranks.

mod common;

use bookclub_server::catalog::shared::ingest::{
    self, AuthorRecord, EditionRecord, Mode, Source, WorkGrouping, WorkRecord,
};
use bookclub_server::catalog::shared::isbn::parse_isbn;
use bookclub_server::catalog::{MergeReport, merge_works};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn insert_id(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn entry(
    app: &TestApp,
    who: &TestClient,
    work: i64,
    status: &str,
    position: i32,
    extra: &str,
) -> i64 {
    let id = insert_id(
        app,
        &format!(
            "INSERT INTO shelf_entries (user_id, work_id, status, position)
             VALUES ({}, {work}, '{status}', {position}) RETURNING id",
            who.id()
        ),
    )
    .await;
    if !extra.is_empty() {
        app.exec(&format!("UPDATE shelf_entries SET {extra} WHERE id = {id}"))
            .await;
    }
    id
}

async fn favourite(app: &TestApp, who: &TestClient, work: i64, position: i32) {
    app.exec(&format!(
        "INSERT INTO user_favorites (user_id, work_id, position) VALUES ({}, {work}, {position})",
        who.id()
    ))
    .await;
}

async fn favourites(app: &TestApp, who: &TestClient) -> Vec<(i64, i32)> {
    sqlx::query_as(
        "SELECT work_id, position FROM user_favorites WHERE user_id = $1 ORDER BY position",
    )
    .bind(who.id())
    .fetch_all(&app.db)
    .await
    .unwrap()
}

async fn shelf(app: &TestApp, who: &TestClient) -> Vec<(i64, i64, String, i32)> {
    sqlx::query_as(
        "SELECT id, work_id, status, position FROM shelf_entries WHERE user_id = $1
         ORDER BY status, position",
    )
    .bind(who.id())
    .fetch_all(&app.db)
    .await
    .unwrap()
}

/// The #41 merge test: users who have both works on their shelf and as favourites (and in
/// their layouts), and votes that nominated both. Every other reference moves too.
#[sqlx::test]
async fn merging_two_works_reconciles_shelves_favourites_and_votes(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let eve = app.user("eve").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    let (loser, loser_edition) = app
        .dnb_work("Circe", "Madeline Miller", "9783423282710")
        .await;
    let (loser, winner) = (
        loser.id,
        app.ol_work("OL1W", "Circe", "Madeline Miller").await.id,
    );
    let other = app.work("Other").await.id;
    let third = app.work("Third").await.id;

    // Shelves. ada: finished on the loser beats want_to_read on the winner.
    let ada_loser = entry(
        &app,
        &ada,
        loser,
        "finished",
        0,
        &format!(
            "rating = 4, take = 'Great', finished_at = '2025-01-02T12:00:00Z',
                  updated_at = '2025-01-02T12:00:00Z', edition_id = {}",
            loser_edition.id
        ),
    )
    .await;
    let ada_winner = entry(&app, &ada, winner, "want_to_read", 0, "").await;
    let ada_other = entry(&app, &ada, other, "finished", 1, "").await;
    let ada_third = entry(&app, &ada, third, "want_to_read", 1, "").await;
    // grace: both currently reading; the newer entry (the winner's) wins and takes the progress
    // and the edition of the other.
    let grace_loser = entry(
        &app,
        &grace,
        loser,
        "currently_reading",
        0,
        &format!(
            "progress = 30, updated_at = '2025-01-01T00:00:00Z', edition_id = {}",
            loser_edition.id
        ),
    )
    .await;
    let grace_winner = entry(
        &app,
        &grace,
        winner,
        "currently_reading",
        1,
        "updated_at = '2025-02-01T00:00:00Z'",
    )
    .await;
    // eve: only the loser.
    let eve_loser = entry(&app, &eve, loser, "want_to_read", 0, "").await;

    // ada's physical shelf holds both entries; the winner's has a custom spine.
    app.exec(&format!(
        "INSERT INTO physical_shelf_layouts (user_id, layout) VALUES ({}, '{}')",
        ada.id(),
        json!({"rows": [[ada_loser, ada_winner, "decor:lamp"], [ada_other]],
               "spines": {ada_winner.to_string(): {"color": "#112233"}}, "lights": false})
    ))
    .await;

    // Favourites.
    favourite(&app, &ada, loser, 1).await;
    favourite(&app, &ada, winner, 2).await;
    favourite(&app, &ada, third, 3).await;
    favourite(&app, &grace, winner, 1).await;
    favourite(&app, &grace, other, 2).await;
    favourite(&app, &grace, loser, 3).await;
    favourite(&app, &eve, loser, 1).await;

    // An open vote that nominated both (the loser first) and a closed one whose winner was the
    // later nomination of the loser.
    let open_vote = insert_id(
        &app,
        &format!(
            "INSERT INTO next_up_votes (club_id) VALUES ({}) RETURNING id",
            club.id
        ),
    )
    .await;
    let nominate = |vote: i64, work: i64, by: i64, at: &str| {
        format!(
            "INSERT INTO next_up_nominations (club_id, vote_id, work_id, nominated_by, created_at)
             VALUES ({}, {vote}, {work}, {by}, '{at}') RETURNING id",
            club.id
        )
    };
    let nom_loser = insert_id(
        &app,
        &nominate(open_vote, loser, ada.id(), "2025-03-01T00:00:00Z"),
    )
    .await;
    let nom_winner = insert_id(
        &app,
        &nominate(open_vote, winner, grace.id(), "2025-03-02T00:00:00Z"),
    )
    .await;
    let nom_other = insert_id(
        &app,
        &nominate(open_vote, other, ada.id(), "2025-03-03T00:00:00Z"),
    )
    .await;
    let ballot = |vote: i64, who: &TestClient, nomination: i64| {
        format!(
            "INSERT INTO next_up_ballots (club_id, vote_id, user_id, nomination_id)
             VALUES ({}, {vote}, {}, {nomination})",
            club.id,
            who.id()
        )
    };
    app.exec(&ballot(open_vote, &ada, nom_winner)).await;
    app.exec(&ballot(open_vote, &grace, nom_loser)).await;
    let closed_vote = insert_id(
        &app,
        &format!(
            "INSERT INTO next_up_votes (club_id, ended_at) VALUES ({}, now()) RETURNING id",
            club.id
        ),
    )
    .await;
    let nom_winner2 = insert_id(
        &app,
        &nominate(closed_vote, winner, grace.id(), "2025-01-01T00:00:00Z"),
    )
    .await;
    let nom_loser2 = insert_id(
        &app,
        &nominate(closed_vote, loser, ada.id(), "2025-01-05T00:00:00Z"),
    )
    .await;
    app.exec(&ballot(closed_vote, &ada, nom_loser2)).await;
    app.exec(&format!(
        "UPDATE next_up_votes SET winner_nomination_id = {nom_loser2} WHERE id = {closed_vote}"
    ))
    .await;

    // Everything else that points at a work.
    let root = insert_id(
        &app,
        &format!(
            "INSERT INTO posts (work_id, author_id, body) VALUES ({loser}, {}, 'Root') RETURNING id",
            ada.id()
        ),
    )
    .await;
    app.exec(&format!(
        "INSERT INTO posts (work_id, author_id, parent_id, body) VALUES ({loser}, {}, {root}, 'Reply');
         INSERT INTO quotes (user_id, work_id, body) VALUES ({}, {loser}, 'A line');
         INSERT INTO events (actor_id, kind, work_id) VALUES ({}, 'shelf_added', {loser});
         INSERT INTO club_picks (club_id, work_id) VALUES ({}, {loser});
         INSERT INTO reports (target_kind, work_id, reason) VALUES ('cover', {loser}, 'other');
         INSERT INTO legacy_work_keys (legacy_key, legacy_book_id, work_id, match)
             VALUES ('/works/BCdeadbeef01', 7, {loser}, 'user');",
        grace.id(),
        ada.id(),
        ada.id(),
        club.id
    ))
    .await;
    let work_edit = insert_id(
        &app,
        &format!(
            "INSERT INTO catalog_edit_requests (kind, work_id, payload)
             VALUES ('work_edit', {loser}, '{{\"subtitle\": \"Roman\"}}') RETURNING id"
        ),
    )
    .await;
    let edition_edit = insert_id(
        &app,
        &format!(
            "INSERT INTO catalog_edit_requests (kind, work_id, edition_id, payload)
             VALUES ('edition_edit', {loser}, {}, '{{\"pages\": 300}}') RETURNING id",
            loser_edition.id
        ),
    )
    .await;
    let merge_into_loser = insert_id(
        &app,
        &format!(
            "INSERT INTO catalog_edit_requests (kind, work_id, payload)
             VALUES ('work_merge', {other}, '{{\"into_work_id\": {loser}}}') RETURNING id"
        ),
    )
    .await;

    let mut conn = app.db.acquire().await.unwrap();
    let report = merge_works(&mut conn, loser, winner).await.unwrap();
    drop(conn);
    assert_eq!(
        report,
        MergeReport {
            loser,
            winner,
            shelf_moved: 1,
            shelf_merged: 2,
            favourites_moved: 1,
            favourites_merged: 2,
            nominations_moved: 1,
            nominations_merged: 2,
            editions_moved: 1,
            legacy_keys_moved: 1,
            references_moved: 6,
            requests_rejected: 1,
            ol_work_id_moved: false,
        }
    );

    // Shelves: one entry per user, on the winner, fields merged, positions dense.
    assert_eq!(
        shelf(&app, &ada).await,
        [
            (ada_loser, winner, "finished".to_owned(), 0),
            (ada_other, other, "finished".to_owned(), 1),
            (ada_third, third, "want_to_read".to_owned(), 0),
        ]
    );
    assert_eq!(
        shelf(&app, &grace).await,
        [(grace_winner, winner, "currently_reading".to_owned(), 0)]
    );
    let merged: (Option<i32>, Option<i64>) =
        sqlx::query_as("SELECT progress, edition_id FROM shelf_entries WHERE id = $1")
            .bind(grace_winner)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(merged, (Some(30), Some(loser_edition.id)));
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM shelf_entries WHERE id IN ({ada_winner}, {grace_loser})"
        ))
        .await,
        0
    );
    assert_eq!(
        shelf(&app, &eve).await,
        [(eve_loser, winner, "want_to_read".to_owned(), 0)]
    );
    let layout: Value =
        sqlx::query_scalar("SELECT layout FROM physical_shelf_layouts WHERE user_id = $1")
            .bind(ada.id())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        layout,
        json!({"rows": [[ada_loser, "decor:lamp"], [ada_other]],
               "spines": {ada_loser.to_string(): {"color": "#112233"}}, "lights": false})
    );

    // Favourites: the lower position is kept, then 1..n.
    assert_eq!(favourites(&app, &ada).await, [(winner, 1), (third, 2)]);
    assert_eq!(favourites(&app, &grace).await, [(winner, 1), (other, 2)]);
    assert_eq!(favourites(&app, &eve).await, [(winner, 1)]);

    // Votes: the earliest nomination stays, with the other's ballots and the vote's winner.
    let noms: Vec<(i64, i64, i64)> =
        sqlx::query_as("SELECT vote_id, id, work_id FROM next_up_nominations ORDER BY vote_id, id")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(
        noms,
        [
            (open_vote, nom_loser, winner),
            (open_vote, nom_other, other),
            (closed_vote, nom_winner2, winner),
        ]
    );
    let ballots: Vec<(i64, i64, i64)> = sqlx::query_as(
        "SELECT vote_id, user_id, nomination_id FROM next_up_ballots ORDER BY vote_id, user_id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        ballots,
        [
            (open_vote, ada.id(), nom_loser),
            (open_vote, grace.id(), nom_loser),
            (closed_vote, ada.id(), nom_winner2),
        ]
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT winner_nomination_id FROM next_up_votes WHERE id = {closed_vote}"
        ))
        .await,
        nom_winner2
    );

    // References and the work itself.
    for table in [
        "posts",
        "quotes",
        "events",
        "club_picks",
        "reports",
        "editions",
        "legacy_work_keys",
    ] {
        assert_eq!(
            app.scalar_i64(&format!(
                "SELECT count(*) FROM {table} WHERE work_id = {loser}"
            ))
            .await,
            0,
            "{table}"
        );
    }
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT merged_into_id FROM works WHERE id = {loser}"
        ))
        .await,
        winner
    );
    let statuses: Vec<(i64, String, Option<i64>, Value)> = sqlx::query_as(
        "SELECT id, status, work_id, payload FROM catalog_edit_requests ORDER BY id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        statuses,
        [
            (
                work_edit,
                "rejected".to_owned(),
                Some(loser),
                json!({"subtitle": "Roman"})
            ),
            (
                edition_edit,
                "open".to_owned(),
                Some(winner),
                json!({"pages": 300})
            ),
            (
                merge_into_loser,
                "open".to_owned(),
                Some(other),
                json!({"into_work_id": winner})
            ),
        ]
    );

    // Through the API: the old id and the legacy key answer with the winner, with ada's merged
    // shelf state.
    let detail = ada.get(&format!("/api/books/works/{loser}")).await.json();
    assert_eq!(detail["id"], winner);
    assert_eq!(detail["on_shelf"], "finished");
    assert_eq!(detail["rating"], 4);
    assert_eq!(detail["take"], "Great");
    assert_eq!(detail["favorite_position"], 1);
    assert_eq!(detail["quote_count"], 1);
    assert_eq!(detail["editions_count"], 1);
    assert_eq!(
        detail["club_picks"],
        json!([{"slug": club.slug, "name": "Readers"}])
    );
    assert_eq!(
        ada.get("/api/books/works/BCdeadbeef01").await.json()["id"],
        winner
    );
    // The merged-away work is gone from search.
    let local = app
        .scalar_i64(&format!(
            "SELECT count(*) FROM works WHERE id = {loser} AND merged_into_id IS NULL"
        ))
        .await;
    assert_eq!(local, 0);
}

/// The winner's record stays the authority: its empty fields are filled from the loser, its
/// legacy bare facts give way to the loser's source values, a reader's value on it stays, the
/// loser's Open Library id moves over (the legacy work becomes an Open Library work) and the
/// derived columns follow the editions it gained.
#[sqlx::test]
async fn merging_fills_the_winner_and_moves_the_open_library_id(db: PgPool) {
    let app = TestApp::new(db).await;
    let mut conn = app.db.acquire().await.unwrap();
    let (winner, _) = ingest::upsert_edition_with_work(
        &mut conn,
        &WorkRecord {
            author_names: "M. Miller".into(),
            pages: Some(300),
            ..WorkRecord::new(Source::Legacy, "circe")
        },
        &EditionRecord {
            isbn13: parse_isbn("9780316556347"),
            ..EditionRecord::new(Source::Legacy)
        },
        WorkGrouping::NewWork,
        Mode::Refresh,
    )
    .await
    .unwrap();
    let (loser, _) = ingest::upsert_edition_with_work(
        &mut conn,
        &WorkRecord {
            ol_work_id: Some("OL1W".into()),
            subtitle: Some("Ein Roman".into()),
            authors: vec![AuthorRecord {
                ol_author_id: Some("OL2A".into()),
                ..AuthorRecord::named("Madeline Miller")
            }],
            description: Some("A witch.".into()),
            subjects: Some(vec!["Mythology".into()]),
            first_publish_year: Some(2018),
            ol_rating: Some(4.27),
            ol_cover_id: Some(555),
            full_details: true,
            ..WorkRecord::new(Source::OpenLibrary, "Circe")
        },
        &EditionRecord {
            isbn13: parse_isbn("9780316769488"),
            title: Some("Circe (Paperback)".into()),
            ..EditionRecord::new(Source::OpenLibrary)
        },
        WorkGrouping::NewWork,
        Mode::Refresh,
    )
    .await
    .unwrap();
    // A reader's subtitle on the winner (rank user) outranks the loser's.
    sqlx::query(
        "UPDATE works SET subtitle = 'A Novel',
                field_sources = field_sources || '{\"subtitle\": \"user\"}' WHERE id = $1",
    )
    .bind(winner)
    .execute(&mut *conn)
    .await
    .unwrap();

    let report = merge_works(&mut conn, loser, winner).await.unwrap();
    assert!(report.ol_work_id_moved);
    assert_eq!(report.editions_moved, 1);
    let row: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object(
                    'title', title, 'subtitle', subtitle, 'author_names', author_names,
                    'description', description, 'subjects', subjects,
                    'first_publish_year', first_publish_year, 'pages', pages,
                    'ol_work_id', ol_work_id, 'source', source,
                    'fetched', details_fetched_at IS NOT NULL,
                    'title_source', field_sources ->> 'title',
                    'subtitle_source', field_sources ->> 'subtitle',
                    'description_source', field_sources ->> 'description')
         FROM works WHERE id = $1",
    )
    .bind(winner)
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(
        row,
        json!({
            "title": "Circe", "subtitle": "A Novel", "author_names": "Madeline Miller",
            "description": "A witch.", "subjects": ["Mythology"], "first_publish_year": 2018,
            "pages": 300, "ol_work_id": "OL1W", "source": "openlibrary", "fetched": true,
            "title_source": "openlibrary", "subtitle_source": "user",
            "description_source": "openlibrary"
        })
    );
    let cover: Option<i64> = sqlx::query_scalar(
        "SELECT m.ol_cover_id FROM works w JOIN media m ON m.id = w.cover_media_id WHERE w.id = $1",
    )
    .bind(winner)
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(cover, Some(555));
    let derived: (String, String) =
        sqlx::query_as("SELECT search_extra, match_key FROM works WHERE id = $1")
            .bind(winner)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(derived.0, "Circe (Paperback)");
    assert_eq!(derived.1, "circe\u{1f}miller");
    let authors: i64 = sqlx::query_scalar("SELECT count(*) FROM work_authors WHERE work_id = $1")
        .bind(winner)
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(authors, 1);
    let loser_ol: Option<String> = sqlx::query_scalar("SELECT ol_work_id FROM works WHERE id = $1")
        .bind(loser)
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(loser_ol, None);
    // The Open Library id now finds the winner.
    let found = ingest::upsert_work(
        &mut conn,
        &WorkRecord {
            ol_work_id: Some("OL1W".into()),
            ..WorkRecord::new(Source::OpenLibrary, "Circe")
        },
        Mode::InsertOnly,
    )
    .await
    .unwrap();
    assert_eq!(found, winner);
}

/// A work cannot be merged into itself, into a missing work, or after it was merged away.
#[sqlx::test]
async fn merge_works_refuses_bad_pairs(db: PgPool) {
    let app = TestApp::new(db).await;
    let a = app.work("A").await.id;
    let b = app.work("B").await.id;
    let c = app.work("C").await.id;
    let mut conn = app.db.acquire().await.unwrap();
    assert!(merge_works(&mut conn, a, a).await.is_err());
    assert!(matches!(
        merge_works(&mut conn, a, 424242).await,
        Err(sqlx::Error::RowNotFound)
    ));
    merge_works(&mut conn, a, b).await.unwrap();
    assert!(matches!(
        merge_works(&mut conn, a, c).await,
        Err(sqlx::Error::RowNotFound)
    ));
    // A later merge of the survivor re-points the earlier redirect in one step.
    merge_works(&mut conn, b, c).await.unwrap();
    let redirect: Option<i64> =
        sqlx::query_scalar("SELECT merged_into_id FROM works WHERE id = $1")
            .bind(a)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(redirect, Some(c));
}

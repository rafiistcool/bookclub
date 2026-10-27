//! Personal stats (`GET /api/stats`) and club stats (`GET /api/clubs/{slug}/stats`): the year
//! in review on one `MemberYear` rule set (endpoint-map §7, §9, docs/phase1/spec/diary.md
//! §1.11, R23; #44).

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;

/// Puts `work` on the client's shelf with the extra body fields; asserts 201.
async fn add(client: &TestClient, work: &TestWork, extra: Value) -> Value {
    let mut body = json!({ "work_id": work.id });
    if let (Some(body), Value::Object(extra)) = (body.as_object_mut(), extra) {
        body.extend(extra);
    }
    let res = client.post("/api/shelf", body).await;
    res.assert_status(201);
    res.json()
}

async fn get(client: &TestClient, path: &str) -> Value {
    let res = client.get(path).await;
    res.assert_status(200);
    res.json()
}

/// A pick of `work` in `club`, set by `by` (`None`: a deleted account).
async fn pick(app: &TestApp, club: &TestClub, work: &TestWork, by: Option<&TestClient>) -> i64 {
    app.scalar_i64(&format!(
        "INSERT INTO club_picks (club_id, work_id, set_by, ended_at)
         VALUES ({}, {}, {}, now()) RETURNING id",
        club.id,
        work.id,
        by.map_or("NULL".to_owned(), |by| by.id().to_string())
    ))
    .await
}

fn this_year() -> i32 {
    OffsetDateTime::now_utc().year()
}

/// The `MemberYear` part of personal stats.
fn member_part(stats: &Value) -> Value {
    let mut member = json!({});
    for key in [
        "username",
        "finished",
        "dnf",
        "pages",
        "average_rating",
        "five_stars",
        "top_book",
        "longest_book",
        "quotes",
    ] {
        member[key] = stats[key].clone();
    }
    member
}

/// parity: test_stats_milestones_quotes.py::test_stats_year_in_review
/// The legacy club-wide `/api/stats` is now two endpoints: the member numbers come from
/// `GET /api/stats` (personal) and from `members` of `GET /api/clubs/{slug}/stats`, the picks
/// and club totals from the club stats. The pick is inserted directly (picks are CLUB's).
#[sqlx::test]
async fn stats_year_in_review(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let achilles = app
        .ol_work("OL2W", "The Song of Achilles", "Madeline Miller")
        .await;
    let dune = app.ol_work("OL3W", "Dune", "Frank Herbert").await;
    add(
        &ada,
        &circe,
        json!({"status": "finished", "rating": 5, "take": "Yes"}),
    )
    .await;
    add(&ada, &achilles, json!({"status": "finished", "rating": 3})).await;
    add(
        &ada,
        &dune,
        json!({"status": "did_not_finish", "dnf_reason": "Sand"}),
    )
    .await;
    let last_year = this_year() - 1;
    app.exec(&format!(
        "UPDATE works SET pages = 393 WHERE id = {circe};
         UPDATE works SET pages = 352 WHERE id = {achilles};
         UPDATE shelf_entries SET finished_at = make_timestamptz({last_year}, 6, 1, 0, 0, 0, 'UTC')
         WHERE work_id = {achilles}",
        circe = circe.id,
        achilles = achilles.id
    ))
    .await;
    pick(&app, &club, &circe, Some(&ada)).await;
    ada.post(
        "/api/quotes",
        json!({"work_id": circe.id, "body": "Humbling women seems to me a chief pastime of poets."}),
    )
    .await
    .assert_status(201);

    let personal = get(&ada, "/api/stats").await;
    assert_eq!(personal["year"], this_year());
    assert_eq!(personal["years"], json!([this_year(), last_year]));
    assert_eq!(personal["username"], "ada");
    assert_eq!(personal["finished"], 1);
    assert_eq!(personal["dnf"], 1);
    assert_eq!(personal["pages"], 393);
    assert_eq!(personal["average_rating"], 5.0);
    assert_eq!(personal["five_stars"], 1);
    assert_eq!(personal["top_book"]["title"], "Circe");
    assert_eq!(personal["longest_book"]["title"], "Circe");
    assert_eq!(personal["quotes"], 1);
    assert_eq!(personal["by_month"].as_array().unwrap().len(), 1);
    assert_eq!(personal["by_month"][0]["finished"], 1);

    let path = format!("/api/clubs/{}/stats", club.slug);
    let body = get(&ada, &path).await;
    assert_eq!(body["year"], this_year());
    assert_eq!(body["years"], json!([this_year(), last_year]));
    assert_eq!(body["members"][0], member_part(&personal));
    assert_eq!(body["club_finished"], 1);
    assert_eq!(body["club_pages"], 393);
    assert_eq!(body["club_average_rating"], 5.0);
    assert_eq!(body["by_month"], personal["by_month"]);
    assert_eq!(body["picks"][0]["book"]["title"], "Circe");
    assert_eq!(body["picks"][0]["finished"], 1);
    assert_eq!(body["picks"][0]["average_rating"], 5.0);
    assert_eq!(body["picks"][0]["ratings"], json!([5]));
    assert_eq!(body["picks"][0]["set_by"], "ada");
    assert_eq!(body["best_pick"]["book"]["title"], "Circe");

    let personal = get(&ada, &format!("/api/stats?year={last_year}")).await;
    assert_eq!(personal["finished"], 1);
    assert_eq!(personal["top_book"]["title"], "The Song of Achilles");
    assert_eq!(personal["pages"], 352);
    let body = get(&ada, &format!("{path}?year={last_year}")).await;
    assert_eq!(body["members"][0]["finished"], 1);
    assert_eq!(
        body["members"][0]["top_book"]["title"],
        "The Song of Achilles"
    );
    assert_eq!(body["club_pages"], 352);
}

/// parity: test_stats_milestones_quotes.py::test_stats_with_empty_club
#[sqlx::test]
async fn stats_with_empty_club(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    let empty = json!({
        "username": "ada",
        "finished": 0,
        "dnf": 0,
        "pages": 0,
        "average_rating": null,
        "five_stars": 0,
        "top_book": null,
        "longest_book": null,
        "quotes": 0
    });

    let personal = get(&ada, "/api/stats").await;
    assert_eq!(member_part(&personal), empty);
    assert_eq!(personal["year"], this_year());
    assert_eq!(personal["years"], json!([this_year()]));
    assert_eq!(personal["by_month"], json!([]));

    let body = get(&ada, &format!("/api/clubs/{}/stats", club.slug)).await;
    assert_eq!(body["members"], json!([empty]));
    assert_eq!(body["best_pick"], Value::Null);
    assert_eq!(body["by_month"], json!([]));
    assert_eq!(body["picks"], json!([]));
    assert_eq!(body["club_average_rating"], Value::Null);
}

/// Pages are the entry edition's, else the work's, else 0; the longest book shows the edition.
#[sqlx::test]
async fn pages_come_from_the_edition_then_the_work(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = app.work("Circe").await;
    let edition = app.edition(&circe, "9780316556347").await;
    let dune = app.work("Dune").await;
    let blank = app.work("Blank").await;
    app.exec(&format!(
        "UPDATE works SET pages = 300 WHERE id IN ({}, {});
         UPDATE editions SET pages = 500, title = 'Circe (large print)' WHERE id = {}",
        circe.id, dune.id, edition.id
    ))
    .await;
    add(
        &ada,
        &circe,
        json!({"status": "finished", "edition_id": edition.id}),
    )
    .await;
    add(&ada, &dune, json!({"status": "finished"})).await;
    add(&ada, &blank, json!({"status": "finished"})).await;
    let stats = get(&ada, "/api/stats").await;
    assert_eq!(stats["finished"], 3);
    assert_eq!(stats["pages"], 800);
    assert_eq!(stats["longest_book"]["title"], "Circe (large print)");
    assert_eq!(stats["longest_book"]["pages"], 500);
    assert_eq!(stats["average_rating"], Value::Null);
    assert_eq!(stats["top_book"], Value::Null);
}

/// Rounding, five stars and the tie-breaks of `top_book` and `longest_book`.
#[sqlx::test]
async fn ratings_round_half_to_even_and_ties_break_like_the_legacy_app(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    // 33/8 = 4.125 → 4.12 (banker's rounding on the binary value).
    let ratings = [5, 5, 5, 5, 5, 4, 3, 1];
    let mut works = Vec::new();
    for (n, rating) in ratings.iter().enumerate() {
        let work = app.work(&format!("Book {n}")).await;
        add(&ada, &work, json!({"status": "finished", "rating": rating})).await;
        works.push(work);
    }
    app.exec(&format!(
        "UPDATE works SET pages = 200 WHERE id IN ({}, {})",
        works[6].id, works[7].id
    ))
    .await;
    let stats = get(&ada, "/api/stats").await;
    assert_eq!(stats["average_rating"], 4.12);
    assert_eq!(stats["five_stars"], 5);
    // The highest rating with the newest entry; the most pages with the oldest entry.
    assert_eq!(stats["top_book"]["title"], "Book 4");
    assert_eq!(stats["longest_book"]["title"], "Book 6");
}

/// A DNF counts in the year of its last change; a year without data is fine; the year is
/// 2000..2100.
#[sqlx::test]
async fn years_and_their_bounds(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let dune = app.work("Dune").await;
    add(&ada, &dune, json!({"status": "did_not_finish"})).await;
    app.exec("UPDATE shelf_entries SET updated_at = '2021-03-01T00:00:00Z'")
        .await;
    let stats = get(&ada, "/api/stats").await;
    assert_eq!(stats["years"], json!([this_year(), 2021]));
    assert_eq!(stats["dnf"], 0);
    assert_eq!(get(&ada, "/api/stats?year=2021").await["dnf"], 1);
    let future = get(&ada, "/api/stats?year=2050").await;
    assert_eq!(future["year"], 2050);
    assert_eq!(future["dnf"], 0);
    let club = app.club(&ada, "Readers").await;
    for path in [
        "/api/stats".to_owned(),
        format!("/api/clubs/{}/stats", club.slug),
    ] {
        ada.get(&format!("{path}?year=1999"))
            .await
            .assert_detail(400, "Input should be greater than or equal to 2000");
        ada.get(&format!("{path}?year=2101"))
            .await
            .assert_detail(400, "Input should be less than or equal to 2100");
        ada.get(&format!("{path}?year=soon")).await.assert_detail(
            400,
            "Input should be a valid integer, unable to parse string as an integer",
        );
    }
}

/// Club stats cover the club's members and picks only: another club's member who read the
/// pick is not counted. Readers count any status; members sort by books finished, then name;
/// a pick whose setter is gone shows `unknown`; the best pick prefers the mean, then more
/// ratings.
#[sqlx::test]
async fn club_stats_cover_the_club_only(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let bob = app.user("bob").await;
    let zoe = app.user("zoe").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    app.join(&club, &zoe, "member").await;
    let other = app.club(&ada, "Other").await;
    app.join(&other, &bob, "member").await;

    let circe = app.work("Circe").await;
    let dune = app.work("Dune").await;
    let emma = app.work("Emma").await;
    let first = pick(&app, &club, &circe, None).await;
    let second = pick(&app, &club, &dune, Some(&grace)).await;
    let elsewhere = pick(&app, &other, &emma, Some(&ada)).await;

    add(&ada, &circe, json!({"status": "finished", "rating": 4})).await;
    add(&grace, &circe, json!({"status": "want_to_read"})).await;
    add(&zoe, &circe, json!({"status": "did_not_finish"})).await;
    add(&bob, &circe, json!({"status": "finished", "rating": 1})).await;
    add(&grace, &dune, json!({"status": "finished", "rating": 4})).await;
    add(&zoe, &dune, json!({"status": "finished", "rating": 4})).await;
    add(&zoe, &emma, json!({"status": "finished", "rating": 5})).await;

    let body = get(&grace, &format!("/api/clubs/{}/stats", club.slug)).await;
    let members: Vec<(&str, i64)> = body["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["username"].as_str().unwrap(),
                m["finished"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(members, vec![("zoe", 2), ("ada", 1), ("grace", 1)]);
    assert_eq!(body["club_finished"], 4);
    // 4, 4, 4, 5 → 4.25
    assert_eq!(body["club_average_rating"], 4.25);

    let picks = body["picks"].as_array().unwrap();
    let ids: Vec<i64> = picks
        .iter()
        .map(|p| p["pick_id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, vec![second, first]);
    assert!(!ids.contains(&elsewhere));
    let circe_pick = &picks[1];
    assert_eq!(circe_pick["set_by"], "unknown");
    assert_eq!(circe_pick["readers"], 3);
    assert_eq!(circe_pick["finished"], 1);
    assert_eq!(circe_pick["dnf"], 1);
    assert_eq!(circe_pick["ratings"], json!([4]));
    assert_eq!(circe_pick["average_rating"], 4.0);
    assert!(circe_pick["ended_at"].as_str().unwrap().ends_with('Z'));
    let dune_pick = &picks[0];
    assert_eq!(dune_pick["set_by"], "grace");
    assert_eq!(dune_pick["ratings"], json!([4, 4]));
    // Same mean, more ratings.
    assert_eq!(body["best_pick"]["pick_id"], second);
}

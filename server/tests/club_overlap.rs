//! Books two or more club members want to read (#45, docs/phase1/spec/club.md §1.23, §2.10).
//! Shelf entries are inserted with SQL (SHELF's routes are another package's).

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// `ada` owns "Readers"; `grace` is a member; Circe and Achilles are in the catalog.
struct Fixture {
    app: TestApp,
    ada: TestClient,
    grace: TestClient,
    club: TestClub,
    circe: TestWork,
    achilles: TestWork,
}

impl Fixture {
    async fn new(db: PgPool) -> Self {
        let app = TestApp::new(db).await;
        let ada = app.user("ada").await;
        let grace = app.user("grace").await;
        let club = app.club(&ada, "Readers").await;
        app.join(&club, &grace, "member").await;
        let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
        let achilles = app
            .ol_work("OL2W", "The Song of Achilles", "Madeline Miller")
            .await;
        Fixture {
            app,
            ada,
            grace,
            club,
            circe,
            achilles,
        }
    }

    async fn shelve(&self, who: &TestClient, work: &TestWork, status: &str) {
        self.app
            .exec(&format!(
                "INSERT INTO shelf_entries (user_id, work_id, status) VALUES ({}, {}, '{status}')",
                who.id(),
                work.id
            ))
            .await;
    }

    async fn overlap(&self, who: &TestClient, query: &str) -> Value {
        let res = who
            .get(&format!("/api/clubs/{}/overlap{query}", self.club.slug))
            .await;
        res.assert_status(200);
        res.json()
    }
}

fn titles(body: &Value) -> Vec<String> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["book"]["title"].as_str().unwrap().to_owned())
        .collect()
}

/// parity: test_overlap.py::test_overlap_requires_auth
#[sqlx::test]
async fn overlap_requires_a_session(db: PgPool) {
    let f = Fixture::new(db).await;
    f.app
        .anon()
        .get(&format!("/api/clubs/{}/overlap", f.club.slug))
        .await
        .assert_detail(401, "Not signed in");
}

/// parity: test_overlap.py::test_two_want_same_book_shows_one_does_not
#[sqlx::test]
async fn a_book_two_members_want_is_shown(db: PgPool) {
    let f = Fixture::new(db).await;
    f.shelve(&f.ada, &f.circe, "want_to_read").await;
    f.shelve(&f.ada, &f.achilles, "want_to_read").await;
    f.shelve(&f.grace, &f.circe, "want_to_read").await;
    let overlap = f.overlap(&f.grace, "").await;
    assert_eq!(overlap["include_reading"], false);
    assert_eq!(titles(&overlap), ["Circe"]);
    let row = &overlap["items"][0];
    assert_eq!(row["count"], 2);
    assert_eq!(row["book"]["id"], f.circe.id);
    assert_eq!(
        row["members"],
        json!([
            {"username": "ada", "status": "want_to_read"},
            {"username": "grace", "status": "want_to_read"}
        ])
    );
}

/// parity: test_overlap.py::test_finished_and_solo_want_are_excluded
#[sqlx::test]
async fn finished_books_and_solo_wants_are_left_out(db: PgPool) {
    let f = Fixture::new(db).await;
    f.shelve(&f.ada, &f.circe, "finished").await;
    f.shelve(&f.grace, &f.circe, "want_to_read").await;
    assert_eq!(f.overlap(&f.ada, "").await["items"], json!([]));
    f.shelve(&f.ada, &f.achilles, "did_not_finish").await;
    f.shelve(&f.grace, &f.achilles, "want_to_read").await;
    assert_eq!(
        f.overlap(&f.ada, "?include_reading=true").await["items"],
        json!([])
    );
}

/// parity: test_overlap.py::test_include_reading_counts_want_plus_reading
#[sqlx::test]
async fn include_reading_counts_reading_too(db: PgPool) {
    let f = Fixture::new(db).await;
    f.shelve(&f.ada, &f.circe, "currently_reading").await;
    f.shelve(&f.grace, &f.circe, "want_to_read").await;
    assert_eq!(f.overlap(&f.grace, "").await["items"], json!([]));
    let shown = f.overlap(&f.grace, "?include_reading=True").await;
    assert_eq!(shown["include_reading"], true);
    assert_eq!(titles(&shown), ["Circe"]);
    assert_eq!(shown["items"][0]["count"], 2);
    assert_eq!(
        shown["items"][0]["members"],
        json!([
            {"username": "ada", "status": "currently_reading"},
            {"username": "grace", "status": "want_to_read"}
        ])
    );
}

/// `include_reading` takes pydantic's lax booleans; anything else is 400.
#[sqlx::test]
async fn include_reading_is_a_lax_boolean(db: PgPool) {
    let f = Fixture::new(db).await;
    for (value, expected) in [
        ("1", true),
        ("yes", true),
        ("ON", true),
        ("t", true),
        ("Y", true),
        ("0", false),
        ("no", false),
        ("off", false),
        ("F", false),
        ("n", false),
    ] {
        let body = f
            .overlap(&f.ada, &format!("?include_reading={value}"))
            .await;
        assert_eq!(body["include_reading"], expected, "{value}");
    }
    for value in ["", "maybe", "2"] {
        f.ada
            .get(&format!(
                "/api/clubs/{}/overlap?include_reading={value}",
                f.club.slug
            ))
            .await
            .assert_detail(
                400,
                "Input should be a valid boolean, unable to interpret input",
            );
    }
}

/// Only the club's members count, the viewer need not be one of them, and books are ordered by
/// members, then title.
#[sqlx::test]
async fn overlap_covers_club_members_only(db: PgPool) {
    let f = Fixture::new(db).await;
    let zed = f.app.user("zed").await;
    f.app.join(&f.club, &zed, "member").await;
    let outsider = f.app.user("outsider").await;
    let other = f.app.club(&outsider, "Other").await;
    f.app.join(&other, &f.ada, "member").await;
    let wolf = f.app.work("the Wolf Den").await;
    for who in [&f.grace, &zed] {
        f.shelve(who, &f.circe, "want_to_read").await;
        f.shelve(who, &wolf, "want_to_read").await;
    }
    f.shelve(&f.grace, &f.achilles, "want_to_read").await;
    f.shelve(&f.ada, &wolf, "want_to_read").await;
    // The outsider shares a club with ada but is no member of this one.
    f.shelve(&outsider, &f.achilles, "want_to_read").await;
    let body = f.overlap(&f.ada, "").await;
    assert_eq!(titles(&body), ["the Wolf Den", "Circe"]);
    assert_eq!(body["items"][0]["count"], 3);
    let names: Vec<&str> = body["items"][1]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["username"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["grace", "zed"]);
    // In the other club, the outsider and ada do not overlap on anything.
    let theirs = outsider
        .get(&format!("/api/clubs/{}/overlap", other.slug))
        .await
        .json();
    assert_eq!(theirs["items"], json!([]));
    outsider
        .get(&format!("/api/clubs/{}/overlap", f.club.slug))
        .await
        .assert_detail(404, "No such club");
}

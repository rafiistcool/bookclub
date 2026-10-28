//! The direct admin edit (#54, endpoint-map §4): `PATCH /api/admin/works/{work_id}` and
//! `PATCH /api/admin/editions/{edition_id}` write through `ingest::apply_admin_fields`, like an
//! approved edit request: the fields get provenance `admin`, no refresh or harvest overwrites
//! them, and invalid input gets the approval's 400s.

mod common;

use bookclub_server::catalog::shared::ingest::{
    self, AuthorRecord, EditionRecord, Mode, Source, WorkGrouping, WorkRecord,
};
use bookclub_server::catalog::shared::isbn::parse_isbn;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

/// Open Library knows OL1W "Circe": its work JSON and its search doc, with `subjects`.
async fn mount_circe(app: &TestApp, subjects: Value) {
    Mock::given(method("GET"))
        .and(path("/works/OL1W.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "title": "Circe",
            "description": "Open Library's description.",
            "subjects": subjects
        })))
        .mount(&app.upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", "key:/works/OL1W"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"numFound": 1, "docs": [{
                "key": "/works/OL1W", "title": "Circe", "author_name": ["Madeline Miller"],
                "first_publish_year": 2018, "number_of_pages_median": 393
            }]})),
        )
        .mount(&app.upstream)
        .await;
}

/// The provenance of `field` on a works or editions row.
async fn source_of(app: &TestApp, table: &str, id: i64, field: &str) -> Option<String> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT field_sources ->> '{field}' FROM {table} WHERE id = $1"
    )))
    .bind(id)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// A DNB record of "Der Hobbit" with one edition, as the harvester ingests it.
async fn harvest_hobbit(app: &TestApp, title: &str, publisher: &str, year: i32) -> (i64, i64) {
    let work = WorkRecord {
        author_names: "J. R. R. Tolkien".into(),
        authors: vec![AuthorRecord::named("J. R. R. Tolkien")],
        ..WorkRecord::new(Source::Dnb, title)
    };
    let edition = EditionRecord {
        isbn13: parse_isbn("9783608938289"),
        publisher: Some(publisher.into()),
        pages: Some(300),
        publication_year: Some(year),
        ..EditionRecord::new(Source::Dnb)
    };
    let mut conn = app.db.acquire().await.unwrap();
    ingest::upsert_edition_with_work(
        &mut conn,
        &work,
        &edition,
        WorkGrouping::DnbTitleAuthor,
        Mode::Replace,
    )
    .await
    .unwrap()
}

/// Acceptance (#54): an admin's values on a work get provenance `admin`, and a later refresh
/// leaves them alone while it still updates the fields nobody edited. The answer is the book as
/// it is now; the work is named by its id or by its Open Library key.
#[sqlx::test]
async fn an_admin_edit_of_a_work_survives_a_refresh(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_circe(&app, json!(["Mythology"])).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let imported = ada.get("/api/books/works/OL1W").await.json();
    let work = imported["id"].as_i64().unwrap();
    assert_eq!(imported["description"], "Open Library's description.");

    let res = root
        .patch(
            "/api/admin/works/OL1W",
            json!({"fields": {"title": "Circe (corrected)", "pages": 400, "subtitle": "A Novel"}}),
        )
        .await;
    res.assert_status(200);
    let body = res.json();
    assert_eq!(body["id"], work);
    assert_eq!(body["title"], "Circe (corrected)");
    assert_eq!(body["subtitle"], "A Novel");
    assert_eq!(body["pages"], 400);
    for field in ["title", "pages", "subtitle"] {
        assert_eq!(
            source_of(&app, "works", work, field).await.as_deref(),
            Some("admin"),
            "{field}"
        );
    }
    let res = root
        .patch(
            &format!("/api/admin/works/{work}"),
            json!({"fields": {"year": 2019}}),
        )
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["year"], 2019);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM catalog_edit_requests")
            .await,
        0,
        "a direct edit files no request"
    );

    // Open Library changes its subjects; the refresh takes them, nothing the admin set.
    app.upstream.reset().await;
    mount_circe(&app, json!(["Witches"])).await;
    let res = ada
        .post(&format!("/api/books/works/{work}/refresh"), json!({}))
        .await;
    res.assert_status(200);
    let body = res.json();
    assert_eq!(body["title"], "Circe (corrected)");
    assert_eq!(body["subtitle"], "A Novel");
    assert_eq!(body["pages"], 400);
    assert_eq!(body["year"], 2019);
    assert_eq!(body["subjects"], json!(["Witches"]));
}

/// Acceptance (#54): an admin's values on an edition and its work outlast a DNB harvest of the
/// same record, which still updates the fields nobody edited. The edition answer is the edition.
#[sqlx::test]
async fn an_admin_edit_of_an_edition_survives_a_harvest(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let (work, edition) = harvest_hobbit(&app, "Der Hobbit", "Klett", 1997).await;

    let res = root
        .patch(
            &format!("/api/admin/editions/{edition}"),
            json!({"fields": {"publisher": "Klett-Cotta", "pages": 380, "format": "hardcover",
                              "planned_publication_date": null}}),
        )
        .await;
    res.assert_status(200);
    let body = res.json();
    assert_eq!(body["id"], edition);
    assert_eq!(body["work_id"], work);
    assert_eq!(body["publisher"], "Klett-Cotta");
    assert_eq!(body["pages"], 380);
    assert_eq!(body["format"], "hardcover");
    assert_eq!(body["isbn13"], "9783608938289");
    root.patch(
        &format!("/api/admin/works/{work}"),
        json!({"fields": {"title": "Der kleine Hobbit"}}),
    )
    .await
    .assert_status(200);
    for field in ["publisher", "pages", "format"] {
        assert_eq!(
            source_of(&app, "editions", edition, field).await.as_deref(),
            Some("admin"),
            "{field}"
        );
    }

    let (again_work, again_edition) = harvest_hobbit(&app, "Der Hobbit", "Klett", 1998).await;
    assert_eq!((again_work, again_edition), (work, edition));
    let row: (String, i32, Option<String>, Option<i32>, String) = sqlx::query_as(
        "SELECT e.publisher, e.pages, e.format, e.publication_year, w.title
         FROM editions e JOIN works w ON w.id = e.work_id WHERE e.id = $1",
    )
    .bind(edition)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            "Klett-Cotta".into(),
            380,
            Some("hardcover".into()),
            Some(1998),
            "Der kleine Hobbit".into()
        )
    );
}

/// Acceptance (#54): invalid input gets the same 400s as approving an edit request with that
/// payload, and changes nothing; an empty edit is `Field required`; unknown books are 404.
#[sqlx::test]
async fn invalid_admin_edits_get_the_approvals_400s(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let work = app.work("My Zine").await;
    let edition = app.edition(&work, "9780316556347").await;
    let before: Value = sqlx::query_scalar(
        "SELECT jsonb_build_array(
             (SELECT to_jsonb(w) FROM works w WHERE w.id = $1),
             (SELECT to_jsonb(e) FROM editions e WHERE e.id = $2))",
    )
    .bind(work.id)
    .bind(edition.id)
    .fetch_one(&app.db)
    .await
    .unwrap();

    let cases = [
        (
            "work_edit",
            json!({"isbn": "9780316556347"}),
            "Unknown field: isbn",
        ),
        (
            "work_edit",
            json!({"year": "soon"}),
            "Invalid value for year",
        ),
        (
            "work_edit",
            json!({"title": null}),
            "Invalid value for title",
        ),
        (
            "work_edit",
            json!({"title": "   "}),
            "Invalid value for title",
        ),
        ("work_edit", json!({"pages": 0}), "Invalid value for pages"),
        (
            "work_edit",
            json!({"subjects": "Myth"}),
            "Invalid value for subjects",
        ),
        ("edition_edit", json!({"year": 2001}), "Unknown field: year"),
        (
            "edition_edit",
            json!({"format": "scroll"}),
            "Invalid value for format",
        ),
        (
            "edition_edit",
            json!({"language": "xx"}),
            "Invalid value for language",
        ),
        (
            "edition_edit",
            json!({"planned_publication_date": "2026-13-01"}),
            "Invalid value for planned_publication_date",
        ),
    ];
    for (kind, fields, detail) in cases {
        let (path, edition_id) = match kind {
            "work_edit" => (format!("/api/admin/works/{}", work.id), None),
            _ => (
                format!("/api/admin/editions/{}", edition.id),
                Some(edition.id),
            ),
        };
        root.patch(&path, json!({"fields": fields}))
            .await
            .assert_detail(400, detail);
        // The same payload as a request (filed directly: the request routes check it first).
        let request: i64 = sqlx::query_scalar(
            "INSERT INTO catalog_edit_requests (kind, work_id, edition_id, payload, created_by)
             VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(kind)
        .bind(work.id)
        .bind(edition_id)
        .bind(&fields)
        .bind(ada.id())
        .fetch_one(&app.db)
        .await
        .unwrap();
        root.post(
            &format!("/api/admin/edit-requests/{request}/approve"),
            json!({}),
        )
        .await
        .assert_detail(400, detail);
    }
    for path in [
        format!("/api/admin/works/{}", work.id),
        format!("/api/admin/editions/{}", edition.id),
    ] {
        root.patch(&path, json!({"fields": {}}))
            .await
            .assert_detail(400, "Field required");
        root.patch(&path, json!({}))
            .await
            .assert_detail(400, "Field required");
        root.patch(&path, json!({"fields": ["title"]}))
            .await
            .assert_status(400);
    }
    let after: Value = sqlx::query_scalar(
        "SELECT jsonb_build_array(
             (SELECT to_jsonb(w) FROM works w WHERE w.id = $1),
             (SELECT to_jsonb(e) FROM editions e WHERE e.id = $2))",
    )
    .bind(work.id)
    .bind(edition.id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(before, after, "a refused edit changed the book");

    for path in [
        "/api/admin/works/424242",
        "/api/admin/works/OL999W",
        "/api/admin/editions/424242",
    ] {
        root.patch(path, json!({"fields": {"pages": 10}}))
            .await
            .assert_detail(404, "No such book.");
    }
    app.anon()
        .patch(
            &format!("/api/admin/works/{}", work.id),
            json!({"fields": {"pages": 10}}),
        )
        .await
        .assert_detail(401, "Not signed in");
    ada.patch(
        &format!("/api/admin/works/{}", work.id),
        json!({"fields": {"pages": 10}}),
    )
    .await
    .assert_detail(404, "Not found");
}

/// A merged-away book is edited where it lives now: the work it was merged into.
#[sqlx::test]
async fn an_admin_edit_follows_a_merge(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let loser = app.work("Der Hobit").await;
    let winner = app.work("Der Hobbit").await;
    root.post(
        &format!("/api/admin/works/{}/merge", loser.id),
        json!({"into_work_id": winner.id}),
    )
    .await
    .assert_status(200);
    let res = root
        .patch(
            &format!("/api/admin/works/{}", loser.id),
            json!({"fields": {"pages": 310}}),
        )
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["id"], winner.id);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM works WHERE id = {} AND pages = 310",
            winner.id
        ))
        .await,
        1
    );
}

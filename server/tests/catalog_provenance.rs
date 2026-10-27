//! Field-level provenance at work in CAT's write paths (#41, conventions §7.2): a refresh never
//! overwrites what a reader uploaded or an admin approved, and a legacy bare-facts work is
//! upgraded by a source record for its ISBN.

mod common;

use bookclub_server::catalog::shared::ingest::{
    self, CoverTarget, EditionRecord, Mode, Source, WorkGrouping, WorkRecord,
};
use bookclub_server::catalog::shared::isbn::parse_isbn;
use bookclub_server::catalog::shared::lookup::CatalogLookup;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::matchers::{method, path, path_regex, query_param};
use wiremock::{Mock, ResponseTemplate};

const SRU_HIT: &str = include_str!("fixtures/dnb_sru/hit_9783694050797.xml");
const OL_SEARCH_CIRCE: &str = include_str!("fixtures/openlibrary/search_isbn_9780316556347.json");

/// Open Library knows OL1W: its work JSON (with `subjects`) and its search doc.
async fn mount_circe(app: &TestApp, subjects: Value) {
    Mock::given(method("GET"))
        .and(path("/works/OL1W.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "title": "Circe",
            "description": "A plain string description.",
            "subjects": subjects,
            "covers": [555]
        })))
        .mount(&app.upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", "key:/works/OL1W"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"numFound": 1, "docs": [{
                "key": "/works/OL1W", "title": "Circe", "author_name": ["Madeline Miller"],
                "cover_i": 555, "first_publish_year": 2018, "number_of_pages_median": 393
            }]})),
        )
        .mount(&app.upstream)
        .await;
}

/// A cover picture as MED stores an upload.
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

async fn cover_of(app: &TestApp, work: i64) -> (Option<i64>, Option<String>) {
    sqlx::query_as(
        "SELECT cover_media_id, field_sources ->> 'cover_media_id' FROM works WHERE id = $1",
    )
    .bind(work)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// A reader's first cover photo has provenance `user`: a refresh fills the other facts but
/// keeps the photo, although Open Library has a cover.
#[sqlx::test]
async fn a_refresh_after_a_user_upload_keeps_the_cover(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_circe(&app, json!(["Mythology"])).await;
    let ada = app.user("ada").await;
    let work = {
        let mut conn = app.db.acquire().await.unwrap();
        ingest::upsert_work(
            &mut conn,
            &WorkRecord {
                ol_work_id: Some("OL1W".into()),
                full_details: true,
                ..WorkRecord::new(Source::OpenLibrary, "Circe")
            },
            Mode::Refresh,
        )
        .await
        .unwrap()
    };
    let photo = upload_media(&app, &ada).await;
    let mut conn = app.db.acquire().await.unwrap();
    assert!(
        ingest::set_cover(
            &mut conn,
            CoverTarget::Work(work),
            photo,
            Source::User,
            true
        )
        .await
        .unwrap()
    );
    drop(conn);

    let res = ada.post("/api/books/works/OL1W/refresh", json!({})).await;
    res.assert_status(200);
    assert_eq!(res.json()["description"], "A plain string description.");
    assert_eq!(res.json()["year"], 2018);
    assert_eq!(
        cover_of(&app, work).await,
        (Some(photo), Some("user".into()))
    );
}

/// Fields and a cover an admin approved have provenance `admin`: a refresh keeps them and still
/// updates the fields nobody edited.
#[sqlx::test]
async fn a_refresh_after_an_approved_edit_keeps_the_fields(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_circe(&app, json!(["Mythology"])).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let imported = ada.get("/api/books/works/OL1W").await.json();
    let work = imported["id"].as_i64().unwrap();
    assert_eq!(imported["subjects"], json!(["Mythology"]));

    let edit = ada
        .post(
            &format!("/api/books/works/{work}/edit-requests"),
            json!({"fields": {"title": "Circe (corrected)", "description": "Corrected."}}),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    root.post(
        &format!("/api/admin/edit-requests/{edit}/approve"),
        json!({}),
    )
    .await
    .assert_status(200);
    let photo = upload_media(&app, &ada).await;
    let cover = sqlx::query_scalar::<_, i64>(
        "INSERT INTO catalog_edit_requests (kind, work_id, media_id, created_by)
         VALUES ('cover_replace', $1, $2, $3) RETURNING id",
    )
    .bind(work)
    .bind(photo)
    .bind(ada.id())
    .fetch_one(&app.db)
    .await
    .unwrap();
    root.post(
        &format!("/api/admin/edit-requests/{cover}/approve"),
        json!({}),
    )
    .await
    .assert_status(200);

    // Open Library changes its subjects; the refresh takes them, nothing else.
    app.upstream.reset().await;
    mount_circe(&app, json!(["Witches"])).await;
    let res = ada
        .post(&format!("/api/books/works/{work}/refresh"), json!({}))
        .await;
    res.assert_status(200);
    let body = res.json();
    assert_eq!(body["title"], "Circe (corrected)");
    assert_eq!(body["description"], "Corrected.");
    assert_eq!(body["subjects"], json!(["Witches"]));
    assert_eq!(
        cover_of(&app, work).await,
        (Some(photo), Some("admin".into()))
    );
}

/// A legacy bare-facts work (an unmatched legacy ISBN row) is upgraded by a source record for
/// its ISBN, as `rematch-legacy` and an added edition's ISBN check do (`resolve_isbn_remote`):
/// the edition is found by its ISBN, legacy-ranked fields take the source's values and the work
/// becomes an Open Library or DNB work. Shelf entries keep pointing at the same rows.
#[sqlx::test]
async fn a_legacy_work_is_upgraded_by_a_source_record_for_its_isbn(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", "isbn:9780316556347"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(OL_SEARCH_CIRCE.as_bytes().to_vec(), "application/json"),
        )
        .mount(&app.upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/sru/dnb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(SRU_HIT.as_bytes().to_vec(), "text/xml;charset=UTF-8"),
        )
        .mount(&app.upstream)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/isbn/"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&app.upstream)
        .await;
    let ada = app.user("ada").await;
    let mut conn = app.db.acquire().await.unwrap();
    let mut legacy = Vec::new();
    for (isbn, title) in [
        ("9780316556347", "circe"),
        ("9783694050797", "der moerder von whitechapel"),
    ] {
        let ids = ingest::upsert_edition_with_work(
            &mut conn,
            &WorkRecord {
                author_names: "Unknown".into(),
                pages: Some(300),
                ..WorkRecord::new(Source::Legacy, title)
            },
            &EditionRecord {
                isbn13: parse_isbn(isbn),
                pages: Some(300),
                ..EditionRecord::new(Source::Legacy)
            },
            WorkGrouping::NewWork,
            Mode::Refresh,
        )
        .await
        .unwrap();
        sqlx::query("INSERT INTO shelf_entries (user_id, work_id, edition_id, status) VALUES ($1, $2, $3, 'finished')")
            .bind(ada.id())
            .bind(ids.0)
            .bind(ids.1)
            .execute(&mut *conn)
            .await
            .unwrap();
        legacy.push(ids);
    }

    for ((work, edition), (isbn, source, title, authors)) in legacy.iter().zip([
        ("9780316556347", "openlibrary", "Circe", "Madeline Miller"),
        (
            "9783694050797",
            "dnb",
            "Der Mörder von Whitechapel",
            "Patrice McDonough",
        ),
    ]) {
        let resolved = app
            .state
            .catalog
            .resolve_isbn_remote(&mut conn, &parse_isbn(isbn).unwrap())
            .await
            .unwrap()
            .expect("a source knows the ISBN");
        assert_eq!(
            (resolved.work_id, resolved.edition_id),
            (*work, Some(*edition))
        );
        let row: (String, String, String, Value) = sqlx::query_as(
            "SELECT source, title, author_names, field_sources FROM works WHERE id = $1",
        )
        .bind(work)
        .fetch_one(&mut *conn)
        .await
        .unwrap();
        assert_eq!(row.0, source, "{isbn}");
        assert_eq!(row.1, title);
        assert_eq!(row.2, authors);
        assert_eq!(row.3["title"], source);
        let edition_source: String =
            sqlx::query_scalar("SELECT source FROM editions WHERE id = $1")
                .bind(edition)
                .fetch_one(&mut *conn)
                .await
                .unwrap();
        assert_eq!(edition_source, source);
        let detail = ada.get(&format!("/api/books/works/{work}")).await.json();
        assert_eq!(detail["title"], title);
        assert_eq!(detail["on_shelf"], "finished");
        assert_eq!(detail["edition_id"], *edition);
    }
    let ol_id: Option<String> = sqlx::query_scalar("SELECT ol_work_id FROM works WHERE id = $1")
        .bind(legacy[0].0)
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(ol_id.as_deref(), Some("OL18012166W"));
}

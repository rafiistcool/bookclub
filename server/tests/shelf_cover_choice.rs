//! The personal cover choice (docs/phase3/notes/cover-choice.md): `GET
//! /api/books/works/{work_id}/cover-candidates`, `PUT`/`DELETE /api/shelf/{entry_id}/cover` and
//! `PUT /api/shelf/{entry_id}/cover/photo`. A reader's choice shows on their shelf (also to the
//! friends and club mates who see it), in the previews, the year in review and their book page;
//! nobody else's book page changes, and a stranger sees nothing.

mod common;
#[path = "common_cover/mod.rs"]
mod cover;

use bookclub_server::catalog::merge_works;
use bookclub_server::catalog::shared::ingest::{self, CoverTarget, Source};
use common::*;
use cover::{jpeg, ol_media, set_cover};
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

/// Open Library answers the editions of `OL1W` and its work record.
async fn mount_open_library(app: &TestApp) {
    Mock::given(method("GET"))
        .and(path("/works/OL1W/editions.json"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "size": 4,
            "entries": [
                {"key": "/books/OL10M", "title": "Circe", "covers": [100],
                 "publishers": ["Little, Brown"], "publish_date": "April 10, 2018",
                 "languages": [{"key": "/languages/eng"}], "isbn_13": ["9780316556347"]},
                {"key": "/books/OL11M", "title": "Circe", "subtitle": "A Novel",
                 "covers": [-1, 201], "publishers": ["Bloomsbury"], "publish_date": "2019",
                 "languages": [{"key": "/languages/eng"}], "isbn_10": ["1408890089"]},
                {"key": "/books/OL12M", "title": "Ich bin Circe", "covers": [202],
                 "publishers": ["Eisele"], "publish_date": "2019",
                 "languages": [{"key": "/languages/ger"}]},
                {"key": "/books/OL13M", "title": "Circe", "covers": [-1]},
                "not an edition"
            ]
        })))
        .mount(&app.upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/works/OL1W.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "key": "/works/OL1W", "title": "Circe", "covers": [100, 203]
        })))
        .mount(&app.upstream)
        .await;
}

/// The Open Library work `OL1W` with Open Library cover 100 as its catalog cover.
async fn circe(app: &TestApp) -> i64 {
    let work = app.ol_work("OL1W", "Circe", "Madeline Miller").await.id;
    let media = ol_media(app, 100).await;
    set_cover(app, CoverTarget::Work(work), media, Source::OpenLibrary).await;
    work
}

async fn add(client: &TestClient, work: i64, status: &str) -> i64 {
    let res = client
        .post("/api/shelf", json!({"work_id": work, "status": status}))
        .await;
    res.assert_status(201);
    res.json()["id"].as_i64().unwrap()
}

async fn candidates(client: &TestClient, work: i64) -> Value {
    let res = client
        .get(&format!("/api/books/works/{work}/cover-candidates"))
        .await;
    res.assert_status(200);
    res.json()
}

fn keys(list: &Value) -> Vec<String> {
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["key"].as_str().unwrap().to_owned())
        .collect()
}

fn item<'a>(list: &'a Value, key: &str) -> &'a Value {
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["key"] == key)
        .unwrap_or_else(|| panic!("no candidate {key} in {list}"))
}

async fn public_id_of_ol(app: &TestApp, ol_cover_id: i64) -> String {
    sqlx::query_scalar("SELECT public_id FROM media WHERE ol_cover_id = $1")
        .bind(ol_cover_id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// The shelf entry of `client` for `work`, as `viewer` sees the owner's shelf.
async fn shelf_item(viewer: &TestClient, owner: &str, work: i64) -> Value {
    let res = viewer.get(&format!("/api/shelf?username={owner}")).await;
    res.assert_status(200);
    res.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["book"]["id"] == work)
        .cloned()
        .unwrap_or_else(|| panic!("{work} is not on {owner}'s shelf"))
}

async fn upload_photo(client: &TestClient, entry: i64, data: Vec<u8>) -> TestResponse {
    client
        .put_multipart(
            &format!("/api/shelf/{entry}/cover/photo"),
            vec![Part::file("copy.jpg", "image/jpeg", data)],
        )
        .await
}

/// The list: the catalog's covers first (the work's, deduplicated against Open Library's, whose
/// edition record labels it), then
/// the Open Library editions' covers with their labels, then the work record's; editions without
/// a cover are left out. Each is a lazy media row served from our domain, and the second list is
/// answered from the response cache.
#[sqlx::test]
async fn the_list_has_the_catalog_and_open_library_covers(db: PgPool) {
    let app = TestApp::new(db).await;
    mount_open_library(&app).await;
    let work = circe(&app).await;
    let ada = app.user("ada").await;

    let list = candidates(&ada, work).await;
    let catalog = format!("media:{}", public_id_of_ol(&app, 100).await);
    assert_eq!(
        keys(&list),
        [catalog.as_str(), "ol:201", "ol:202", "ol:203"]
    );
    assert_eq!(list["partial"], false);
    let first = item(&list, &catalog);
    assert_eq!(first["source"], "catalog");
    assert_eq!(first["chosen"], false);
    // Open Library's edition record labels the catalog's cover too.
    assert_eq!(first["edition_label"]["publisher"], "Little, Brown");
    assert_eq!(first["edition_label"]["year"], 2018);
    let bloomsbury = item(&list, "ol:201");
    assert_eq!(bloomsbury["source"], "openlibrary");
    assert_eq!(
        bloomsbury["edition_label"],
        json!({"title": "Circe: A Novel", "publisher": "Bloomsbury", "year": 2019,
               "language": "eng", "isbn13": "9781408890080"})
    );
    let public_id = public_id_of_ol(&app, 201).await;
    assert_eq!(bloomsbury["cover_public_id"], public_id);
    assert_eq!(bloomsbury["cover_url"], format!("/media/{public_id}/l"));
    assert_eq!(
        bloomsbury["cover_thumb_url"],
        format!("/media/{public_id}/s")
    );
    assert_eq!(item(&list, "ol:203")["edition_label"], Value::Null);

    let asked = app.upstream_requests().await.len();
    assert_eq!(asked, 2, "the editions and the work record");
    assert_eq!(candidates(&ada, work).await, list);
    assert_eq!(
        app.upstream_requests().await.len(),
        asked,
        "served from the cache"
    );
    // Nothing went to another host: every upstream URL of the test points at the stand-in.
    assert!(
        app.upstream_requests()
            .await
            .iter()
            .all(|request| request.url.path().starts_with("/works/OL1W"))
    );
}

/// A choice shows on the reader's shelf, to the friend and the club mate who see it, in the
/// friends' and the club's currently-reading previews and on the reader's book page; another
/// reader's book page keeps the standard cover, and a stranger sees no shelf at all.
#[sqlx::test]
async fn a_choice_shows_wherever_the_readers_shelf_shows(db: PgPool) {
    let app = TestApp::new(db).await;
    mount_open_library(&app).await;
    let work = circe(&app).await;
    let viewers = common::visibility::Viewers::new(&app).await;
    let owner = &viewers.owner;
    let entry = add(owner, work, "currently_reading").await;
    let standard = public_id_of_ol(&app, 100).await;
    // Before a choice, the entry shows the standard cover: that candidate is the chosen one.
    let before = candidates(owner, work).await;
    assert_eq!(item(&before, &format!("media:{standard}"))["chosen"], true);
    assert_eq!(item(&before, "ol:202")["chosen"], false);
    let chosen = public_id_of_ol(&app, 202).await;

    let res = owner
        .put(
            &format!("/api/shelf/{entry}/cover"),
            json!({"key": "ol:202"}),
        )
        .await;
    res.assert_status(200);
    let out = res.json();
    assert_eq!(out["personal_cover"], true);
    assert_eq!(out["book"]["cover_public_id"], chosen);
    assert_eq!(out["book"]["cover_url"], format!("/media/{chosen}/l"));
    assert_eq!(
        item(&candidates(owner, work).await, "ol:202")["chosen"],
        true
    );
    assert_eq!(
        item(&candidates(owner, work).await, &format!("media:{standard}"))["chosen"],
        false
    );

    for viewer in [owner, &viewers.co_member, &viewers.friend] {
        let shown = shelf_item(viewer, "owner", work).await;
        assert_eq!(
            shown["book"]["cover_public_id"],
            chosen,
            "{}",
            viewer.username()
        );
        assert_eq!(shown["personal_cover"], true);
    }
    let friends = viewers.friend.get("/api/friends").await.json();
    let preview = friends["friends"]
        .as_array()
        .unwrap()
        .iter()
        .find(|friend| friend["user"]["username"] == "owner")
        .unwrap()["currently_reading"][0]["book"]["cover_public_id"]
        .clone();
    assert_eq!(preview, chosen);
    let members = viewers
        .co_member
        .get(&format!("/api/clubs/{}/members", viewers.shared.slug))
        .await
        .json();
    let preview = members["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["username"] == "owner")
        .unwrap()["currently_reading_preview"][0]["cover_public_id"]
        .clone();
    assert_eq!(preview, chosen);

    let page = owner.get(&format!("/api/books/works/{work}")).await.json();
    assert_eq!(page["cover_public_id"], chosen);
    assert_eq!(page["personal_cover"], true);
    let theirs = viewers
        .co_member
        .get(&format!("/api/books/works/{work}"))
        .await
        .json();
    assert_eq!(theirs["cover_public_id"], standard);
    assert_eq!(theirs["personal_cover"], false);

    for stranger in viewers.strangers() {
        stranger
            .get("/api/shelf?username=owner")
            .await
            .assert_detail(404, "No member with that name");
    }

    // Back to the standard cover.
    let res = owner.delete(&format!("/api/shelf/{entry}/cover")).await;
    res.assert_status(200);
    assert_eq!(res.json()["personal_cover"], false);
    assert_eq!(res.json()["book"]["cover_public_id"], standard);
    let shown = shelf_item(&viewers.friend, "owner", work).await;
    assert_eq!(shown["book"]["cover_public_id"], standard);
}

/// The book page shows the cover of the edition on the viewer's shelf entry (it showed the
/// work's before), and the year in review shows the reader's choice.
#[sqlx::test]
async fn the_book_page_and_the_stats_show_the_readers_cover(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = circe(&app).await;
    let edition = app
        .edition(
            &TestWork {
                id: work,
                title: "Circe".into(),
            },
            "9780316556347",
        )
        .await;
    let edition_cover = ol_media(&app, 300).await;
    set_cover(
        &app,
        CoverTarget::Edition(edition.id),
        edition_cover,
        Source::OpenLibrary,
    )
    .await;
    let ada = app.user("ada").await;
    let res = ada
        .post(
            "/api/shelf",
            json!({"work_id": work, "edition_id": edition.id, "status": "finished", "rating": 5}),
        )
        .await;
    res.assert_status(201);
    let entry = res.json()["id"].as_i64().unwrap();
    let page = ada.get(&format!("/api/books/works/{work}")).await.json();
    assert_eq!(page["cover_public_id"], public_id_of_ol(&app, 300).await);
    assert_eq!(page["title"], "Circe");

    let photo = upload_photo(&ada, entry, jpeg([20, 120, 60])).await;
    photo.assert_status(200);
    let mine = photo.json()["book"]["cover_public_id"].clone();
    let stats = ada.get("/api/stats").await.json();
    assert_eq!(stats["top_book"]["cover_public_id"], mine, "{stats}");
}

/// Keys: only the two forms, an existing usable picture, and one this reader may choose for
/// this book. Someone else's entry is 404 like a missing one.
#[sqlx::test]
async fn keys_name_pictures_the_reader_may_choose(db: PgPool) {
    let app = TestApp::new(db).await;
    mount_open_library(&app).await;
    let work = circe(&app).await;
    let other = app.ol_work("OL2W", "Ariadne", "Jennifer Saint").await.id;
    let other_cover = ol_media(&app, 900).await;
    let other_cover_id: String = sqlx::query_scalar("SELECT public_id FROM media WHERE id = $1")
        .bind(other_cover)
        .fetch_one(&app.db)
        .await
        .unwrap();
    // An upload that is the other book's catalog cover (not an Open Library row).
    let upload = cover::upload_media(&app).await;
    set_cover(&app, CoverTarget::Work(other), upload, Source::User).await;
    let upload_id: String = sqlx::query_scalar("SELECT public_id FROM media WHERE id = $1")
        .bind(upload)
        .fetch_one(&app.db)
        .await
        .unwrap();
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let entry = add(&ada, work, "want_to_read").await;
    let bobs = add(&bob, work, "want_to_read").await;
    let bobs_photo =
        upload_photo(&bob, bobs, jpeg([200, 10, 10])).await.json()["book"]["cover_public_id"]
            .as_str()
            .unwrap()
            .to_owned();
    candidates(&ada, work).await;

    let choose = |key: String| {
        let ada = &ada;
        async move {
            ada.put(&format!("/api/shelf/{entry}/cover"), json!({"key": key}))
                .await
        }
    };
    for key in [
        "".to_owned(),
        "201".to_owned(),
        "ol:abc".to_owned(),
        "ol:0".to_owned(),
        "ol:999999".to_owned(),
        "media:short".to_owned(),
        format!("media:{upload_id}"),
        format!("media:{bobs_photo}"),
        "x".repeat(65),
    ] {
        choose(key.clone())
            .await
            .assert_detail(400, "This cover cannot be chosen");
    }
    // Any Open Library picture's row is fine (also by its media key), as is the catalog cover.
    choose(format!("media:{other_cover_id}"))
        .await
        .assert_status(200);
    choose("ol:201".into()).await.assert_status(200);
    choose(format!("media:{}", public_id_of_ol(&app, 100).await))
        .await
        .assert_status(200);

    bob.put(
        &format!("/api/shelf/{entry}/cover"),
        json!({"key": "ol:201"}),
    )
    .await
    .assert_detail(404, "Book not on your shelf");
    bob.delete(&format!("/api/shelf/{entry}/cover"))
        .await
        .assert_detail(404, "Book not on your shelf");
    upload_photo(&bob, entry, jpeg([1, 2, 3]))
        .await
        .assert_detail(404, "Book not on your shelf");
    ada.put("/api/shelf/999999/cover", json!({"key": "ol:201"}))
        .await
        .assert_detail(404, "Book not on your shelf");
}

/// A photo of the reader's copy is personal: it shows for them at once, files no cover request,
/// never becomes the book's cover (`set_cover` refuses it), and a report of it names the book.
#[sqlx::test]
async fn a_photo_stays_personal(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = circe(&app).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let entry = add(&ada, work, "want_to_read").await;

    let res = upload_photo(&ada, entry, jpeg([20, 120, 60])).await;
    res.assert_status(200);
    let out = res.json();
    assert_eq!(out["personal_cover"], true);
    let public_id = out["book"]["cover_public_id"].as_str().unwrap().to_owned();
    assert_ne!(public_id, public_id_of_ol(&app, 100).await);
    let (media_id, personal, uploader): (i64, bool, Option<i64>) =
        sqlx::query_as("SELECT id, personal, uploaded_by FROM media WHERE public_id = $1")
            .bind(&public_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(personal);
    assert_eq!(uploader, Some(ada.id()));
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM catalog_edit_requests")
            .await,
        0
    );
    let photo = item(&candidates(&ada, work).await, &format!("media:{public_id}")).clone();
    assert_eq!(photo["source"], "photo");
    assert_eq!(photo["chosen"], true);
    // Not a catalog cover, not even for an admin's write.
    let mut conn = app.db.acquire().await.unwrap();
    for target in [CoverTarget::Work(work)] {
        assert!(
            !ingest::set_cover(&mut conn, target, media_id, Source::Admin, false)
                .await
                .unwrap()
        );
    }
    assert_eq!(
        bob.get(&format!("/api/books/works/{work}")).await.json()["cover_public_id"],
        public_id_of_ol(&app, 100).await
    );
    // Bob cannot choose it, but he can report it, and the report names the book.
    let bobs = add(&bob, work, "want_to_read").await;
    bob.put(
        &format!("/api/shelf/{bobs}/cover"),
        json!({"key": format!("media:{public_id}")}),
    )
    .await
    .assert_detail(400, "This cover cannot be chosen");
    bob.post(
        &format!("/api/media/{public_id}/reports"),
        json!({"reason": "offensive"}),
    )
    .await
    .assert_status(201);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT work_id FROM reports WHERE media_id = {media_id}"
        ))
        .await,
        work
    );
}

/// A takedown of the chosen picture brings back the standard cover; the picture cannot be
/// chosen again.
#[sqlx::test]
async fn a_takedown_brings_back_the_standard_cover(db: PgPool) {
    let app = TestApp::new(db).await;
    mount_open_library(&app).await;
    let work = circe(&app).await;
    let root = app.admin("root").await;
    let ada = app.user("ada").await;
    let entry = add(&ada, work, "want_to_read").await;
    candidates(&ada, work).await;
    ada.put(
        &format!("/api/shelf/{entry}/cover"),
        json!({"key": "ol:201"}),
    )
    .await
    .assert_status(200);
    let taken = public_id_of_ol(&app, 201).await;
    root.delete(&format!("/api/admin/media/{taken}"))
        .await
        .assert_status(204);

    let shown = shelf_item(&ada, "ada", work).await;
    assert_eq!(shown["personal_cover"], false);
    assert_eq!(
        shown["book"]["cover_public_id"],
        public_id_of_ol(&app, 100).await
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM shelf_entries WHERE id = {entry} AND cover_media_id IS NOT NULL"
        ))
        .await,
        0
    );
    assert!(!keys(&candidates(&ada, work).await).contains(&"ol:201".to_owned()));
    ada.put(
        &format!("/api/shelf/{entry}/cover"),
        json!({"key": "ol:201"}),
    )
    .await
    .assert_detail(400, "This cover cannot be chosen");

    // A personal photo's takedown too.
    let photo =
        upload_photo(&ada, entry, jpeg([20, 120, 60])).await.json()["book"]["cover_public_id"]
            .as_str()
            .unwrap()
            .to_owned();
    root.delete(&format!("/api/admin/media/{photo}"))
        .await
        .assert_status(204);
    assert_eq!(shelf_item(&ada, "ada", work).await["personal_cover"], false);
}

/// "Undo" after removing a book: the book goes back on the shelf and the client chooses the
/// same key again, a personal photo too (it outlives its choice by 7 days).
#[sqlx::test]
async fn undo_chooses_the_same_cover_again(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = circe(&app).await;
    let ada = app.user("ada").await;
    let entry = add(&ada, work, "currently_reading").await;
    let photo =
        upload_photo(&ada, entry, jpeg([20, 120, 60])).await.json()["book"]["cover_public_id"]
            .as_str()
            .unwrap()
            .to_owned();
    ada.delete(&format!("/api/shelf/{entry}"))
        .await
        .assert_status(204);
    let mut conn = app.db.acquire().await.unwrap();
    bookclub_server::auth::sweep_personal_covers(&mut conn, app.media.as_ref())
        .await
        .unwrap();

    let again = add(&ada, work, "currently_reading").await;
    let res = ada
        .put(
            &format!("/api/shelf/{again}/cover"),
            json!({"key": format!("media:{photo}")}),
        )
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["book"]["cover_public_id"], photo);
    assert_eq!(res.json()["personal_cover"], true);
}

/// With `DNB_PORTAL_COVERS`, a German-group edition without a cover gets the publisher's cover
/// (provenance `dnb`, only where there is none) and lists as the catalog's `dnb_portal` cover;
/// without the setting the portal is never asked. A work without an Open Library id asks Open
/// Library's cover of its ISBN.
#[sqlx::test]
async fn the_portal_and_the_isbn_cover_feed_the_list(db: PgPool) {
    const ISBN: &str = "9783462050493";
    let app = TestApp::with_env(db, &[("DNB_PORTAL_COVERS", "true")]).await;
    let (work, edition) = app.dnb_work("Tyll", "Daniel Kehlmann", ISBN).await;
    app.catalog
        .on_publisher_cover(ISBN, Ok(Some(jpeg([120, 40, 30]))));
    app.catalog.on_isbn_cover(ISBN, Some(301));
    let ada = app.user("ada").await;

    let list = candidates(&ada, work.id).await;
    assert_eq!(app.catalog.publisher_calls(), [ISBN]);
    let items = list["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{list}");
    assert_eq!(items[0]["source"], "dnb_portal");
    assert_eq!(items[0]["edition_label"]["isbn13"], ISBN);
    assert_eq!(items[1]["key"], "ol:301");
    assert_eq!(items[1]["edition_label"]["isbn13"], ISBN);
    let provenance: Option<String> = sqlx::query_scalar(
        "SELECT e.field_sources ->> 'cover_media_id' FROM editions e
         JOIN media m ON m.id = e.cover_media_id AND m.source = 'dnb_portal' WHERE e.id = $1",
    )
    .bind(edition.id)
    .fetch_optional(&app.db)
    .await
    .unwrap()
    .flatten();
    assert_eq!(provenance.as_deref(), Some("dnb"));
    // The edition has its cover now: the portal is not asked again.
    candidates(&ada, work.id).await;
    assert_eq!(app.catalog.publisher_calls(), [ISBN]);
}

#[sqlx::test]
async fn without_the_setting_the_portal_is_never_asked(db: PgPool) {
    const ISBN: &str = "9783462050493";
    let app = TestApp::new(db).await;
    let (work, _) = app.dnb_work("Tyll", "Daniel Kehlmann", ISBN).await;
    app.catalog
        .on_publisher_cover(ISBN, Ok(Some(jpeg([120, 40, 30]))));
    let ada = app.user("ada").await;
    let list = candidates(&ada, work.id).await;
    assert!(app.catalog.publisher_calls().is_empty());
    assert_eq!(list["items"], json!([]));
    assert_eq!(list["partial"], false);
}

/// Open Library not answering leaves the list partial, with the catalog's covers.
#[sqlx::test]
async fn an_open_library_outage_leaves_the_list_partial(db: PgPool) {
    let app = TestApp::new(db).await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&app.upstream)
        .await;
    let work = circe(&app).await;
    let ada = app.user("ada").await;
    let list = candidates(&ada, work).await;
    assert_eq!(list["partial"], true);
    assert_eq!(
        keys(&list),
        [format!("media:{}", public_id_of_ol(&app, 100).await)]
    );
    ada.get("/api/books/works/OL404W/cover-candidates")
        .await
        .assert_detail(404, "No such book.");
}

/// A merge keeps the kept entry's choice, else the dropped entry's; the export names the chosen
/// picture by its public id; account deletion deletes the reader's personal photos with their
/// files.
#[sqlx::test]
async fn merge_export_and_deletion_handle_the_choice(db: PgPool) {
    let app = TestApp::new(db).await;
    let work = circe(&app).await;
    let duplicate = app.work("Circe (duplicate)").await.id;
    let ada = app.user("ada").await;
    add(&ada, work, "want_to_read").await;
    let dropped = add(&ada, duplicate, "finished").await;
    let photo = upload_photo(&ada, dropped, jpeg([20, 120, 60]))
        .await
        .json()["book"]["cover_public_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut conn = app.db.acquire().await.unwrap();
    merge_works(&mut conn, duplicate, work).await.unwrap();
    let shown = shelf_item(&ada, "ada", work).await;
    assert_eq!(shown["book"]["cover_public_id"], photo);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM shelf_entries").await,
        1
    );

    let export = ada.get("/api/auth/me/export").await;
    export.assert_status(200);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(export.body.to_vec())).unwrap();
    let mut data = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("data.json").unwrap(), &mut data).unwrap();
    let data: Value = serde_json::from_str(&data).unwrap();
    let entry = &data["shelf_entries"][0];
    assert_eq!(entry["cover_public_id"], photo);
    assert!(entry.get("cover_media_id").is_none());
    let uploaded = data["uploaded_media"]
        .as_array()
        .unwrap()
        .iter()
        .find(|media| media["public_id"] == photo)
        .unwrap();
    assert_eq!(uploaded["personal"], true);

    ada.delete_json("/api/auth/me", json!({"password": PASSWORD}))
        .await
        .assert_status(204);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM media WHERE public_id = '{photo}'"
        ))
        .await,
        0
    );
    assert!(
        !app.media.keys().iter().any(|key| key.contains(&photo)),
        "the photo's files are still there"
    );
}

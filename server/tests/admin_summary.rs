//! The admin queues at a glance (#54): `GET /api/admin/summary` (open counts, overdue counts,
//! the oldest open item per queue, 90-day report statistics) and `?status=open|resolved&page=`
//! on `GET /api/admin/reports` and `GET /api/admin/edit-requests`.

mod common;

use bookclub_server::catalog::shared::ingest::{CoverTarget, Source, ol_cover_media, set_cover};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// A work showing the Open Library cover `ol_cover_id`; returns the work id and the picture's
/// public id.
async fn picture(app: &TestApp, title: &str, ol_cover_id: i64) -> (i64, String) {
    let work = app.work(title).await;
    let mut conn = app.db.acquire().await.unwrap();
    let media = ol_cover_media(&mut conn, ol_cover_id)
        .await
        .unwrap()
        .unwrap();
    let set = set_cover(
        &mut conn,
        CoverTarget::Work(work.id),
        media,
        Source::OpenLibrary,
        false,
    )
    .await;
    assert!(set.unwrap(), "the cover is set");
    let public_id = sqlx::query_scalar("SELECT public_id FROM media WHERE id = $1")
        .bind(media)
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    (work.id, public_id)
}

/// Files a report of `public_id` as `client`; returns its id.
async fn report(client: &TestClient, public_id: &str) -> i64 {
    let res = client
        .post(
            &format!("/api/media/{public_id}/reports"),
            json!({"reason": "copyright"}),
        )
        .await;
    res.assert_status(201);
    res.json()["id"].as_i64().unwrap()
}

/// Moves a report's arrival `hours` into the past.
async fn age_report(app: &TestApp, id: i64, hours: i64) {
    app.exec(&format!(
        "UPDATE reports SET created_at = now() - interval '{hours} hours' WHERE id = {id}"
    ))
    .await;
}

async fn summary(admin: &TestClient) -> Value {
    let res = admin.get("/api/admin/summary").await;
    res.assert_status(200);
    res.json()
}

fn ids(page: &Value) -> Vec<i64> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_i64().unwrap())
        .collect()
}

#[sqlx::test]
async fn an_empty_instance_has_nothing_waiting(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let empty = json!({"open": 0, "overdue": 0, "oldest": null});
    assert_eq!(
        summary(&root).await,
        json!({
            "target_hours": 72,
            "reports": empty,
            "edit_requests": empty,
            "report_stats": {
                "days": 90,
                "resolved": 0,
                "median_hours": null,
                "max_hours": null,
                "over_target": 0
            }
        })
    );
}

/// Acceptance (#54): a report resolved after 50 h gives a median of 50 and nothing over the
/// 72 h target. Reports closed on arrival (no admin decided them) and reports resolved more
/// than 90 days ago are not counted; a direct takedown's notice is.
#[sqlx::test]
async fn report_stats_count_what_admins_resolved_in_90_days(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let root = app.admin("root").await;
    let (_, first) = picture(&app, "Circe", 101).await;
    let (_, second) = picture(&app, "Ich bin Circe", 102).await;
    let (_, third) = picture(&app, "Kirke", 103).await;

    let id = report(&ada, &first).await;
    age_report(&app, id, 50).await;
    root.post(
        &format!("/api/admin/reports/{id}/resolve"),
        json!({"action": "dismiss"}),
    )
    .await
    .assert_status(200);
    let stats = summary(&root).await["report_stats"].clone();
    assert_eq!(
        stats,
        json!({"days": 90, "resolved": 1, "median_hours": 50.0, "max_hours": 50.0, "over_target": 0})
    );

    let id = report(&ada, &second).await;
    age_report(&app, id, 80).await;
    root.post(
        &format!("/api/admin/reports/{id}/resolve"),
        json!({"action": "remove"}),
    )
    .await
    .assert_status(200);
    // Closed on arrival: the picture is down already, no admin decided it.
    report(&grace, &second).await;
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM reports WHERE resolved_by IS NULL")
            .await,
        1
    );
    // Resolved long ago.
    let old = report(&grace, &first).await;
    app.exec(&format!(
        "UPDATE reports SET status = 'dismissed', resolved_by = {root},
                created_at = now() - interval '100 days',
                resolved_at = now() - interval '91 days'
         WHERE id = {old}",
        root = root.id()
    ))
    .await;
    let stats = summary(&root).await["report_stats"].clone();
    assert_eq!(
        stats,
        json!({"days": 90, "resolved": 2, "median_hours": 65.0, "max_hours": 80.0, "over_target": 1})
    );

    // A notice that arrived by email 10 hours ago, answered with a direct takedown now.
    let received = time::OffsetDateTime::now_utc() - time::Duration::hours(10);
    root.delete_json(
        &format!("/api/admin/media/{third}"),
        json!({"received_at": received.format(&time::format_description::well_known::Rfc3339).unwrap()}),
    )
    .await
    .assert_status(204);
    let stats = summary(&root).await["report_stats"].clone();
    assert_eq!(
        stats,
        json!({"days": 90, "resolved": 3, "median_hours": 50.0, "max_hours": 80.0, "over_target": 1})
    );
}

/// The open counts, the overdue counts (older than 72 h) and the oldest open item of each
/// queue; decided items are not open.
#[sqlx::test]
async fn open_items_count_with_the_oldest_and_the_overdue(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let root = app.admin("root").await;
    let (work, first) = picture(&app, "Circe", 101).await;
    let (_, second) = picture(&app, "Ich bin Circe", 102).await;
    let fresh = report(&ada, &first).await;
    let old = report(&ada, &second).await;
    age_report(&app, old, 73).await;
    let closed = report(&grace, &second).await;
    age_report(&app, closed, 200).await;
    app.exec(&format!(
        "UPDATE reports SET status = 'dismissed', resolved_by = {}, resolved_at = now()
         WHERE id = {closed}",
        root.id()
    ))
    .await;
    let request = ada
        .post(
            &format!("/api/books/works/{work}/edit-requests"),
            json!({"fields": {"pages": 393}}),
        )
        .await;
    request.assert_status(201);
    let request = request.json()["id"].as_i64().unwrap();

    let summary = summary(&root).await;
    assert_eq!(summary["reports"]["open"], 2);
    assert_eq!(summary["reports"]["overdue"], 1);
    assert_eq!(summary["reports"]["oldest"]["id"], old);
    let old_at: String = sqlx::query_scalar(
        "SELECT to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS') FROM reports
         WHERE id = $1",
    )
    .bind(old)
    .fetch_one(&app.db)
    .await
    .unwrap();
    let oldest_at = summary["reports"]["oldest"]["created_at"].as_str().unwrap();
    assert!(oldest_at.starts_with(&old_at), "{oldest_at} vs {old_at}");
    assert_ne!(summary["reports"]["oldest"]["id"], fresh);
    assert_eq!(summary["edit_requests"]["open"], 1);
    assert_eq!(summary["edit_requests"]["overdue"], 0);
    assert_eq!(summary["edit_requests"]["oldest"]["id"], request);

    app.exec(&format!(
        "UPDATE catalog_edit_requests SET created_at = now() - interval '100 hours'
         WHERE id = {request}"
    ))
    .await;
    assert_eq!(requests_overdue(&root).await, 1);
}

async fn requests_overdue(root: &TestClient) -> i64 {
    summary(root).await["edit_requests"]["overdue"]
        .as_i64()
        .unwrap()
}

/// Acceptance (#54): `?status=resolved` lists the resolved reports newest decision first, 50 a
/// page; `?status=open` (the default) the open ones oldest first.
#[sqlx::test]
async fn resolved_reports_list_newest_first_with_paging(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let (_, public_id) = picture(&app, "Circe", 101).await;
    // 52 decided reports, the first decided longest ago, and one open report.
    app.exec(&format!(
        "INSERT INTO reports (target_kind, reason, status, created_by, created_at, resolved_by,
                              resolved_at)
         SELECT 'cover', 'other', CASE WHEN i % 2 = 0 THEN 'dismissed' ELSE 'removed' END,
                {ada}, now() - interval '10 days', {root}, now() - make_interval(mins => 100 - i)
         FROM generate_series(1, 52) AS i",
        ada = ada.id(),
        root = root.id()
    ))
    .await;
    let open = report(&ada, &public_id).await;
    let mut decided: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM reports WHERE status <> 'open' ORDER BY resolved_at DESC",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(decided.len(), 52);

    let first = root.get("/api/admin/reports?status=resolved").await;
    first.assert_status(200);
    let first = first.json();
    assert_eq!(
        (first["page"].clone(), first["has_more"].clone()),
        (json!(1), json!(true))
    );
    assert_eq!(ids(&first), decided[..50]);
    assert_eq!(first["items"][0]["resolved_by"]["username"], "root");
    let second = root
        .get("/api/admin/reports?status=resolved&page=2")
        .await
        .json();
    assert_eq!(
        (second["page"].clone(), second["has_more"].clone()),
        (json!(2), json!(false))
    );
    assert_eq!(ids(&second), decided.split_off(50));
    let beyond = root
        .get("/api/admin/reports?status=resolved&page=3")
        .await
        .json();
    assert_eq!(beyond, json!({"items": [], "page": 3, "has_more": false}));

    for path in [
        "/api/admin/reports",
        "/api/admin/reports?status=open&page=1",
    ] {
        let page = root.get(path).await.json();
        assert_eq!(ids(&page), [open], "{path}");
        assert_eq!(page["has_more"], false);
    }
}

/// The same for catalog requests: `?status=resolved` lists approved and rejected requests,
/// newest decision first.
#[sqlx::test]
async fn decided_requests_list_newest_first_with_paging(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let work = app.work("Circe").await;
    app.exec(&format!(
        "INSERT INTO catalog_edit_requests (kind, work_id, payload, status, created_by,
                                            created_at, decided_by, decided_at)
         SELECT 'work_edit', {work}, jsonb_build_object('pages', i),
                CASE WHEN i % 2 = 0 THEN 'approved' ELSE 'rejected' END, {ada},
                now() - interval '10 days', {root}, now() - make_interval(mins => 100 - i)
         FROM generate_series(1, 51) AS i",
        work = work.id,
        ada = ada.id(),
        root = root.id()
    ))
    .await;
    let open = ada
        .post(
            &format!("/api/books/works/{}/edit-requests", work.id),
            json!({"fields": {"pages": 393}}),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    let decided: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM catalog_edit_requests WHERE status <> 'open' ORDER BY decided_at DESC",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();

    let first = root
        .get("/api/admin/edit-requests?status=resolved")
        .await
        .json();
    assert_eq!(ids(&first), decided[..50]);
    assert_eq!(first["has_more"], true);
    assert_eq!(first["items"][0]["payload"], json!({"pages": 51}));
    assert_eq!(first["items"][0]["status"], "rejected");
    assert_eq!(first["items"][1]["status"], "approved");
    assert_eq!(first["items"][0]["decided_by"]["username"], "root");
    let second = root
        .get("/api/admin/edit-requests?status=resolved&page=2")
        .await
        .json();
    assert_eq!(ids(&second), decided[50..]);
    assert_eq!(second["has_more"], false);
    let open_page = root.get("/api/admin/edit-requests").await.json();
    assert_eq!(
        open_page,
        json!({"items": open_page["items"], "page": 1, "has_more": false})
    );
    assert_eq!(ids(&open_page), [open]);
}

#[sqlx::test]
async fn the_queue_query_is_checked(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    for list in ["/api/admin/reports", "/api/admin/edit-requests"] {
        for (query, detail) in [
            ("status=closed", "Input should be 'open' or 'resolved'"),
            ("page=0", "Input should be greater than or equal to 1"),
            ("page=1001", "Input should be less than or equal to 1000"),
            (
                "page=x",
                "Input should be a valid integer, unable to parse string as an integer",
            ),
        ] {
            root.get(&format!("{list}?{query}"))
                .await
                .assert_detail(400, detail);
        }
    }
}

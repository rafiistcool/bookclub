//! `GET /api/admin/summary` (#54): what waits in the two admin queues and how fast reports were
//! decided, for the tab badges, the overdue flags and the open count in Settings.

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::queue::TARGET_HOURS;
use crate::AppState;
use crate::auth::extract::InstanceAdmin;
use crate::error::{ApiResult, ErrorBody};

/// The window of the report statistics, in days.
pub const STATS_DAYS: i64 = 90;

/// Both queues at a glance.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdminSummary {
    /// Hours after which an open item is overdue (72 until the takedown targets are settled).
    pub target_hours: i64,
    /// Reports of pictures.
    pub reports: AdminQueueSummary,
    /// Catalog requests: edits, cover photos, merges.
    pub edit_requests: AdminQueueSummary,
    /// How reports were decided in the last 90 days.
    pub report_stats: AdminReportStats,
}

/// The open part of one queue.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdminQueueSummary {
    /// Open items.
    pub open: i64,
    /// Open items older than `target_hours`.
    pub overdue: i64,
    /// The oldest open item; null when nothing is open.
    pub oldest: Option<AdminOldestItem>,
}

/// The item that has waited longest.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdminOldestItem {
    /// The report's or request's id.
    pub id: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Time to a decision of the reports an admin resolved in the last `days` days (by
/// `resolved_at`). A report of a picture that was down already is closed on arrival without an
/// admin (`resolved_by` null) and is not counted.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AdminReportStats {
    /// The window: 90.
    pub days: i64,
    /// Reports resolved in the window.
    pub resolved: i64,
    /// The median hours from report (or a notice's arrival) to decision, to 0.1 h; null when
    /// none was resolved.
    pub median_hours: Option<f64>,
    /// The longest, to 0.1 h; null when none was resolved.
    pub max_hours: Option<f64>,
    /// Resolved later than `target_hours` after they arrived.
    pub over_target: i64,
}

/// Open counts, overdue counts and the oldest open item of both queues, and the 90-day report
/// statistics.
#[utoipa::path(
    get,
    path = "/api/admin/summary",
    operation_id = "admin_summary",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, body = AdminSummary, description = "Both queues at a glance"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found` for everyone but instance admins")
    )
)]
pub async fn summary(
    _admin: InstanceAdmin,
    State(state): State<AppState>,
) -> ApiResult<Json<AdminSummary>> {
    let mut conn = state.db.acquire().await?;
    Ok(Json(admin_summary(&mut conn).await?))
}

/// The [`AdminSummary`] as of now.
pub async fn admin_summary(conn: &mut PgConnection) -> sqlx::Result<AdminSummary> {
    let reports = sqlx::query!(
        r#"SELECT count(*) AS "open!",
                  count(*) FILTER (WHERE created_at < now() - make_interval(hours => $1))
                      AS "overdue!",
                  (array_agg(id ORDER BY created_at, id))[1] AS oldest_id,
                  min(created_at) AS oldest_at
           FROM reports WHERE status = 'open'"#,
        TARGET_HOURS as i32
    )
    .fetch_one(&mut *conn)
    .await?;
    let requests = sqlx::query!(
        r#"SELECT count(*) AS "open!",
                  count(*) FILTER (WHERE created_at < now() - make_interval(hours => $1))
                      AS "overdue!",
                  (array_agg(id ORDER BY created_at, id))[1] AS oldest_id,
                  min(created_at) AS oldest_at
           FROM catalog_edit_requests WHERE status = 'open'"#,
        TARGET_HOURS as i32
    )
    .fetch_one(&mut *conn)
    .await?;
    let stats = sqlx::query!(
        r#"WITH decided AS (
               SELECT extract(epoch FROM resolved_at - created_at)::float8 / 3600 AS hours
               FROM reports
               WHERE status <> 'open' AND resolved_by IS NOT NULL
                 AND resolved_at >= now() - make_interval(days => $1)
           )
           SELECT count(*) AS "resolved!",
                  round((percentile_cont(0.5) WITHIN GROUP (ORDER BY hours))::numeric, 1)::float8
                      AS median_hours,
                  round(max(hours)::numeric, 1)::float8 AS max_hours,
                  count(*) FILTER (WHERE hours > $2) AS "over_target!"
           FROM decided"#,
        STATS_DAYS as i32,
        TARGET_HOURS as f64
    )
    .fetch_one(&mut *conn)
    .await?;
    let oldest = |id: Option<i64>, at: Option<OffsetDateTime>| match (id, at) {
        (Some(id), Some(created_at)) => Some(AdminOldestItem { id, created_at }),
        _ => None,
    };
    Ok(AdminSummary {
        target_hours: TARGET_HOURS,
        reports: AdminQueueSummary {
            open: reports.open,
            overdue: reports.overdue,
            oldest: oldest(reports.oldest_id, reports.oldest_at),
        },
        edit_requests: AdminQueueSummary {
            open: requests.open,
            overdue: requests.overdue,
            oldest: oldest(requests.oldest_id, requests.oldest_at),
        },
        report_stats: AdminReportStats {
            days: STATS_DAYS,
            resolved: stats.resolved,
            median_hours: stats.median_hours,
            max_hours: stats.max_hours,
            over_target: stats.over_target,
        },
    })
}

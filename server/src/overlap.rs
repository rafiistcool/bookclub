//! Books two or more club members want to read (#45, owner CLUB; endpoint-map §9,
//! docs/phase1/spec/club.md §1.23, §2.10). Computed over the club's members only, from their
//! shelves as the caller may see them (`bc_visible_shelf`).

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;
use crate::catalog::shared::books::{BookRef, book_outs};
use crate::clubs::extract::ClubMember;
use crate::dto::{BookOut, ShelfStatus};
use crate::error::{ApiResult, ErrorBody};
use crate::extract::{ApiQuery, Validate};

/// The overlap route.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(overlap))
}

/// `?include_reading=`: also count books members are reading.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct OverlapQuery {
    /// Also count `currently_reading` (`true/false/1/0/yes/no/on/off/t/f/y/n`, any case);
    /// default false.
    #[serde(default, deserialize_with = "crate::extract::lax_bool")]
    #[param(value_type = Option<bool>)]
    pub include_reading: bool,
}

impl Validate for OverlapQuery {}

/// Books two or more club members want to read (or are reading).
#[derive(Debug, Serialize, ToSchema)]
pub struct OverlapOut {
    /// Most members first, then by title.
    pub items: Vec<OverlapBookOut>,
    pub include_reading: bool,
}

/// One shared book.
#[derive(Debug, Serialize, ToSchema)]
pub struct OverlapBookOut {
    pub book: BookOut,
    /// The number of members.
    pub count: i64,
    /// By username.
    pub members: Vec<OverlapMember>,
}

/// A member with the book on their shelf.
#[derive(Debug, Serialize, ToSchema)]
pub struct OverlapMember {
    pub username: String,
    /// `want_to_read`, or `currently_reading` with `include_reading`.
    pub status: ShelfStatus,
}

/// Books that two or more club members want to read; with `include_reading`, reading counts
/// too. Finished and abandoned books never count.
#[utoipa::path(
    get,
    path = "/api/clubs/{slug}/overlap",
    operation_id = "overlap_get",
    tag = "overlap",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(
        ("slug" = String, Path, description = "The club's slug"),
        OverlapQuery
    ),
    responses(
        (status = 200, body = OverlapOut, description = "The shared books"),
        (status = 400, body = ErrorBody, description = "`Input should be a valid boolean, unable to interpret input`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club` (also for a non-member)")
    )
)]
pub async fn overlap(
    member: ClubMember,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<OverlapQuery>,
) -> ApiResult<Json<OverlapOut>> {
    let mut statuses = vec!["want_to_read".to_owned()];
    if query.include_reading {
        statuses.push("currently_reading".to_owned());
    }
    let mut conn = state.db.acquire().await?;
    let rows = sqlx::query!(
        r#"SELECT s.work_id AS "work_id!", u.username, s.status AS "status!: ShelfStatus",
                  w.title
           FROM bc_visible_shelf($1) s
           JOIN club_memberships m ON m.club_id = $2 AND m.user_id = s.user_id
           JOIN users u ON u.id = s.user_id
           JOIN works w ON w.id = s.work_id
           WHERE s.status = ANY($3)"#,
        member.user.id,
        member.club.id,
        &statuses
    )
    .fetch_all(&mut *conn)
    .await?;
    // work id → (title, members); a user has one entry per work.
    let mut by_work: BTreeMap<i64, (String, Vec<OverlapMember>)> = BTreeMap::new();
    for row in rows {
        by_work
            .entry(row.work_id)
            .or_insert_with(|| (row.title, Vec::new()))
            .1
            .push(OverlapMember {
                username: row.username,
                status: row.status,
            });
    }
    let mut shared: Vec<(i64, String, Vec<OverlapMember>)> = by_work
        .into_iter()
        .filter(|(_, (_, members))| members.len() >= 2)
        .map(|(work_id, (title, mut members))| {
            members.sort_by(|a, b| a.username.cmp(&b.username));
            (work_id, title.to_lowercase(), members)
        })
        .collect();
    shared.sort_by(|a, b| {
        b.2.len()
            .cmp(&a.2.len())
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.0.cmp(&b.0))
    });
    let refs: Vec<BookRef> = shared.iter().map(|row| BookRef::work(row.0)).collect();
    let books = book_outs(&mut conn, &refs).await?;
    let items = shared
        .into_iter()
        .filter_map(|(work_id, _, members)| {
            Some(OverlapBookOut {
                book: books.get(&BookRef::work(work_id))?.clone(),
                count: members.len() as i64,
                members,
            })
        })
        .collect();
    Ok(Json(OverlapOut {
        items,
        include_reading: query.include_reading,
    }))
}

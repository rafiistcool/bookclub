//! The personal shelf (#44a, owner SHELF): shelf, favourites, physical layout, the Goodreads
//! import, quotes, search history, personal and club stats (endpoint-map §2 favourites, §7, §9
//! club stats; docs/phase1/spec/shelf.md).
//!
//! - `entries`: the shelf list, add, change and remove, with the ordering and date rules of
//!   `ordering` and the shelf events.
//! - `favorites`: the three favourite works under `/api/auth/me/favorites`.
//! - `layout`: the physical shelf layout and its validation.
//! - `search_history`: the caller's last 20 searches.
//! - `goodreads`: the Goodreads CSV import, a background job with progress.
//! - `quotes`: the lines a reader keeps, each `audience` or `only_me`.
//! - `stats`: the personal and the club year in review, on one `MemberYear` rule set.
//!
//! Every write of a user's shelf, favourites or layout first locks that user's row
//! ([`lock_user`]), so two devices editing one shelf run one after the other and the column
//! order never gets duplicate positions (conventions §13). Reads of another user's shelf go
//! through `visibility::visible_user` and `bc_visible_shelf`.

pub mod entries;
pub mod favorites;
pub mod goodreads;
pub mod layout;
pub mod ordering;
pub mod quotes;
pub mod search_history;
pub mod stats;

use serde::Deserialize;
use sqlx::PgConnection;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::dto::UserSummaryRow;
use crate::error::{ApiResult, AppError};
use crate::extract::Validate;
use crate::visibility;

/// The shelf routes, the favourites under `/api/auth/me/favorites`, the Goodreads import, the
/// search history, the quotes and both stats routes.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(goodreads::routes())
        .routes(routes!(entries::list_shelf, entries::add_to_shelf))
        .routes(routes!(entries::update_entry, entries::remove_entry))
        .routes(routes!(layout::get_layout, layout::save_layout))
        .routes(routes!(
            favorites::list_favorites,
            favorites::replace_favorites
        ))
        .routes(routes!(favorites::add_favorite, favorites::remove_favorite))
        .routes(routes!(
            search_history::list_history,
            search_history::remember_search,
            search_history::clear_history
        ))
        .routes(routes!(search_history::delete_history_entry))
        .routes(routes!(quotes::list_quotes, quotes::add_quote))
        .routes(routes!(quotes::update_quote, quotes::delete_quote))
        .routes(routes!(stats::personal_stats))
        .routes(routes!(stats::club_stats))
}

/// `?username=` on the personal reads: whose data to show. Missing or empty means the caller.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct OwnerQuery {
    /// Another user's name (case-insensitive). Unknown names and users outside the caller's
    /// audience get the same 404 `No member with that name`.
    #[serde(default)]
    pub username: Option<String>,
}

impl Validate for OwnerQuery {}

impl OwnerQuery {
    /// The requested name, or `None` for the caller (missing or empty, as in the legacy app).
    pub fn name(&self) -> Option<&str> {
        self.username.as_deref().filter(|name| !name.is_empty())
    }
}

/// The owner of the personal data a read asks for: the caller, or `?username=` when the caller
/// may see that user (`visibility::visible_user`: 404 `No member with that name` otherwise).
pub async fn resolve_owner(
    conn: &mut PgConnection,
    me: &CurrentUser,
    query: &OwnerQuery,
) -> ApiResult<UserSummaryRow> {
    let name = query.name().unwrap_or(&me.username);
    visibility::visible_user(conn, me.id, name).await
}

/// Locks the user's row (`SELECT … FOR UPDATE`) for a shelf, favourites or layout write, so the
/// writes of one user run one after the other. 401 when the account is gone.
pub async fn lock_user(conn: &mut PgConnection, user_id: i64) -> ApiResult<()> {
    sqlx::query_scalar!("SELECT id FROM users WHERE id = $1 FOR UPDATE", user_id)
        .fetch_optional(conn)
        .await?
        .map(|_| ())
        .ok_or_else(AppError::unauthorized)
}

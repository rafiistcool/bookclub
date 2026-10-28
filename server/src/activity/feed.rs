//! The home and club activity feeds (`GET /api/activity`, `GET /api/clubs/{slug}/activity`).
//! Owned by CLUB from Wave 2 on (docs/phase1/conventions.md §6.4, endpoint-map §9,
//! docs/phase1/spec/club.md §1.24, §2.11, §5.7).
//!
//! - **Club feed:** the club's events, plus the personal events (`club_id IS NULL`) of its
//!   current members created at or after their `joined_at`.
//! - **Home feed:** the union of the caller's club feeds plus the caller's own personal events,
//!   each event once.
//! - **Redaction** for everyone but the actor: `shelf_finished` loses `rating` and `take` when
//!   the actor's current shelf entry for the work is "only me" or gone; `quote_added` is left out
//!   when its quote is gone, "only me", or not named (`quote_id`).
//! - **Names at read time:** a `reaction` payload's `author_id` is served as `author`, the
//!   username or null when the account is gone.
//!
//! Newest first, keyset-paged by id (`before`), `limit` 1..100 (default 30).

use std::collections::HashMap;

use axum::Json;
use axum::extract::State;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgConnection;
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::record::PRIVATE_KEY;
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::catalog::shared::books::{BookRef, book_outs};
use crate::clubs::extract::ClubMember;
use crate::dto::{BookOut, ClubRef};
use crate::error::{ApiResult, ErrorBody};
use crate::extract::{ApiQuery, Validate, Validator};
use crate::picks::club_zone;
use crate::shelf::quotes::EXCERPT_CHARS;
use crate::text::truncate_chars;
use crate::tz;

/// The default page size.
const DEFAULT_LIMIT: i64 = 30;
/// The largest page size.
const MAX_LIMIT: i64 = 100;

/// The feed routes.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(home_feed))
        .routes(routes!(club_feed))
}

/// The query of both feeds.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FeedQuery {
    /// Only events with a smaller id (≥ 1): the last id of the previous page.
    #[serde(default)]
    pub before: Option<i64>,
    /// 1 to 100, default 30.
    #[serde(default = "default_limit")]
    #[param(value_type = Option<i64>)]
    pub limit: i64,
    /// Only this person's events (case-insensitive; empty means no filter). A name the caller
    /// cannot see (home feed) or that is not a member of the club (club feed) gives an empty
    /// list.
    #[serde(default)]
    pub username: Option<String>,
}

fn default_limit() -> i64 {
    DEFAULT_LIMIT
}

impl Validate for FeedQuery {
    fn validate(&mut self, v: &mut Validator) {
        v.range_i64(self.before, 1, i64::MAX);
        v.range_i64(Some(self.limit), 1, MAX_LIMIT);
    }
}

impl FeedQuery {
    /// The username filter, lowercased; `None` when absent or empty.
    fn username(&self) -> Option<String> {
        self.username
            .as_deref()
            .filter(|name| !name.is_empty())
            .map(str::to_lowercase)
    }
}

/// A page of the feed, newest first.
#[derive(Debug, Serialize, ToSchema)]
pub struct ActivityOut {
    pub items: Vec<ActivityItem>,
    /// More events are older than the last item (pass its id as `before`).
    pub has_more: bool,
    /// The club's zone (club feed) or the caller's (home feed).
    pub timezone: String,
}

/// One event.
#[derive(Debug, Serialize, ToSchema)]
pub struct ActivityItem {
    pub id: i64,
    /// `member_joined`, `pick_set`, `pick_cleared`, `note_posted`, `reaction`, `nominated`,
    /// `voted`, `vote_closed`, `milestone_added`, `shelf_added`, `shelf_moved`,
    /// `shelf_finished`, `shelf_dnf`, `shelf_removed`, `progress` or `quote_added`.
    pub kind: String,
    /// The actor's username; null for a system event (an auto-close credited to a deleted
    /// account).
    pub actor: Option<String>,
    /// The caller is the actor.
    pub mine: bool,
    /// The work, without an edition context.
    pub book: Option<BookOut>,
    pub pick_id: Option<i64>,
    /// The club of a club event; null for a personal event.
    pub club: Option<ClubRef>,
    /// Kind-specific (docs/phase1/spec/club.md §2.11); `reaction` carries `author` (a username
    /// or null), never `author_id`.
    #[schema(value_type = Object)]
    pub payload: Value,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// E.g. `Sat 22 Aug 2026, 19:00 CEST` (always English): in the club's zone for club events
    /// and in the club feed, in the caller's zone for personal events in the home feed.
    pub created_label: String,
}

/// An `events` row as the feed reads it.
struct EventRow {
    id: i64,
    club_id: Option<i64>,
    actor_id: Option<i64>,
    actor: Option<String>,
    kind: String,
    work_id: Option<i64>,
    pick_id: Option<i64>,
    payload: Value,
    created_at: OffsetDateTime,
    /// The current body and page of a `quote_added` event's quote, when it still exists.
    quote_body: Option<String>,
    quote_page: Option<i32>,
    /// A `shelf_finished` whose rating and take the caller may not see.
    hide_rating: bool,
}

/// The clubs the items name, with the zone of their labels, and the zone of personal items.
struct Zones {
    clubs: HashMap<i64, (ClubRef, TimeZone)>,
    personal: TimeZone,
}

/// A feed request after the username filter: `None` filter means everyone.
struct Page {
    before: Option<i64>,
    limit: i64,
    actor: Option<i64>,
}

/// The caller's home feed: events of every club they are in, the personal events of their
/// co-members from the moment each joined the shared club, and their own personal events.
#[utoipa::path(
    get,
    path = "/api/activity",
    operation_id = "activity_home_feed",
    tag = "activity",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(FeedQuery),
    responses(
        (status = 200, body = ActivityOut, description = "A page of the feed"),
        (status = 400, body = ErrorBody, description = "`Input should be greater than or equal to 1`, `Input should be less than or equal to 100`, `Input should be a valid integer, unable to parse string as an integer`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn home_feed(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<FeedQuery>,
) -> ApiResult<Json<ActivityOut>> {
    let (personal, timezone) = club_zone(&user.timezone);
    let mut conn = state.db.acquire().await?;
    let actor = match query.username() {
        None => None,
        Some(name) => {
            let found = sqlx::query_scalar!(
                "SELECT id FROM users WHERE username = $2 AND bc_can_see_user($1, id)",
                user.id,
                name
            )
            .fetch_optional(&mut *conn)
            .await?;
            match found {
                Some(id) => Some(id),
                None => return Ok(Json(empty(timezone))),
            }
        }
    };
    let page = Page {
        before: query.before,
        limit: query.limit,
        actor,
    };
    let rows = sqlx::query_as!(
        EventRow,
        r#"SELECT e.id, e.club_id, e.actor_id, u.username AS "actor?", e.kind, e.work_id,
                  e.pick_id, e.payload, e.created_at, q.body AS "quote_body?",
                  q.page AS "quote_page?",
                  (e.kind = 'shelf_finished' AND e.actor_id IS DISTINCT FROM $1
                   AND (e.payload ? 'private' OR NOT EXISTS (
                       SELECT 1 FROM shelf_entries s
                       WHERE s.user_id = e.actor_id AND s.work_id = e.work_id
                         AND bc_can_see_item($1, s.user_id, s.rating_visibility)))) AS "hide_rating!"
           FROM events e
           LEFT JOIN users u ON u.id = e.actor_id
           LEFT JOIN quotes q ON e.kind = 'quote_added' AND q.user_id = e.actor_id
                AND q.id = CASE WHEN jsonb_typeof(e.payload -> 'quote_id') = 'number'
                                     AND e.payload ->> 'quote_id' ~ '^[0-9]{1,18}$'
                                THEN (e.payload ->> 'quote_id')::int8 END
           WHERE (e.club_id IN (SELECT mc.club_id FROM bc_member_clubs($1) mc)
                  OR (e.club_id IS NULL
                      AND (e.actor_id = $1 OR EXISTS (
                          SELECT 1 FROM bc_member_clubs($1) mc
                          JOIN club_memberships m ON m.club_id = mc.club_id
                          WHERE m.user_id = e.actor_id AND m.joined_at <= e.created_at))))
             AND ($2::int8 IS NULL OR e.id < $2)
             AND ($3::int8 IS NULL OR e.actor_id = $3)
             AND (e.kind <> 'quote_added' OR e.actor_id = $1 OR EXISTS (
                  SELECT 1 FROM bc_visible_quotes($1) q
                  WHERE q.user_id = e.actor_id
                    AND q.id = CASE WHEN jsonb_typeof(e.payload -> 'quote_id') = 'number'
                                         AND e.payload ->> 'quote_id' ~ '^[0-9]{1,18}$'
                                    THEN (e.payload ->> 'quote_id')::int8 END))
           ORDER BY e.id DESC
           LIMIT $4"#,
        user.id,
        page.before,
        page.actor,
        page.limit + 1
    )
    .fetch_all(&mut *conn)
    .await?;
    let club_ids: Vec<i64> = rows.iter().filter_map(|row| row.club_id).collect();
    let clubs = sqlx::query!(
        "SELECT c.id, c.slug, c.name, c.timezone
         FROM clubs c
         JOIN bc_member_clubs($1) mc ON mc.club_id = c.id
         WHERE c.id = ANY($2)",
        user.id,
        &club_ids
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|row| {
        let (zone, _) = club_zone(&row.timezone);
        let club = ClubRef {
            slug: row.slug,
            name: row.name,
        };
        (row.id, (club, zone))
    })
    .collect();
    let zones = Zones { clubs, personal };
    let (items, has_more) = items(&mut conn, user.id, rows, &page, &zones).await?;
    Ok(Json(ActivityOut {
        items,
        has_more,
        timezone,
    }))
}

/// The club's feed: its events plus the personal events of its current members from the moment
/// each joined.
#[utoipa::path(
    get,
    path = "/api/clubs/{slug}/activity",
    operation_id = "activity_club_feed",
    tag = "activity",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(
        ("slug" = String, Path, description = "The club's slug"),
        FeedQuery
    ),
    responses(
        (status = 200, body = ActivityOut, description = "A page of the feed"),
        (status = 400, body = ErrorBody, description = "`Input should be greater than or equal to 1`, `Input should be less than or equal to 100`, `Input should be a valid integer, unable to parse string as an integer`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club` (also for a non-member)")
    )
)]
pub async fn club_feed(
    member: ClubMember,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<FeedQuery>,
) -> ApiResult<Json<ActivityOut>> {
    let club_id = member.club.id;
    let (zone, timezone) = club_zone(&member.club.timezone);
    let mut conn = state.db.acquire().await?;
    let actor = match query.username() {
        None => None,
        Some(name) => {
            let found = sqlx::query_scalar!(
                "SELECT u.id FROM users u
                 JOIN club_memberships m ON m.user_id = u.id AND m.club_id = $1
                 WHERE u.username = $2",
                club_id,
                name
            )
            .fetch_optional(&mut *conn)
            .await?;
            match found {
                Some(id) => Some(id),
                None => return Ok(Json(empty(timezone))),
            }
        }
    };
    let page = Page {
        before: query.before,
        limit: query.limit,
        actor,
    };
    let rows = sqlx::query_as!(
        EventRow,
        r#"SELECT e.id, e.club_id, e.actor_id, u.username AS "actor?", e.kind, e.work_id,
                  e.pick_id, e.payload, e.created_at, q.body AS "quote_body?",
                  q.page AS "quote_page?",
                  (e.kind = 'shelf_finished' AND e.actor_id IS DISTINCT FROM $1
                   AND (e.payload ? 'private' OR NOT EXISTS (
                       SELECT 1 FROM shelf_entries s
                       WHERE s.user_id = e.actor_id AND s.work_id = e.work_id
                         AND bc_can_see_item($1, s.user_id, s.rating_visibility)))) AS "hide_rating!"
           FROM events e
           LEFT JOIN users u ON u.id = e.actor_id
           LEFT JOIN quotes q ON e.kind = 'quote_added' AND q.user_id = e.actor_id
                AND q.id = CASE WHEN jsonb_typeof(e.payload -> 'quote_id') = 'number'
                                     AND e.payload ->> 'quote_id' ~ '^[0-9]{1,18}$'
                                THEN (e.payload ->> 'quote_id')::int8 END
           WHERE (e.club_id = $2
                  OR (e.club_id IS NULL AND EXISTS (
                      SELECT 1 FROM club_memberships m
                      WHERE m.club_id = $2 AND m.user_id = e.actor_id
                        AND m.joined_at <= e.created_at)))
             AND ($3::int8 IS NULL OR e.id < $3)
             AND ($4::int8 IS NULL OR e.actor_id = $4)
             AND (e.kind <> 'quote_added' OR e.actor_id = $1 OR EXISTS (
                  SELECT 1 FROM bc_visible_quotes($1) q
                  WHERE q.user_id = e.actor_id
                    AND q.id = CASE WHEN jsonb_typeof(e.payload -> 'quote_id') = 'number'
                                         AND e.payload ->> 'quote_id' ~ '^[0-9]{1,18}$'
                                    THEN (e.payload ->> 'quote_id')::int8 END))
           ORDER BY e.id DESC
           LIMIT $5"#,
        member.user.id,
        club_id,
        page.before,
        page.actor,
        page.limit + 1
    )
    .fetch_all(&mut *conn)
    .await?;
    let club = ClubRef {
        slug: member.club.slug.clone(),
        name: member.club.name.clone(),
    };
    let zones = Zones {
        clubs: HashMap::from([(club_id, (club, zone.clone()))]),
        personal: zone,
    };
    let (items, has_more) = items(&mut conn, member.user.id, rows, &page, &zones).await?;
    Ok(Json(ActivityOut {
        items,
        has_more,
        timezone,
    }))
}

/// An empty page (an unknown or hidden `username`).
fn empty(timezone: String) -> ActivityOut {
    ActivityOut {
        items: Vec::new(),
        has_more: false,
        timezone,
    }
}

/// The page's items from up to `limit + 1` rows, and whether there are more.
async fn items(
    conn: &mut PgConnection,
    viewer: i64,
    mut rows: Vec<EventRow>,
    page: &Page,
    zones: &Zones,
) -> ApiResult<(Vec<ActivityItem>, bool)> {
    let limit = usize::try_from(page.limit).unwrap_or(0);
    let has_more = rows.len() > limit;
    rows.truncate(limit);

    let refs: Vec<BookRef> = rows
        .iter()
        .filter_map(|r| r.work_id.map(BookRef::work))
        .collect();
    let books = book_outs(conn, &refs).await?;
    let author_ids: Vec<i64> = rows
        .iter()
        .filter(|row| row.kind == "reaction")
        .filter_map(|row| row.payload.get("author_id").and_then(Value::as_i64))
        .collect();
    let authors: HashMap<i64, String> = sqlx::query!(
        "SELECT id, username FROM users WHERE id = ANY($1)",
        &author_ids
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|row| (row.id, row.username))
    .collect();

    let items = rows
        .into_iter()
        .map(|row| {
            let (club, zone) = match row.club_id.and_then(|id| zones.clubs.get(&id)) {
                Some((club, zone)) => (Some(club.clone()), zone),
                None => (None, &zones.personal),
            };
            let payload = served_payload(&row, &authors);
            ActivityItem {
                id: row.id,
                mine: row.actor_id == Some(viewer),
                actor: row.actor,
                book: row
                    .work_id
                    .and_then(|id| books.get(&BookRef::work(id)).cloned()),
                pick_id: row.pick_id,
                club,
                payload,
                created_label: tz::label(row.created_at, zone),
                created_at: row.created_at,
                kind: row.kind,
            }
        })
        .collect();
    Ok((items, has_more))
}

/// The payload as the API serves it: `reaction` names its post's author (`author`, from
/// `author_id`), a `shelf_finished` the caller may not rate (sealed to its actor, or its entry
/// only-me or gone) loses `rating` and `take`, the seal itself is never shown, and a
/// `quote_added` shows the quote as it is now (`excerpt`, `page`), never the text it was added
/// with: that text may have been only-me, and an edit made the quote what its owner shares.
fn served_payload(row: &EventRow, authors: &HashMap<i64, String>) -> Value {
    let Value::Object(mut payload) = row.payload.clone() else {
        return Value::Object(serde_json::Map::new());
    };
    match row.kind.as_str() {
        "reaction" => {
            let author = payload
                .remove("author_id")
                .and_then(|id| id.as_i64())
                .and_then(|id| authors.get(&id).cloned());
            payload.insert("author".into(), author.map_or(Value::Null, Value::String));
        }
        "shelf_finished" => {
            payload.remove(PRIVATE_KEY);
            if row.hide_rating {
                payload.remove("rating");
                payload.remove("take");
            }
        }
        "quote_added" => {
            if let Some(body) = &row.quote_body {
                payload.insert("excerpt".into(), truncate_chars(body, EXCERPT_CHARS).into());
                match row.quote_page {
                    Some(page) => payload.insert("page".into(), page.into()),
                    None => payload.remove("page"),
                };
            }
        }
        _ => {}
    }
    Value::Object(payload)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn row(kind: &str, payload: Value, hide_rating: bool) -> EventRow {
        EventRow {
            id: 1,
            club_id: None,
            actor_id: Some(1),
            actor: Some("ada".into()),
            kind: kind.into(),
            work_id: Some(1),
            pick_id: None,
            payload,
            created_at: OffsetDateTime::UNIX_EPOCH,
            quote_body: None,
            quote_page: None,
            hide_rating,
        }
    }

    #[test]
    fn reactions_name_the_author_from_the_id() {
        let authors = HashMap::from([(7, "grace".to_owned())]);
        let served = served_payload(
            &row(
                "reaction",
                json!({"emoji": "❤️", "post_id": 3, "author_id": 7}),
                false,
            ),
            &authors,
        );
        assert_eq!(
            served,
            json!({"emoji": "❤️", "post_id": 3, "author": "grace"})
        );
        let gone = served_payload(
            &row("reaction", json!({"emoji": "🔥", "author_id": 8}), false),
            &authors,
        );
        assert_eq!(gone, json!({"emoji": "🔥", "author": null}));
        let unknown = served_payload(&row("reaction", json!({"emoji": "🔥"}), false), &authors);
        assert_eq!(unknown, json!({"emoji": "🔥", "author": null}));
    }

    #[test]
    fn the_seal_is_never_served() {
        let payload = json!({"rating": 2, "take": "Meh", "private": true});
        let authors = HashMap::new();
        assert_eq!(
            served_payload(&row("shelf_finished", payload.clone(), false), &authors),
            json!({"rating": 2, "take": "Meh"})
        );
        assert_eq!(
            served_payload(&row("shelf_finished", payload, true), &authors),
            json!({})
        );
    }

    #[test]
    fn a_hidden_rating_drops_rating_and_take_only() {
        let payload = json!({"rating": 4, "take": "Great", "extra": 1});
        let authors = HashMap::new();
        assert_eq!(
            served_payload(&row("shelf_finished", payload.clone(), true), &authors),
            json!({"extra": 1})
        );
        assert_eq!(
            served_payload(&row("shelf_finished", payload.clone(), false), &authors),
            payload
        );
        // Only shelf_finished is redacted this way.
        assert_eq!(
            served_payload(&row("shelf_added", payload.clone(), true), &authors),
            payload
        );
    }
}

//! Club access (docs/phase1/conventions.md §5.3). Frozen: every `/api/clubs/{slug}/…` handler
//! takes one of the extractors, and club rows reached any other way go through `member_by_slug`
//! or `role_in`.

use std::collections::HashMap;

use axum::extract::rejection::RawPathParamsRejection;
use axum::extract::{FromRequestParts, RawPathParams};
use axum::http::request::Parts;
use serde::Serialize;
use sqlx::PgConnection;
use utoipa::ToSchema;

use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::dto::ClubRef;
use crate::error::{ApiResult, AppError, msg};

/// A member's role in a club, ordered `member < admin < owner`.
// The schema is `ClubRole`: `ContributorOut.role` is another kind of role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, sqlx::Type, Serialize, ToSchema)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
#[schema(as = ClubRole)]
pub enum Role {
    Member,
    Admin,
    Owner,
}

impl Role {
    /// The stored and serialised name: `member`, `admin` or `owner`.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Member => "member",
            Role::Admin => "admin",
            Role::Owner => "owner",
        }
    }
}

/// The club of a `/api/clubs/{slug}/…` request.
#[derive(Debug, Clone)]
pub struct ClubCtx {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub timezone: String,
}

/// A signed-in member of the club in the path; 404 `No such club` for a missing club and for a
/// non-member alike.
#[derive(Debug, Clone)]
pub struct ClubMember {
    pub user: CurrentUser,
    pub club: ClubCtx,
    pub role: Role,
}

/// A member with role admin or owner; else 403 `Only club admins can do that`.
#[derive(Debug, Clone)]
pub struct ClubAdmin(pub ClubMember);

/// The club's owner; else 403 `Only the club owner can do that`.
#[derive(Debug, Clone)]
pub struct ClubOwner(pub ClubMember);

impl FromRequestParts<AppState> for ClubMember {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        let slug = path_slug(parts).await?;
        let mut conn = state.db.acquire().await?;
        member_by_slug(&mut conn, &user, &slug).await
    }
}

/// The `{slug}` path parameter. When any parameter of the path is not valid UTF-8, axum reads
/// none of them: such a path names no club the caller can see, so 404 `No such club`. A slug
/// with a NUL character (`%00`) names none either (and Postgres could not compare it).
async fn path_slug(parts: &mut Parts) -> ApiResult<String> {
    let params = match RawPathParams::from_request_parts(parts, &()).await {
        Ok(params) => params,
        Err(RawPathParamsRejection::InvalidUtf8InPathParam(_)) => {
            return Err(AppError::not_found(msg::NO_SUCH_CLUB));
        }
        Err(rejection) => return Err(AppError::internal(rejection.body_text())),
    };
    let slug = params
        .iter()
        .find(|(key, _)| *key == "slug")
        .map(|(_, value)| value.to_owned())
        .ok_or_else(|| AppError::internal("club extractor on a route without {slug}"))?;
    if slug.contains('\0') {
        return Err(AppError::not_found(msg::NO_SUCH_CLUB));
    }
    Ok(slug)
}

impl FromRequestParts<AppState> for ClubAdmin {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let member = ClubMember::from_request_parts(parts, state).await?;
        if member.role >= Role::Admin {
            Ok(ClubAdmin(member))
        } else {
            Err(AppError::forbidden(msg::ONLY_ADMINS))
        }
    }
}

impl FromRequestParts<AppState> for ClubOwner {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let member = ClubMember::from_request_parts(parts, state).await?;
        if member.role == Role::Owner {
            Ok(ClubOwner(member))
        } else {
            Err(AppError::forbidden(msg::ONLY_OWNER))
        }
    }
}

/// The caller's membership of the club with this slug. 404 `No such club` for an unknown slug or
/// a non-member. Also for `?club=<slug>` query parameters, which `RawPathParams` cannot read.
pub async fn member_by_slug(
    conn: &mut PgConnection,
    user: &CurrentUser,
    slug: &str,
) -> ApiResult<ClubMember> {
    if slug.contains('\0') {
        return Err(AppError::not_found(msg::NO_SUCH_CLUB));
    }
    let row = sqlx::query!(
        r#"SELECT c.id, c.slug, c.name, c.timezone, m.role AS "role: Role"
           FROM clubs c
           JOIN club_memberships m ON m.club_id = c.id AND m.user_id = $2
           WHERE c.slug = $1"#,
        slug,
        user.id
    )
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NO_SUCH_CLUB))?;
    Ok(ClubMember {
        user: user.clone(),
        club: ClubCtx {
            id: row.id,
            slug: row.slug,
            name: row.name,
            timezone: row.timezone,
        },
        role: row.role,
    })
}

/// The user's role in a club, `None` for a non-member: for club rows reached by a global id.
pub async fn role_in(
    conn: &mut PgConnection,
    user_id: i64,
    club_id: i64,
) -> sqlx::Result<Option<Role>> {
    sqlx::query_scalar!(
        r#"SELECT role AS "role: Role" FROM club_memberships WHERE club_id = $1 AND user_id = $2"#,
        club_id,
        user_id
    )
    .fetch_optional(conn)
    .await
}

/// The `ClubRef` (slug, name) of each existing club id, for DTO annotation. This loader checks
/// nothing: callers pass only club ids already restricted to the viewer (`ClubMember`,
/// `member_by_slug`, `role_in`, or `JOIN bc_member_clubs($viewer)`), never every club whose row
/// matches (a `club_picks` list of all clubs reading a work would name foreign clubs).
pub async fn club_refs(
    conn: &mut PgConnection,
    club_ids: &[i64],
) -> sqlx::Result<HashMap<i64, ClubRef>> {
    let rows = sqlx::query!(
        "SELECT id, slug, name FROM clubs WHERE id = ANY($1)",
        club_ids
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.id,
                ClubRef {
                    slug: row.slug,
                    name: row.name,
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::extract::Request;
    use axum::http::StatusCode;
    use axum::routing::get;
    use tower::ServiceExt;

    use super::*;

    #[test]
    fn roles_are_ordered() {
        assert!(Role::Member < Role::Admin && Role::Admin < Role::Owner);
        assert_eq!(Role::Owner.as_str(), "owner");
    }

    async fn slug_of(request: Request) -> ApiResult<String> {
        let (mut parts, _) = request.into_parts();
        path_slug(&mut parts).await
    }

    async fn call(path: &str) -> (StatusCode, String) {
        let app = Router::new().route("/api/clubs/{slug}/picks/{pick_id}", get(slug_of));
        let request = Request::get(path).body(Body::empty()).unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn the_slug_is_read_percent_decoded() {
        assert_eq!(
            call("/api/clubs/thursday-readers/picks/1").await,
            (StatusCode::OK, "thursday-readers".to_owned())
        );
        assert_eq!(
            call("/api/clubs/a%20b/picks/1").await,
            (StatusCode::OK, "a b".to_owned())
        );
    }

    #[tokio::test]
    async fn an_undecodable_path_parameter_is_no_such_club() {
        for path in [
            "/api/clubs/%ff/picks/1",
            "/api/clubs/thursday-readers/picks/%ff",
            "/api/clubs/a%00b/picks/1",
        ] {
            assert_eq!(
                call(path).await,
                (
                    StatusCode::NOT_FOUND,
                    r#"{"detail":"No such club"}"#.to_owned()
                ),
                "{path}"
            );
        }
    }
}

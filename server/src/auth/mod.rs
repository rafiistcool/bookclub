//! Accounts and auth (#39): sessions, bearer tokens, passwords, email flows, account deletion
//! and export (docs/phase1/conventions.md §5, endpoint-map §2).

pub mod account;
pub mod address;
pub mod deletion;
pub mod export;
pub mod extract;
pub mod login;
pub mod mail;
pub mod origin;
pub mod password;
pub mod random;
pub mod reset;
pub mod rules;
pub mod session;
pub mod signup;
pub mod tokens;

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use sqlx::{Connection, PgConnection};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;
use crate::clubs::extract::Role;
use crate::dto::{ClubSummary, UserOut};
use crate::error::{ApiResult, AppError};
use crate::legal::{self, LegalDoc};
use crate::media::delete_prefixes;
use crate::media::shared::MediaStore;
use crate::media::shared::urls::{avatar_url, storage_prefix};

/// Name of the session cookie when it is not `Secure` (`COOKIE_SECURE=false`, the DEV_MODE
/// default). `session::cookie_name` picks the name for a configuration.
pub const SESSION_COOKIE: &str = "bookclub_session";
/// Name of the `Secure` session cookie: the `__Host-` prefix makes browsers refuse it unless
/// this host set it, with `Path=/` and no `Domain`.
pub const SECURE_SESSION_COOKIE: &str = "__Host-bookclub_session";

/// The service accent colours: the legacy defaults (docs/phase1/spec/auth.md §1.20).
const THEME: &str = "#b44a2a";
const THEME_DARK: &str = "#8e361c";

/// The auth routes, plus `GET /api/config`.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(service_config))
        .routes(routes!(signup::register))
        .routes(routes!(login::login))
        .routes(routes!(login::logout))
        .routes(routes!(account::me, deletion::delete_account))
        .routes(routes!(account::update_preferences))
        .routes(routes!(account::change_password))
        .routes(routes!(account::accept_terms))
        .routes(routes!(address::verify_email))
        .routes(routes!(address::preview_verification))
        .routes(routes!(address::resend_verification))
        .routes(routes!(address::change_email))
        .routes(routes!(address::dismiss_email_prompt))
        .routes(routes!(reset::request_reset))
        .routes(routes!(reset::confirm_reset))
        .routes(routes!(export::export_me))
        .routes(routes!(
            account::notifications,
            account::update_notifications
        ))
        .routes(routes!(tokens::create_token))
        .routes(routes!(tokens::list_sessions))
        .routes(routes!(tokens::revoke_session))
}

/// Public service branding and legal settings for the SPA.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceConfig {
    /// `SERVICE_NAME`.
    pub name: String,
    /// The accent colour, `#rrggbb`.
    pub theme: String,
    /// The accent colour in dark mode.
    pub theme_dark: String,
    /// `PUBLIC_URL`, an origin.
    pub public_url: String,
    /// `"invite"` (sign-up needs an invite code) or `"open"`.
    pub signup: String,
    /// The documents `GET /api/legal/{doc}` serves: the operator's (`LEGAL_DIR`), or the
    /// DEV_MODE placeholders. Empty when there are none.
    pub legal: Vec<LegalDoc>,
    /// The current version of the operator's terms. A sign-up must send it as `terms_version`;
    /// null when no terms are configured (nothing to accept).
    pub terms_version: Option<String>,
    /// `CONTACT_EMAIL`: the public contact and takedown address. Null when unset (DEV_MODE).
    pub contact_email: Option<String>,
}

/// The service name, accent colours, public origin, sign-up mode and legal settings. Public;
/// answers from memory.
#[utoipa::path(
    get,
    path = "/api/config",
    operation_id = "auth_get_config",
    tag = "auth",
    responses((status = 200, body = ServiceConfig, description = "Service branding"))
)]
pub async fn service_config(State(state): State<AppState>) -> Json<ServiceConfig> {
    let config = &state.config;
    Json(ServiceConfig {
        name: config.service_name.clone(),
        theme: THEME.to_owned(),
        theme_dark: THEME_DARK.to_owned(),
        public_url: config.public_url.clone(),
        signup: config.signup_mode.as_str().to_owned(),
        legal: config.legal.documents(),
        terms_version: config.legal.terms_version().map(str::to_owned),
        contact_email: config.contact_email.clone(),
    })
}

/// The full `UserOut` of `user_id`, including `clubs` (frozen; MED's avatar endpoints return
/// it). A user that no longer exists is 401 `Not signed in`.
///
/// - `pending_email` is the address of the open verification link of an account whose own
///   address is verified (an email change waiting for confirmation); null otherwise.
/// - `needs_email_prompt` is `email IS NULL AND email_prompt_dismissed_at IS NULL`.
/// - `needs_terms` compares `terms_version` with the current terms of the configuration serving
///   the request (`legal::needs_terms`); false outside a request.
/// - `clubs` are ordered newest join first.
pub async fn user_out(conn: &mut PgConnection, user_id: i64) -> ApiResult<UserOut> {
    let Some(row) = sqlx::query!(
        r#"SELECT u.id, u.username, u.email, u.email_verified_at IS NOT NULL AS "email_verified!",
                  (SELECT t.email FROM email_tokens t
                   WHERE t.user_id = u.id AND t.purpose = 'verify_email'
                     AND t.used_at IS NULL AND t.expires_at > now()
                   ORDER BY t.id DESC LIMIT 1) AS open_verification,
                  (u.email IS NULL AND u.email_prompt_dismissed_at IS NULL) AS "needs_email_prompt!",
                  u.terms_version, u.theme, u.color_mode, u.locale, u.timezone, u.is_admin,
                  m.public_id AS "avatar_public_id?"
           FROM users u
           LEFT JOIN media m ON m.id = u.avatar_media_id AND m.removed_at IS NULL
           WHERE u.id = $1"#,
        user_id
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Err(AppError::unauthorized());
    };
    let clubs = sqlx::query_as!(
        ClubSummary,
        r#"SELECT c.slug, c.name, m.role AS "role: Role", c.timezone,
                  (SELECT count(*) FROM club_memberships x WHERE x.club_id = c.id) AS "member_count!"
           FROM club_memberships m
           JOIN clubs c ON c.id = m.club_id
           WHERE m.user_id = $1
           ORDER BY m.joined_at DESC, c.id DESC"#,
        user_id
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(UserOut {
        id: row.id,
        username: row.username,
        pending_email: row.open_verification.filter(|_| row.email_verified),
        email: row.email,
        email_verified: row.email_verified,
        needs_email_prompt: row.needs_email_prompt,
        needs_terms: legal::needs_terms(row.terms_version.as_deref()),
        theme: row.theme,
        color_mode: row.color_mode,
        locale: row.locale,
        timezone: row.timezone,
        avatar_url: avatar_url(row.avatar_public_id.as_deref()),
        is_admin: row.is_admin,
        clubs,
    })
}

/// Empty sign-ups, and photos of rejected cover requests, deleted per run at most
/// ([`purge_empty_sign_ups`], [`delete_rejected_cover_photos`]); the rest wait for the next
/// hour.
pub const PURGE_BATCH: i64 = 500;

/// The hourly cleanup job and the retention periods of Q14 (docs/legal/privacy-facts.md §7).
/// It deletes:
///
/// - expired sessions, and used or expired email tokens;
/// - reports and catalog edit requests 3 years after their decision (`resolved_at`,
///   `decided_at`); open ones stay however old they are. A rejected cover request whose photo is
///   still stored is left to [`delete_rejected_cover_photos`], which deletes the photo with it:
///   deleted here, the request would leave its photo behind with nothing referencing it;
/// - empty sign-ups, 30 days after their last sign-in ([`purge_empty_sign_ups`]).
pub async fn cleanup(conn: &mut PgConnection) -> sqlx::Result<()> {
    let sessions = sqlx::query!("DELETE FROM sessions WHERE expires_at < now()")
        .execute(&mut *conn)
        .await?
        .rows_affected();
    let tokens =
        sqlx::query!("DELETE FROM email_tokens WHERE used_at IS NOT NULL OR expires_at < now()")
            .execute(&mut *conn)
            .await?
            .rows_affected();
    let reports = sqlx::query!(
        "DELETE FROM reports WHERE status <> 'open' AND resolved_at < now() - interval '3 years'"
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    let edit_requests = sqlx::query!(
        "DELETE FROM catalog_edit_requests r
         WHERE r.status <> 'open' AND r.decided_at < now() - interval '3 years'
           AND NOT (r.kind = 'cover_replace' AND r.status = 'rejected'
                    AND EXISTS (SELECT 1 FROM media m
                                WHERE m.id = r.media_id AND m.removed_at IS NULL))"
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    let sign_ups = purge_empty_sign_ups(conn).await?;
    tracing::info!(
        sessions,
        tokens,
        reports,
        edit_requests,
        sign_ups,
        "deleted expired sessions and email tokens, old decisions and empty sign-ups"
    );
    Ok(())
}

/// Deletes the photo of a rejected cover request (`cover_replace`) together with the request,
/// 3 years after the decision (Q14), and then the photo's files. Returns how many photos it
/// deleted, at most [`PURGE_BATCH`] a run.
///
/// Such a photo was never a cover, so nothing shows it, but its row names the uploader and its
/// URL still serves it: [`cleanup`] leaves these requests here so that the photo goes with its
/// request. It belongs in the hourly cleanup job, right after [`cleanup`], and unlike it needs
/// the media store (`jobs::auth_cleanup_loop` has `state.media`). A photo that anything else
/// references (a work, an edition, an account, a report, a request that is not due) stays, with
/// its request, until nothing does. A taken-down photo has no files left; its row stays like
/// every taken-down picture's, and [`cleanup`] deletes its request.
///
/// Each photo is locked first, media rows before requests as a takedown and an approval take
/// them (conventions §7.2), and then looked at again in a new statement: a write that would
/// reference it (a cover, a report) holds the row `FOR SHARE` or `FOR KEY SHARE`, so it has
/// either committed and is seen, or it waits and finds the photo gone.
pub async fn delete_rejected_cover_photos(
    conn: &mut PgConnection,
    store: &dyn MediaStore,
) -> sqlx::Result<u64> {
    let due = sqlx::query_scalar!(
        r#"SELECT DISTINCT r.media_id AS "media_id!"
           FROM catalog_edit_requests r
           JOIN media m ON m.id = r.media_id
           WHERE r.kind = 'cover_replace' AND r.status = 'rejected'
             AND r.decided_at < now() - interval '3 years' AND m.removed_at IS NULL
           ORDER BY 1
           LIMIT $1"#,
        PURGE_BATCH
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut deleted = 0;
    for media_id in due {
        let mut tx = conn.begin().await?;
        let photo = sqlx::query!(
            "SELECT kind, public_id, storage_key FROM media
             WHERE id = $1 AND removed_at IS NULL
             FOR UPDATE",
            media_id
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(photo) = photo else {
            tx.rollback().await?;
            continue;
        };
        let in_use = sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM works WHERE cover_media_id = $1)
                   OR EXISTS (SELECT 1 FROM editions WHERE cover_media_id = $1)
                   OR EXISTS (SELECT 1 FROM users WHERE avatar_media_id = $1)
                   OR EXISTS (SELECT 1 FROM reports WHERE media_id = $1)
                   OR EXISTS (SELECT 1 FROM media_removed_hashes WHERE media_id = $1)
                   OR EXISTS (SELECT 1 FROM catalog_edit_requests
                              WHERE media_id = $1
                                AND NOT (kind = 'cover_replace' AND status = 'rejected'
                                         AND decided_at < now() - interval '3 years'))
               AS "in_use!""#,
            media_id
        )
        .fetch_one(&mut *tx)
        .await?;
        if in_use {
            tx.rollback().await?;
            continue;
        }
        // The request goes with it (ON DELETE CASCADE).
        sqlx::query!("DELETE FROM media WHERE id = $1", media_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let mut prefixes = vec![storage_prefix(&photo.kind, &photo.public_id)];
        prefixes.extend(photo.storage_key);
        delete_prefixes(store, &prefixes).await;
        deleted += 1;
    }
    Ok(deleted)
}

/// Deletes up to [`PURGE_BATCH`] empty sign-ups ([`is_empty_sign_up`]), lowest id first, each in
/// a transaction of its own through [`deletion::delete_user`], the path of `DELETE /api/auth/me`.
/// Returns how many it deleted. An account whose deletion fails is logged without naming it and
/// kept, and the others go on; a failure of the connection itself ends the run.
///
/// Each account is locked first, then its sessions, and then it is looked at again in a new
/// statement. A write that gives the account data or a new session references it, so it holds
/// the users row `FOR KEY SHARE` until it commits; using an existing session updates only that
/// session's row, which the second lock waits for. Once both locks are granted, such a write
/// has either committed, and the second look sees it, or it waits for this transaction and
/// fails or changes nothing when the account is gone. The users row comes before the sessions
/// as in a password change and an account deletion, and a logout's trace
/// (`users.last_seen_at`, migration 2201) never waits for the users row, so no lock order is
/// reversed. Taking the account's lock before any club's breaks no lock order either
/// (conventions §5.6): an empty account has no row in a club, so `delete_user` locks no club.
pub async fn purge_empty_sign_ups(conn: &mut PgConnection) -> sqlx::Result<u64> {
    let candidates = empty_sign_ups(conn, None, PURGE_BATCH).await?;
    let mut deleted = 0;
    for user_id in candidates {
        let mut tx = conn.begin().await?;
        let hash = sqlx::query_scalar!(
            "SELECT password_hash FROM users WHERE id = $1 FOR UPDATE",
            user_id
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(hash) = hash else {
            tx.rollback().await?;
            continue;
        };
        sqlx::query!(
            "SELECT id FROM sessions WHERE user_id = $1 FOR UPDATE",
            user_id
        )
        .fetch_all(&mut *tx)
        .await?;
        if !is_empty_sign_up(&mut tx, user_id).await? {
            tx.rollback().await?;
            continue;
        }
        let prefixes = match deletion::delete_user(&mut tx, user_id, &hash).await {
            Ok(prefixes) => prefixes,
            Err(error) => {
                // A database error (logged by `AppError::internal`); the account's own checks
                // cannot fail for an empty account under its lock. The log names no account:
                // the operator finds the ones left with the query of `empty_sign_ups`.
                tracing::error!(%error, "deleting an empty sign-up failed; kept");
                tx.rollback().await?;
                continue;
            }
        };
        if !prefixes.is_empty() {
            // Unreachable while `empty_sign_ups` requires no picture. The job has no media store
            // to delete the files with, so it never deletes rows whose files would stay behind.
            tracing::error!(
                pictures = prefixes.len(),
                "an empty sign-up has pictures; kept"
            );
            tx.rollback().await?;
            continue;
        }
        tx.commit().await?;
        deleted += 1;
    }
    Ok(deleted)
}

/// Whether the account `user_id` is an empty sign-up of Q14 now ([`empty_sign_ups`]): the
/// purge's second look under its locks.
pub async fn is_empty_sign_up(conn: &mut PgConnection, user_id: i64) -> sqlx::Result<bool> {
    Ok(!empty_sign_ups(conn, Some(user_id), 1).await?.is_empty())
}

/// The empty sign-ups of Q14, lowest id first, at most `limit`; with `only`, that account if it
/// is one. An empty sign-up is an account that:
///
/// - **has an address it never verified.** Sign-up always stores the address it was given, so an
///   account without any is an imported one that never added an address, and it stays. An
///   imported account that typed an address without verifying it stays while it holds data.
/// - **is not an instance admin.**
/// - **shows no sign-in for 30 days:** it was created, changed its password, accepted the terms
///   and last used a session that has ended (`last_seen_at`) more than 30 days ago, and no
///   session of it was used and no mail link was sent to it since then.
/// - **holds no data:** no profile picture, and no row behind any foreign key to `users` other
///   than the traces of signing up and signing in: its sessions, its mail links and the use of
///   a sign-up invite (`club_invites.club_id` NULL). The use of a club invite is data.
///
/// One `NOT EXISTS` per foreign-key column. `tests/jobs.rs`
/// (`every_reference_to_an_account_is_data_or_a_trace_of_signing_in`) reads every foreign key
/// to `users` from `pg_constraint` and gives an account a row behind each: a new one fails there
/// until it has a line here, or is named a trace in that test. A new column of `users` fails
/// `every_column_of_users_is_considered_by_the_purge` the same way.
async fn empty_sign_ups(
    conn: &mut PgConnection,
    only: Option<i64>,
    limit: i64,
) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar!(
        "SELECT u.id FROM users u
         WHERE ($1::bigint IS NULL OR u.id = $1)
           AND u.email IS NOT NULL
           AND u.email_verified_at IS NULL
           AND NOT u.is_admin
           -- no sign-in for 30 days
           AND greatest(u.created_at, u.password_changed_at, u.terms_accepted_at,
                        u.last_seen_at)
               < now() - interval '30 days'
           AND NOT EXISTS (SELECT 1 FROM sessions x
                           WHERE x.user_id = u.id AND x.last_used_at >= now() - interval '30 days')
           AND NOT EXISTS (SELECT 1 FROM email_tokens x
                           WHERE x.user_id = u.id AND x.created_at >= now() - interval '30 days')
           -- no data
           AND u.avatar_media_id IS NULL
           AND NOT EXISTS (SELECT 1 FROM shelf_entries x WHERE x.user_id = u.id)
           AND NOT EXISTS (SELECT 1 FROM physical_shelf_layouts x WHERE x.user_id = u.id)
           AND NOT EXISTS (SELECT 1 FROM user_favorites x WHERE x.user_id = u.id)
           AND NOT EXISTS (SELECT 1 FROM search_history x WHERE x.user_id = u.id)
           AND NOT EXISTS (SELECT 1 FROM quotes x WHERE x.user_id = u.id)
           AND NOT EXISTS (SELECT 1 FROM import_jobs x WHERE x.user_id = u.id)
           AND NOT EXISTS (SELECT 1 FROM posts x WHERE x.author_id = u.id)
           AND NOT EXISTS (SELECT 1 FROM post_reactions x WHERE x.user_id = u.id)
           AND NOT EXISTS (SELECT 1 FROM clubs x WHERE x.created_by = u.id)
           AND NOT EXISTS (SELECT 1 FROM club_memberships x WHERE x.user_id = u.id)
           AND NOT EXISTS (SELECT 1 FROM club_memberships x WHERE x.invited_by = u.id)
           AND NOT EXISTS (SELECT 1 FROM club_removals x WHERE x.user_id = u.id)
           AND NOT EXISTS (SELECT 1 FROM club_invites x WHERE x.created_by = u.id)
           AND NOT EXISTS (SELECT 1 FROM club_invite_uses x
                           JOIN club_invites i ON i.id = x.invite_id
                           WHERE x.user_id = u.id AND i.club_id IS NOT NULL)
           AND NOT EXISTS (SELECT 1 FROM club_picks x WHERE x.set_by = u.id)
           AND NOT EXISTS (SELECT 1 FROM milestones x WHERE x.created_by = u.id)
           AND NOT EXISTS (SELECT 1 FROM next_up_nominations x WHERE x.nominated_by = u.id)
           AND NOT EXISTS (SELECT 1 FROM next_up_ballots x WHERE x.user_id = u.id)
           AND NOT EXISTS (SELECT 1 FROM events x WHERE x.actor_id = u.id)
           AND NOT EXISTS (SELECT 1 FROM works x WHERE x.created_by = u.id)
           AND NOT EXISTS (SELECT 1 FROM editions x WHERE x.created_by = u.id)
           AND NOT EXISTS (SELECT 1 FROM media x WHERE x.uploaded_by = u.id)
           AND NOT EXISTS (SELECT 1 FROM catalog_edit_requests x WHERE x.created_by = u.id)
           AND NOT EXISTS (SELECT 1 FROM catalog_edit_requests x WHERE x.decided_by = u.id)
           AND NOT EXISTS (SELECT 1 FROM reports x WHERE x.created_by = u.id)
           AND NOT EXISTS (SELECT 1 FROM reports x WHERE x.resolved_by = u.id)
         ORDER BY u.id
         LIMIT $2",
        only,
        limit
    )
    .fetch_all(conn)
    .await
}

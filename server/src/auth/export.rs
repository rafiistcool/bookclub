//! `GET /api/auth/me/export`: the DSGVO export of the signed-in user's data (endpoint-map §2,
//! docs/phase1/conventions.md §5.7).
//!
//! A ZIP with `data.json`, one key per foreign-key column that references `users(id)`, and
//! `media/`, the bytes of every picture the user uploaded. The rows are read in one
//! repeatable-read transaction, so the sections agree with each other. Most rows are the table
//! row as Postgres renders it (`to_jsonb`) minus the reference to the user, plus the names that
//! make it readable (work titles, club names).
//!
//! The ZIP is streamed: the pictures are read one at a time and each is sent as soon as it is
//! written, then `data.json` (in slices, as it is deflated) and the central directory follow.
//! An export therefore holds at most one picture and `data.json` in memory, however many
//! pictures the user uploaded. Two exports stream at once on the instance, one per account; a
//! client that stops reading for a minute, or a download still running after an hour, ends
//! the export and frees what it holds.

use std::io::{self, Write};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures::stream;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};
use tokio::sync::{OwnedSemaphorePermit, mpsc};
use zip::write::{SimpleFileOptions, StreamWriter};
use zip::{CompressionMethod, ZipWriter};

use super::account::account_key;
use super::extract::CurrentUser;
use crate::AppState;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::media::shared::MediaStore;
use crate::ratelimit::GatePass;
use crate::tz;

/// A piece of the response body; an error aborts the download.
type Chunk = io::Result<Bytes>;

/// The longest the export waits for the client to take the next chunk. A client that stops
/// reading (a zero TCP window, a stalled HTTP/2 stream) ends the export, so its rows and its
/// slot are not held until the connection dies.
const SEND_TIMEOUT: Duration = Duration::from_secs(60);
/// The longest an export streams in all, however slowly its client reads.
const EXPORT_DEADLINE: Duration = Duration::from_secs(3600);
/// `data.json` is deflated and sent in slices of this many bytes.
const JSON_SLICE: usize = 256 * 1024;

/// What an export holds while it streams: its account's turn and one of the instance's slots.
struct ExportTurn {
    _account: GatePass,
    _slot: OwnedSemaphorePermit,
}

/// Downloads everything the service stores about the signed-in user. Five exports an hour, one
/// at a time per account and two at a time on the instance.
#[utoipa::path(
    get,
    path = "/api/auth/me/export",
    operation_id = "auth_export_me",
    tag = "auth",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, content_type = "application/zip",
         description = "`bookclub-export-<username>-<YYYYMMDD>.zip` with `data.json` and `media/`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (5 an hour, another export of the account still running, or two exports running on the instance)")
    )
)]
pub async fn export_me(user: CurrentUser, State(state): State<AppState>) -> ApiResult<Response> {
    // Taken before the rows are read, and held until the download ends (security review
    // 2026-10-28 #6): each export holds the account's whole `data.json` in memory.
    let key = account_key(user.id);
    let busy = || AppError::too_many(msg::SLOW_DOWN, None);
    let account = state.limits.export_gate.try_enter(&key).ok_or_else(busy)?;
    let slot = state
        .limits
        .exports
        .clone()
        .try_acquire_owned()
        .map_err(|_| busy())?;
    state
        .limits
        .export_user
        .check_and_hit(&key)
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let turn = ExportTurn {
        _account: account,
        _slot: slot,
    };
    let mut tx = state
        .db
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .await?;
    // Postgres renders timestamps in the session time zone; `utc_timestamps` relies on UTC.
    sqlx::query_scalar!("SELECT set_config('TimeZone', 'UTC', true)")
        .fetch_one(&mut *tx)
        .await?;
    let mut data = Map::new();
    account_sections(&mut tx, user.id, &mut data).await?;
    club_sections(&mut tx, user.id, &mut data).await?;
    catalog_sections(&mut tx, user.id, &mut data).await?;
    let media = uploaded_media(&mut tx, user.id).await?;
    tx.commit().await?;

    let today = tz::today(&tz::zone_or_utc(&user.timezone))
        .format(format_description!("[year][month][day]"))
        .map_err(AppError::internal)?;
    let disposition = format!(
        "attachment; filename=\"bookclub-export-{}-{today}.zip\"",
        user.username
    );
    // One chunk in flight: the producer waits for the client instead of reading ahead.
    let (sender, receiver) = mpsc::channel::<Chunk>(1);
    tokio::spawn(stream_zip(state.media.clone(), data, media, sender, turn));
    let body = Body::from_stream(stream::unfold(receiver, |mut receiver| async move {
        let chunk = receiver.recv().await?;
        Some((chunk, receiver))
    }));
    Ok((
        [
            (CONTENT_TYPE, "application/zip".to_owned()),
            (CONTENT_DISPOSITION, disposition),
            (CACHE_CONTROL, "no-store".to_owned()),
        ],
        body,
    )
        .into_response())
}

/// `profile`, `sessions` and the personal data (shelf, layout, favourites, search history,
/// quotes, posts, reactions, import jobs).
///
/// `profile.pending_email` follows `UserOut`: the address of the newest open verification link
/// of an account whose own address is verified, else null. An unverified account's link is for
/// the address in `profile.email`, or, after a sign-up with an address a verified account
/// holds, does not exist; showing it would tell the two sign-ups apart (§5.5 "No
/// enumeration").
async fn account_sections(
    conn: &mut PgConnection,
    user_id: i64,
    data: &mut Map<String, Value>,
) -> ApiResult<()> {
    let profile = sqlx::query_scalar!(
        r#"SELECT (to_jsonb(u) - 'password_hash')
                  || jsonb_build_object('pending_email',
                         CASE WHEN u.email_verified_at IS NOT NULL THEN
                             (SELECT t.email FROM email_tokens t
                              WHERE t.user_id = u.id AND t.purpose = 'verify_email'
                                AND t.used_at IS NULL AND t.expires_at > now()
                              ORDER BY t.id DESC LIMIT 1)
                         END) AS "profile!"
           FROM users u WHERE u.id = $1"#,
        user_id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(AppError::unauthorized)?;
    data.insert("profile".into(), profile);

    // An allow-list: the token hash never leaves the database.
    let sessions = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg(jsonb_build_object(
                      'id', s.id, 'kind', s.kind, 'label', s.label, 'created_at', s.created_at,
                      'last_used_at', s.last_used_at, 'expires_at', s.expires_at)
                  ORDER BY s.id), '[]') AS "rows!"
           FROM sessions s WHERE s.user_id = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("sessions".into(), sessions);

    let shelf = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg((to_jsonb(s) - 'user_id')
                      || jsonb_build_object('work_title', w.title, 'work_authors', w.author_names,
                                            'edition_isbn13', e.isbn13)
                  ORDER BY s.id), '[]') AS "rows!"
           FROM shelf_entries s
           JOIN works w ON w.id = s.work_id
           LEFT JOIN editions e ON e.id = s.edition_id
           WHERE s.user_id = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("shelf_entries".into(), shelf);

    let layout = sqlx::query_scalar!(
        r#"SELECT to_jsonb(l) - 'user_id' AS "layout!"
           FROM physical_shelf_layouts l WHERE l.user_id = $1"#,
        user_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    data.insert(
        "physical_shelf_layout".into(),
        layout.unwrap_or(Value::Null),
    );

    let favorites = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg((to_jsonb(f) - 'user_id')
                      || jsonb_build_object('work_title', w.title)
                  ORDER BY f.position), '[]') AS "rows!"
           FROM user_favorites f JOIN works w ON w.id = f.work_id
           WHERE f.user_id = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("favorites".into(), favorites);

    let searches = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg(to_jsonb(h) - 'user_id'
                  ORDER BY h.last_used_at DESC, h.id DESC), '[]') AS "rows!"
           FROM search_history h WHERE h.user_id = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("search_history".into(), searches);

    let quotes = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg((to_jsonb(q) - 'user_id')
                      || jsonb_build_object('work_title', w.title)
                  ORDER BY q.id), '[]') AS "rows!"
           FROM quotes q JOIN works w ON w.id = q.work_id
           WHERE q.user_id = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("quotes".into(), quotes);

    let posts = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg((to_jsonb(p) - 'author_id' - 'scope_key')
                      || jsonb_build_object('work_title', w.title, 'club_name', c.name)
                  ORDER BY p.id), '[]') AS "rows!"
           FROM posts p
           JOIN works w ON w.id = p.work_id
           LEFT JOIN clubs c ON c.id = p.club_id
           WHERE p.author_id = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("posts".into(), posts);

    let reactions = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg(to_jsonb(r) - 'user_id'
                  ORDER BY r.created_at, r.post_id, r.emoji), '[]') AS "rows!"
           FROM post_reactions r WHERE r.user_id = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("reactions".into(), reactions);

    let imports = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg(to_jsonb(j) - 'user_id' ORDER BY j.id), '[]') AS "rows!"
           FROM import_jobs j WHERE j.user_id = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("import_jobs".into(), imports);
    Ok(())
}

/// Memberships, removals, clubs, invites, picks, milestones, votes and activity events.
///
/// Rows in a club the user no longer belongs to keep only what the user wrote and nobody can
/// change since (ids, dates, the work), plus the club's name: other members' memberships, the
/// note and meeting of a pick, a milestone's title and dates, an invite's code, counters and
/// revocation, and the club's settings are club data a former member no longer sees
/// (DSGVO Art. 15(4)). Invites keep their live state only while the user may list them (an
/// admin of the club, or an instance admin for a sign-up invite).
async fn club_sections(
    conn: &mut PgConnection,
    user_id: i64,
    data: &mut Map<String, Value>,
) -> sqlx::Result<()> {
    let memberships = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg(jsonb_build_object(
                      'club_name', c.name, 'club_slug', c.slug, 'role', m.role,
                      'joined_at', m.joined_at, 'invited_by', i.username)
                  ORDER BY m.joined_at, c.id), '[]') AS "rows!"
           FROM club_memberships m
           JOIN clubs c ON c.id = m.club_id
           LEFT JOIN users i ON i.id = m.invited_by
           WHERE m.user_id = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("memberships".into(), memberships);

    let invited = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg(jsonb_build_object(
                      'club_name', c.name, 'username', u.username, 'joined_at', m.joined_at)
                  ORDER BY m.joined_at, u.username), '[]') AS "rows!"
           FROM club_memberships m
           JOIN clubs c ON c.id = m.club_id
           JOIN users u ON u.id = m.user_id
           WHERE m.invited_by = $1 AND bc_is_member($1, m.club_id)"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("members_invited".into(), invited);

    let removals = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg(jsonb_build_object(
                      'club_name', c.name, 'removed_at', r.removed_at)
                  ORDER BY r.removed_at, c.id), '[]') AS "rows!"
           FROM club_removals r JOIN clubs c ON c.id = r.club_id
           WHERE r.user_id = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("club_removals".into(), removals);

    let clubs = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg(
                      CASE WHEN bc_is_member($1, c.id) THEN to_jsonb(c) - 'created_by'
                           ELSE jsonb_build_object(
                               'id', c.id, 'name', c.name, 'created_at', c.created_at)
                      END
                  ORDER BY c.id), '[]') AS "rows!"
           FROM clubs c WHERE c.created_by = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("clubs_created".into(), clubs);

    let invites = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg(
                      CASE WHEN CASE WHEN i.club_id IS NULL
                                     THEN (SELECT is_admin FROM users WHERE id = $1)
                                     ELSE EXISTS (
                                         SELECT 1 FROM club_memberships m
                                         WHERE m.club_id = i.club_id AND m.user_id = $1
                                           AND m.role IN ('owner', 'admin'))
                                END
                           THEN (to_jsonb(i) - 'created_by')
                                || jsonb_build_object('club_name', c.name)
                           ELSE jsonb_build_object(
                               'id', i.id, 'club_id', i.club_id, 'club_name', c.name,
                               'created_at', i.created_at, 'expires_at', i.expires_at,
                               'max_uses', i.max_uses)
                      END
                  ORDER BY i.id), '[]') AS "rows!"
           FROM club_invites i LEFT JOIN clubs c ON c.id = i.club_id
           WHERE i.created_by = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("invites_created".into(), invites);

    // Not the code: the invite belongs to whoever created it.
    let uses = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg(jsonb_build_object(
                      'invite_id', u.invite_id, 'club_name', c.name, 'used_at', u.used_at)
                  ORDER BY u.used_at, u.id), '[]') AS "rows!"
           FROM club_invite_uses u
           JOIN club_invites i ON i.id = u.invite_id
           LEFT JOIN clubs c ON c.id = i.club_id
           WHERE u.user_id = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("invite_uses".into(), uses);

    let picks = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg(
                      CASE WHEN bc_is_member($1, p.club_id)
                           THEN (to_jsonb(p) - 'set_by')
                                || jsonb_build_object('club_name', c.name, 'work_title', w.title)
                           ELSE jsonb_build_object(
                               'id', p.id, 'club_id', p.club_id, 'club_name', c.name,
                               'work_id', p.work_id, 'work_title', w.title,
                               'started_at', p.started_at)
                      END
                  ORDER BY p.id), '[]') AS "rows!"
           FROM club_picks p
           JOIN clubs c ON c.id = p.club_id
           JOIN works w ON w.id = p.work_id
           WHERE p.set_by = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("picks_set".into(), picks);

    let milestones = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg(
                      CASE WHEN bc_is_member($1, m.club_id)
                           THEN (to_jsonb(m) - 'created_by')
                                || jsonb_build_object('club_name', c.name)
                           ELSE jsonb_build_object(
                               'id', m.id, 'club_id', m.club_id, 'club_name', c.name,
                               'pick_id', m.pick_id, 'created_at', m.created_at)
                      END
                  ORDER BY m.id), '[]') AS "rows!"
           FROM milestones m JOIN clubs c ON c.id = m.club_id
           WHERE m.created_by = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("milestones_created".into(), milestones);

    let nominations = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg((to_jsonb(n) - 'nominated_by')
                      || jsonb_build_object('club_name', c.name, 'work_title', w.title)
                  ORDER BY n.id), '[]') AS "rows!"
           FROM next_up_nominations n
           JOIN clubs c ON c.id = n.club_id
           JOIN works w ON w.id = n.work_id
           WHERE n.nominated_by = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("nominations".into(), nominations);

    let ballots = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg((to_jsonb(b) - 'user_id')
                      || jsonb_build_object('club_name', c.name, 'work_title', w.title)
                  ORDER BY b.created_at, b.vote_id), '[]') AS "rows!"
           FROM next_up_ballots b
           JOIN clubs c ON c.id = b.club_id
           JOIN next_up_nominations n ON n.id = b.nomination_id
           JOIN works w ON w.id = n.work_id
           WHERE b.user_id = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("ballots".into(), ballots);

    let events = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg((to_jsonb(e) - 'actor_id')
                      || jsonb_build_object('club_name', c.name)
                  ORDER BY e.id), '[]') AS "rows!"
           FROM events e LEFT JOIN clubs c ON c.id = e.club_id
           WHERE e.actor_id = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("events".into(), events);
    Ok(())
}

/// Catalog contributions, edit requests and reports, each as filed and as decided.
async fn catalog_sections(
    conn: &mut PgConnection,
    user_id: i64,
    data: &mut Map<String, Value>,
) -> sqlx::Result<()> {
    let contributions = sqlx::query_scalar!(
        r#"SELECT jsonb_build_object(
               'works', (SELECT coalesce(jsonb_agg(jsonb_build_object(
                             'id', w.id, 'title', w.title, 'subtitle', w.subtitle,
                             'author_names', w.author_names,
                             'first_publish_year', w.first_publish_year, 'source', w.source,
                             'created_at', w.created_at) ORDER BY w.id), '[]')
                         FROM works w WHERE w.created_by = $1),
               'editions', (SELECT coalesce(jsonb_agg(jsonb_build_object(
                                'id', e.id, 'work_id', e.work_id, 'isbn13', e.isbn13,
                                'title', e.title, 'publisher', e.publisher, 'source', e.source,
                                'created_at', e.created_at) ORDER BY e.id), '[]')
                            FROM editions e WHERE e.created_by = $1)) AS "rows!""#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("catalog_contributions".into(), contributions);

    let requests = sqlx::query_scalar!(
        r#"SELECT jsonb_build_object(
               'filed', (SELECT coalesce(jsonb_agg(to_jsonb(r) - 'created_by' - 'decided_by'
                             ORDER BY r.id), '[]')
                         FROM catalog_edit_requests r WHERE r.created_by = $1),
               'decided', (SELECT coalesce(jsonb_agg(jsonb_build_object(
                               'id', r.id, 'kind', r.kind, 'work_id', r.work_id,
                               'edition_id', r.edition_id, 'status', r.status,
                               'decided_at', r.decided_at, 'decision_note', r.decision_note)
                           ORDER BY r.id), '[]')
                           FROM catalog_edit_requests r WHERE r.decided_by = $1)) AS "rows!""#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("edit_requests".into(), requests);

    let reports = sqlx::query_scalar!(
        r#"SELECT jsonb_build_object(
               'filed', (SELECT coalesce(jsonb_agg(to_jsonb(r) - 'created_by' - 'resolved_by'
                             ORDER BY r.id), '[]')
                         FROM reports r WHERE r.created_by = $1),
               'resolved', (SELECT coalesce(jsonb_agg(jsonb_build_object(
                                'id', r.id, 'target_kind', r.target_kind,
                                'media_id', r.media_id, 'reason', r.reason, 'status', r.status,
                                'resolved_at', r.resolved_at)
                            ORDER BY r.id), '[]')
                            FROM reports r WHERE r.resolved_by = $1)) AS "rows!""#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("reports".into(), reports);
    Ok(())
}

/// A picture the user uploaded (or their current avatar).
struct MediaRow {
    public_id: String,
    kind: String,
    source: String,
    content_type: Option<String>,
    variants: Vec<String>,
    width: Option<i32>,
    height: Option<i32>,
    byte_size: Option<i64>,
    storage_key: Option<String>,
    created_at: OffsetDateTime,
    removed_at: Option<OffsetDateTime>,
}

impl MediaRow {
    /// The `uploaded_media` entry, with the paths of its files in the ZIP.
    fn to_json(&self, files: Vec<String>) -> Value {
        json!({
            "public_id": self.public_id,
            "kind": self.kind,
            "source": self.source,
            "content_type": self.content_type,
            "width": self.width,
            "height": self.height,
            "byte_size": self.byte_size,
            "created_at": rfc3339_utc(self.created_at),
            "removed_at": self.removed_at.and_then(rfc3339_utc),
            "files": files,
        })
    }
}

async fn uploaded_media(conn: &mut PgConnection, user_id: i64) -> sqlx::Result<Vec<MediaRow>> {
    sqlx::query_as!(
        MediaRow,
        "SELECT public_id, kind, source, content_type, variants, width, height, byte_size,
                storage_key, created_at, removed_at
         FROM media
         WHERE uploaded_by = $1 OR id = (SELECT avatar_media_id FROM users WHERE id = $1)
         ORDER BY id",
        user_id
    )
    .fetch_all(conn)
    .await
}

/// Sends the ZIP through `sender`. The answer has started, so a failure can only abort the
/// download; the archive then lacks its central directory, and no reader takes it for whole.
async fn stream_zip(
    media: Arc<dyn MediaStore>,
    data: Map<String, Value>,
    rows: Vec<MediaRow>,
    sender: mpsc::Sender<Chunk>,
    turn: ExportTurn,
) {
    let written = tokio::time::timeout(
        EXPORT_DEADLINE,
        write_zip(media.as_ref(), data, rows, &sender, SEND_TIMEOUT),
    )
    .await
    .unwrap_or_else(|_| Err(anyhow::anyhow!("the export ran past its deadline")));
    if let Err(error) = written {
        tracing::warn!(error = %format!("{error:#}"), "the export failed");
        // Nothing to do when the client is gone or not reading.
        let _ = sender.try_send(Err(io::Error::other("the export failed")));
    }
    drop(turn);
}

/// Hands `chunk` to the response body within `wait`. `Ok(false)`: the client went away; an
/// error: it stopped reading.
async fn send_chunk(
    sender: &mpsc::Sender<Chunk>,
    chunk: Bytes,
    wait: Duration,
) -> anyhow::Result<bool> {
    match tokio::time::timeout(wait, sender.send(Ok(chunk))).await {
        Ok(sent) => Ok(sent.is_ok()),
        Err(_) => anyhow::bail!("the client stopped reading the export"),
    }
}

/// `media/`: every stored variant of every picture that was not taken down, as
/// `media/<public_id>/<variant>.<webp|jpg>`, each sent once it is written; then `data.json`
/// with the `uploaded_media` rows, serialised once (the rows are dropped then) and deflated and
/// sent slice by slice. Stops quietly when the client goes away, and with an error when it
/// takes no chunk for `wait`.
async fn write_zip(
    media: &dyn MediaStore,
    mut data: Map<String, Value>,
    rows: Vec<MediaRow>,
    sender: &mpsc::Sender<Chunk>,
    wait: Duration,
) -> anyhow::Result<()> {
    let mut zip = ExportZip::new();
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let mut paths = Vec::new();
        if let (None, Some(prefix)) = (row.removed_at, &row.storage_key) {
            let extension = match row.content_type.as_deref() {
                Some("image/jpeg") => "jpg",
                _ => "webp",
            };
            for variant in &row.variants {
                let key = format!("{prefix}/{variant}");
                let Some(bytes) = media.get(&key).await? else {
                    tracing::warn!(key, "an uploaded picture has no stored bytes");
                    continue;
                };
                let path = format!("media/{}/{variant}.{extension}", row.public_id);
                let chunk = zip.add_media(&path, &bytes)?;
                if !send_chunk(sender, chunk, wait).await? {
                    return Ok(());
                }
                paths.push(path);
            }
        }
        items.push(row.to_json(paths));
    }
    data.insert("uploaded_media".into(), Value::Array(items));
    let json = tokio::task::spawn_blocking(move || {
        let mut data = Value::Object(data);
        utc_timestamps(&mut data);
        serde_json::to_vec_pretty(&data).map(Bytes::from)
    })
    .await??;
    zip.start_data()?;
    for start in (0..json.len()).step_by(JSON_SLICE) {
        let slice = json.slice(start..(start + JSON_SLICE).min(json.len()));
        let (back, chunk) = tokio::task::spawn_blocking(move || {
            let chunk = zip.write_data(&slice);
            (zip, chunk)
        })
        .await?;
        zip = back;
        let chunk = chunk?;
        if !chunk.is_empty() && !send_chunk(sender, chunk, wait).await? {
            return Ok(());
        }
    }
    let last = tokio::task::spawn_blocking(move || zip.finish()).await??;
    // A client that left before the last chunk needs nothing more.
    send_chunk(sender, last, wait).await?;
    Ok(())
}

/// Rewrites the timestamps Postgres rendered (`2026-10-27T10:15:30.5+00:00`) as RFC 3339 in
/// UTC with `Z`, like every timestamp of the API (§12). Only string values of keys ending in
/// `_at` change, and only when they parse as RFC 3339; `posts.progress_at` is a number and
/// `posts.status_at` a status, so they stay as they are.
fn utc_timestamps(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(utc_timestamps),
        Value::Object(fields) => {
            for (key, field) in fields.iter_mut() {
                if key.ends_with("_at")
                    && let Value::String(text) = field
                {
                    if let Some(utc) = OffsetDateTime::parse(text, &Rfc3339)
                        .ok()
                        .and_then(rfc3339_utc)
                    {
                        *text = utc;
                    }
                } else {
                    utc_timestamps(field);
                }
            }
        }
        _ => {}
    }
}

fn rfc3339_utc(at: OffsetDateTime) -> Option<String> {
    at.to_offset(UtcOffset::UTC).format(&Rfc3339).ok()
}

/// A ZIP written as a stream: each step returns the bytes produced since the previous one, so
/// they can be sent before the next file is read. Entries carry their sizes and checksums in
/// data descriptors, which every unzip tool reads from the central directory.
struct ExportZip {
    zip: ZipWriter<StreamWriter<Spool>>,
    spool: Spool,
}

impl ExportZip {
    fn new() -> Self {
        let spool = Spool::default();
        ExportZip {
            zip: ZipWriter::new_stream(spool.clone()),
            spool,
        }
    }

    /// Adds a media file, stored as it is: WebP and JPEG compress no further.
    fn add_media(&mut self, path: &str, bytes: &[u8]) -> anyhow::Result<Bytes> {
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        self.zip.start_file(path, stored)?;
        self.zip.write_all(bytes)?;
        Ok(self.spool.take())
    }

    /// Starts `data.json` (deflated).
    fn start_data(&mut self) -> anyhow::Result<()> {
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        self.zip.start_file("data.json", deflated)?;
        Ok(())
    }

    /// Adds the next bytes of `data.json` and returns what the deflater produced so far.
    fn write_data(&mut self, bytes: &[u8]) -> anyhow::Result<Bytes> {
        self.zip.write_all(bytes)?;
        Ok(self.spool.take())
    }

    /// Ends `data.json` and adds the central directory.
    fn finish(self) -> anyhow::Result<Bytes> {
        let ExportZip { zip, spool } = self;
        zip.finish()?;
        Ok(spool.take())
    }
}

/// The bytes the ZIP writer produced that are not sent yet. Shared, because the writer owns
/// its sink.
#[derive(Clone, Default)]
struct Spool(Arc<Mutex<Vec<u8>>>);

impl Spool {
    fn take(&self) -> Bytes {
        Bytes::from(std::mem::take(&mut *self.buffer()))
    }

    fn buffer(&self) -> MutexGuard<'_, Vec<u8>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Write for Spool {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use super::*;

    #[test]
    fn timestamps_become_utc_with_z() {
        let mut value = json!({
            "created_at": "2026-10-27T12:15:30.25+02:00",
            "used_at": "2026-10-27T10:15:30+00:00",
            "rows": [{"last_used_at": "2026-01-02T03:04:05+00:00"}],
            "progress_at": 40,
            "status_at": "finished",
            "removed_at": null,
            "body": "2026-10-27T10:15:30+00:00",
        });
        utc_timestamps(&mut value);
        assert_eq!(
            value,
            json!({
                "created_at": "2026-10-27T10:15:30.25Z",
                "used_at": "2026-10-27T10:15:30Z",
                "rows": [{"last_used_at": "2026-01-02T03:04:05Z"}],
                "progress_at": 40,
                "status_at": "finished",
                "removed_at": null,
                "body": "2026-10-27T10:15:30+00:00",
            })
        );
    }

    /// Security review 2026-10-28 #6: a client that stops reading ends the export instead of
    /// holding its rows (and its export slot) until the connection dies.
    #[tokio::test]
    async fn a_client_that_stops_reading_ends_the_export() {
        let (sender, mut receiver) = mpsc::channel::<Chunk>(1);
        let wait = Duration::from_millis(50);
        assert!(
            send_chunk(&sender, Bytes::from_static(b"one"), wait)
                .await
                .unwrap()
        );
        // The channel holds one chunk; nobody takes it.
        let stalled = send_chunk(&sender, Bytes::from_static(b"two"), wait).await;
        assert!(stalled.is_err(), "{stalled:?}");
        assert_eq!(receiver.recv().await.unwrap().unwrap(), "one");
        drop(receiver);
        assert!(
            !send_chunk(&sender, Bytes::from_static(b"three"), wait)
                .await
                .unwrap()
        );
    }

    #[test]
    fn the_zip_streams_each_file_then_data_json() {
        let mut zip = ExportZip::new();
        let first = zip.add_media("media/abc/a.webp", b"RIFF....WEBP").unwrap();
        assert!(
            first.starts_with(b"PK\x03\x04"),
            "sent before the next file is read"
        );
        assert!(first.ends_with(b"RIFF....WEBP"));
        let second = zip.add_media("media/abc/s.jpg", b"\xff\xd8jpeg").unwrap();
        zip.start_data().unwrap();
        let data = serde_json::to_vec_pretty(&json!({"profile": {"username": "ada"}})).unwrap();
        let (head, tail) = data.split_at(5);
        let third = zip.write_data(head).unwrap();
        let fourth = zip.write_data(tail).unwrap();
        let last = zip.finish().unwrap();
        let bytes = [first, second, third, fourth, last].concat();

        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut names: Vec<String> = archive
            .file_names()
            .map(|name| name.unwrap().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["data.json", "media/abc/a.webp", "media/abc/s.jpg"]);
        let read = |archive: &mut zip::ZipArchive<Cursor<Vec<u8>>>, name: &str| {
            let mut bytes = Vec::new();
            archive
                .by_name(name)
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            bytes
        };
        assert_eq!(read(&mut archive, "media/abc/a.webp"), b"RIFF....WEBP");
        assert_eq!(read(&mut archive, "media/abc/s.jpg"), b"\xff\xd8jpeg");
        assert_eq!(
            serde_json::from_slice::<Value>(&read(&mut archive, "data.json")).unwrap(),
            json!({"profile": {"username": "ada"}})
        );
    }
}

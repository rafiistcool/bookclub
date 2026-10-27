//! `GET /api/auth/me/export`: the DSGVO export of the signed-in user's data (endpoint-map §2,
//! docs/phase1/conventions.md §5.7).
//!
//! A ZIP built in memory with `data.json`, one key per foreign-key column that references
//! `users(id)`, and `media/`, the bytes of every picture the user uploaded. The rows are read in
//! one repeatable-read transaction, so the sections agree with each other. Most rows are the
//! table row as Postgres renders it (`to_jsonb`) minus the reference to the user, plus the
//! names that make it readable (work titles, club names).

use std::io::{Cursor, Write};

use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::account::account_key;
use super::extract::CurrentUser;
use crate::AppState;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::tz;

/// Downloads everything the service stores about the signed-in user. Five exports an hour.
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
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.`")
    )
)]
pub async fn export_me(user: CurrentUser, State(state): State<AppState>) -> ApiResult<Response> {
    state
        .limits
        .export_user
        .check_and_hit(&account_key(user.id))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
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

    let (media_rows, files) = media_files(&state, media).await?;
    data.insert("uploaded_media".into(), media_rows);
    let mut data = Value::Object(data);
    utc_timestamps(&mut data);
    let zip = tokio::task::spawn_blocking(move || write_zip(&data, &files))
        .await
        .map_err(AppError::internal)??;

    let today = tz::today(&tz::zone_or_utc(&user.timezone))
        .format(format_description!("[year][month][day]"))
        .map_err(AppError::internal)?;
    let disposition = format!(
        "attachment; filename=\"bookclub-export-{}-{today}.zip\"",
        user.username
    );
    Ok((
        [
            (CONTENT_TYPE, "application/zip".to_owned()),
            (CONTENT_DISPOSITION, disposition),
            (CACHE_CONTROL, "no-store".to_owned()),
        ],
        zip,
    )
        .into_response())
}

/// `profile`, `sessions` and the personal data (shelf, layout, favourites, search history,
/// quotes, posts, reactions, import jobs).
async fn account_sections(
    conn: &mut PgConnection,
    user_id: i64,
    data: &mut Map<String, Value>,
) -> ApiResult<()> {
    let profile = sqlx::query_scalar!(
        r#"SELECT (to_jsonb(u) - 'password_hash')
                  || jsonb_build_object('pending_email',
                         (SELECT t.email FROM email_tokens t
                          WHERE t.user_id = u.id AND t.purpose = 'verify_email'
                            AND t.used_at IS NULL AND t.expires_at > now()
                          ORDER BY t.created_at DESC, t.id DESC LIMIT 1)) AS "profile!"
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
           WHERE m.invited_by = $1"#,
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
        r#"SELECT coalesce(jsonb_agg(to_jsonb(c) - 'created_by' ORDER BY c.id), '[]') AS "rows!"
           FROM clubs c WHERE c.created_by = $1"#,
        user_id
    )
    .fetch_one(&mut *conn)
    .await?;
    data.insert("clubs_created".into(), clubs);

    let invites = sqlx::query_scalar!(
        r#"SELECT coalesce(jsonb_agg((to_jsonb(i) - 'created_by')
                      || jsonb_build_object('club_name', c.name)
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
        r#"SELECT coalesce(jsonb_agg((to_jsonb(p) - 'set_by')
                      || jsonb_build_object('club_name', c.name, 'work_title', w.title)
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
        r#"SELECT coalesce(jsonb_agg((to_jsonb(m) - 'created_by')
                      || jsonb_build_object('club_name', c.name)
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

/// The `uploaded_media` rows and the files of `media/`: every stored variant of every picture
/// that was not taken down, as `media/<public_id>/<variant>.<webp|jpg>`.
async fn media_files(
    state: &AppState,
    rows: Vec<MediaRow>,
) -> ApiResult<(Value, Vec<(String, Bytes)>)> {
    let mut items = Vec::with_capacity(rows.len());
    let mut files = Vec::new();
    for row in rows {
        let mut paths = Vec::new();
        if let (None, Some(prefix)) = (row.removed_at, &row.storage_key) {
            let extension = match row.content_type.as_deref() {
                Some("image/jpeg") => "jpg",
                _ => "webp",
            };
            for variant in &row.variants {
                let key = format!("{prefix}/{variant}");
                match state.media.get(&key).await? {
                    Some(bytes) => {
                        let path = format!("media/{}/{variant}.{extension}", row.public_id);
                        paths.push(path.clone());
                        files.push((path, bytes));
                    }
                    None => tracing::warn!(key, "an uploaded picture has no stored bytes"),
                }
            }
        }
        items.push(json!({
            "public_id": row.public_id,
            "kind": row.kind,
            "source": row.source,
            "content_type": row.content_type,
            "width": row.width,
            "height": row.height,
            "byte_size": row.byte_size,
            "created_at": rfc3339_utc(row.created_at),
            "removed_at": row.removed_at.and_then(rfc3339_utc),
            "files": paths,
        }));
    }
    Ok((Value::Array(items), files))
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

/// `data.json` (pretty, deflated) and the media files (stored as they are: they are WebP or
/// JPEG already).
fn write_zip(data: &Value, files: &[(String, Bytes)]) -> anyhow::Result<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("data.json", deflated)?;
    zip.write_all(&serde_json::to_vec_pretty(data)?)?;
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (path, bytes) in files {
        zip.start_file(path.as_str(), stored)?;
        zip.write_all(bytes)?;
    }
    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn the_zip_holds_data_json_and_the_media_files() {
        let files = vec![(
            "media/abc/a.webp".to_owned(),
            Bytes::from_static(b"RIFF....WEBP"),
        )];
        let bytes = write_zip(&json!({"profile": {"username": "ada"}}), &files).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut names: Vec<String> = archive
            .file_names()
            .map(|name| name.unwrap().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["data.json", "media/abc/a.webp"]);
        let mut data = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("data.json").unwrap(), &mut data)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&data).unwrap(),
            json!({"profile": {"username": "ada"}})
        );
    }
}

//! Frozen media interfaces shared by every package (docs/phase1/conventions.md §8): the store,
//! keys and URLs, and the media rows that point at stored bytes.

pub mod store;
pub mod urls;

use sqlx::PgConnection;

pub use store::{FsMediaStore, MediaStore, MemMediaStore};

/// A media row for bytes the caller has already stored.
#[derive(Debug, Clone)]
pub struct NewMedia<'a> {
    /// `cover` or `avatar`.
    pub kind: &'a str,
    /// `upload`, `openlibrary` or `legacy`.
    pub source: &'a str,
    /// `image/webp` or `image/jpeg`.
    pub content_type: &'a str,
    /// The stored variants, e.g. `["s", "l"]` or `["a"]`.
    pub variants: &'a [&'a str],
    pub width: i32,
    pub height: i32,
    pub byte_size: i64,
    /// SHA-256 of the bytes as received (before any re-encoding).
    pub sha256: [u8; 32],
    /// `#rrggbb`, `""` (sampled, no usable colour) or None (not sampled yet).
    pub dominant_color: Option<String>,
    pub uploaded_by: Option<i64>,
    /// None = now(); the importer passes the legacy `avatar_updated_at`.
    pub created_at: Option<time::OffsetDateTime>,
}

/// Inserts the media row for bytes the caller has already stored under
/// `object_key(kind, public_id, variant)` for every variant. The caller generates `public_id`
/// (`urls::new_public_id`) first, because the keys contain it. Sets
/// `storage_key = storage_prefix(kind, public_id)`. Returns the row id.
pub async fn register(
    conn: &mut PgConnection,
    public_id: &str,
    m: &NewMedia<'_>,
) -> sqlx::Result<i64> {
    let variants: Vec<String> = m.variants.iter().map(|v| (*v).to_owned()).collect();
    sqlx::query_scalar!(
        "INSERT INTO media (public_id, kind, source, storage_key, content_type, variants, width,
                            height, byte_size, sha256, dominant_color, uploaded_by, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, COALESCE($13, now()))
         RETURNING id",
        public_id,
        m.kind,
        m.source,
        urls::storage_prefix(m.kind, public_id),
        m.content_type,
        &variants,
        m.width,
        m.height,
        m.byte_size,
        &m.sha256[..],
        m.dominant_color,
        m.uploaded_by,
        m.created_at
    )
    .fetch_one(conn)
    .await
}

/// True when a removed media row has this hash (uploads of a taken-down picture are refused).
pub async fn is_removed_hash(conn: &mut PgConnection, sha256: &[u8; 32]) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM media WHERE sha256 = $1 AND removed_at IS NOT NULL
           ) AS "removed!""#,
        &sha256[..]
    )
    .fetch_one(conn)
    .await
}

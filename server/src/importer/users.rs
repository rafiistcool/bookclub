//! `users` → `users`, avatar blobs → `media` (docs/phase1/legacy-mapping.md §1).

use std::io::Cursor;

use bytes::Bytes;
use sha2::{Digest, Sha256};

use super::source::LegacyUser;
use super::{ImportError, Importer};
use crate::media::shared::urls::{new_public_id, object_key, storage_prefix};
use crate::media::shared::{NewMedia, register};

/// The values the `users` CHECKs accept; anything else becomes the legacy DDL default.
const THEMES: [&str; 4] = ["paper", "slate", "forest", "ink"];
const COLOR_MODES: [&str; 3] = ["light", "dark", "system"];
const LOCALES: [&str; 2] = ["en", "de"];

/// The image types `media.content_type` accepts, sniffed from the magic bytes.
pub(crate) fn sniff_image(bytes: &[u8]) -> Option<(&'static str, image::ImageFormat)> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("image/jpeg", image::ImageFormat::Jpeg))
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(("image/webp", image::ImageFormat::WebP))
    } else {
        None
    }
}

/// The content type and size of an avatar blob, read from its header (never decoded). `None`
/// for anything `media.content_type` cannot hold or whose header is unreadable.
pub(crate) fn avatar_facts(bytes: &[u8]) -> Option<(&'static str, i32, i32)> {
    let (content_type, format) = sniff_image(bytes)?;
    let (width, height) = image::ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .ok()?;
    let width = i32::try_from(width).ok().filter(|w| *w > 0)?;
    let height = i32::try_from(height).ok().filter(|h| *h > 0)?;
    Some((content_type, width, height))
}

/// `value` when it is one of `allowed`, else `fallback` (counted as normalised).
fn pick<'a>(
    imp: &mut Importer,
    column: &str,
    value: &'a str,
    allowed: &[&'a str],
    fallback: &'a str,
) -> &'a str {
    if allowed.contains(&value) {
        value
    } else {
        imp.report.normalised(column);
        fallback
    }
}

impl Importer {
    /// Inserts every legacy user with its id, then the avatars.
    pub(crate) async fn import_users(&mut self) -> Result<(), ImportError> {
        let users = self.legacy.users.clone();
        for u in &users {
            let theme = pick(self, "users.theme", &u.theme, &THEMES, "paper").to_owned();
            let color_mode = pick(
                self,
                "users.color_mode",
                &u.color_mode,
                &COLOR_MODES,
                "system",
            )
            .to_owned();
            let locale = pick(self, "users.locale", &u.locale, &LOCALES, "en").to_owned();
            sqlx::query!(
                "INSERT INTO users (id, username, email, password_hash, password_changed_at,
                                    is_admin, theme, color_mode, locale, timezone,
                                    notify_meeting, notify_pick, notify_note, created_at)
                 VALUES ($1, $2, NULL, $3, $4, false, $5, $6, $7, $8, $9, $10, $11, $4)",
                u.id,
                u.username,
                u.password_hash,
                u.created_at,
                theme,
                color_mode,
                locale,
                self.args.timezone,
                u.notify_meeting,
                u.notify_pick,
                u.notify_note
            )
            .execute(&mut *self.tx)
            .await?;
            self.user_ids.insert(u.id);
        }
        self.setval_users().await?;
        self.report.entity(
            "users",
            Some(users.len() as u64),
            Some(users.len() as u64),
            "",
        );

        let mut imported = 0;
        let mut unusable = 0;
        let with_avatar = users.iter().filter(|u| u.avatar.is_some()).count();
        for u in &users {
            if let Some(blob) = &u.avatar {
                if self.import_avatar(u, blob).await? {
                    imported += 1;
                } else {
                    unusable += 1;
                }
            }
        }
        let mut detail = String::from("media (legacy)");
        if unusable > 0 {
            detail.push_str(&format!(
                "; {unusable} not a JPEG or WebP image, left without an avatar"
            ));
        }
        self.report
            .entity("avatars", Some(with_avatar as u64), Some(imported), detail);
        Ok(())
    }

    async fn setval_users(&mut self) -> Result<(), ImportError> {
        sqlx::query!(
            "SELECT setval(pg_get_serial_sequence('users', 'id'), (SELECT max(id) FROM users))"
        )
        .fetch_one(&mut *self.tx)
        .await?;
        Ok(())
    }

    /// Stores one avatar blob unchanged under `avatars/<pp>/<public_id>/a` (not in a dry run)
    /// and registers its media row. False when the blob is not an image the media table holds.
    async fn import_avatar(&mut self, u: &LegacyUser, blob: &[u8]) -> Result<bool, ImportError> {
        let Some((content_type, width, height)) = avatar_facts(blob) else {
            return Ok(false);
        };
        let public_id = new_public_id();
        if !self.args.dry_run {
            let key = object_key("avatar", &public_id, "a");
            self.media
                .put(&key, Bytes::copy_from_slice(blob), content_type)
                .await
                .map_err(|e| ImportError::Other(e.context(format!("cannot store {key}"))))?;
            self.written_media
                .push(storage_prefix("avatar", &public_id));
        }
        let sha256: [u8; 32] = Sha256::digest(blob).into();
        let media_id = register(
            &mut self.tx,
            &public_id,
            &NewMedia {
                kind: "avatar",
                source: "legacy",
                content_type,
                variants: &["a"],
                width,
                height,
                byte_size: blob.len() as i64,
                sha256,
                dominant_color: None,
                uploaded_by: Some(u.id),
                created_at: u.avatar_updated_at,
            },
        )
        .await?;
        sqlx::query!(
            "UPDATE users SET avatar_media_id = $2 WHERE id = $1",
            u.id,
            media_id
        )
        .execute(&mut *self.tx)
        .await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(format: image::ImageFormat) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(256, 128, image::Rgb([200, 120, 80]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    #[test]
    fn avatar_size_comes_from_the_header() {
        assert_eq!(
            avatar_facts(&encoded(image::ImageFormat::Jpeg)),
            Some(("image/jpeg", 256, 128))
        );
    }

    #[test]
    fn only_types_the_media_table_holds_are_kept() {
        assert_eq!(avatar_facts(&encoded(image::ImageFormat::Png)), None);
        assert_eq!(avatar_facts(b"GIF89a\x01\x00\x01\x00"), None);
        assert_eq!(avatar_facts(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]), None);
        assert_eq!(avatar_facts(b""), None);
    }
}

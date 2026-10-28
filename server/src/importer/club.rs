//! The imported club, its memberships (legacy-mapping.md §0, §15) and `invites` →
//! `club_invites` + `club_invite_uses` (§2).

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use super::{ImportError, Importer};
// The slug is built as club creation builds it (`POST /api/clubs`).
use crate::clubs::manage::{slug_base, slug_suffix};
use crate::text::is_py_space;

/// `clubs.slug`'s CHECK.
static SLUG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("^[a-z0-9](?:[a-z0-9-]{1,62}[a-z0-9])$").unwrap());
/// `club_invites.code`'s CHECK (besides being uppercase).
static CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[^\s-]{1,128}$").unwrap());

const SLUG_ATTEMPTS: usize = 20;

/// Whether `slug` is a valid `clubs.slug`.
pub(crate) fn valid_slug(slug: &str) -> bool {
    SLUG.is_match(slug)
}

/// A legacy invite code as `club_invites.code` stores it: whitespace and `-` removed,
/// uppercased (legacy codes already are), cut to 128 characters. `None` when nothing is left.
pub(crate) fn normalize_code(raw: &str) -> Option<String> {
    let code: String = raw
        .chars()
        .filter(|c| !is_py_space(*c) && *c != '-')
        .collect::<String>()
        .to_uppercase()
        .chars()
        .take(128)
        .collect();
    (CODE.is_match(&code) && code == code.to_uppercase()).then_some(code)
}

impl Importer {
    /// Creates the club and makes every imported user a member, `--owner` the owner. A member
    /// who redeemed an invite is `invited_by` its creator; everyone joined when their account
    /// was created.
    pub(crate) async fn import_club(&mut self) -> Result<(), ImportError> {
        let created_at = self.legacy.users.iter().map(|u| u.created_at).min();
        let club_id = match self.args.club_slug.clone() {
            Some(slug) => sqlx::query_scalar!(
                "INSERT INTO clubs (slug, name, timezone, created_by, created_at)
                 VALUES ($1, $2, $3, $4, COALESCE($5, now()))
                 ON CONFLICT (slug) DO NOTHING
                 RETURNING id",
                slug,
                self.club_name,
                self.args.timezone,
                self.owner_id,
                created_at
            )
            .fetch_optional(&mut *self.tx)
            .await?
            .ok_or_else(|| {
                ImportError::Other(anyhow::anyhow!("the club slug {slug:?} is taken"))
            })?,
            None => self.insert_club_with_fresh_slug(created_at).await?,
        };
        self.club_id = club_id;
        let slug = sqlx::query_scalar!("SELECT slug FROM clubs WHERE id = $1", club_id)
            .fetch_one(&mut *self.tx)
            .await?;

        // Who brought whom: the first invite each user redeemed.
        let mut inviter: HashMap<i64, Option<i64>> = HashMap::new();
        for invite in &self.legacy.invites {
            if let Some(user) = invite.used_by_id {
                inviter.entry(user).or_insert(invite.created_by_id);
            }
        }
        let users = self.legacy.users.clone();
        for u in &users {
            let role = if u.id == self.owner_id {
                "owner"
            } else {
                "member"
            };
            let invited_by = inviter
                .get(&u.id)
                .copied()
                .flatten()
                .filter(|id| self.user_ids.contains(id) && *id != u.id);
            sqlx::query!(
                "INSERT INTO club_memberships (club_id, user_id, role, joined_at, invited_by)
                 VALUES ($1, $2, $3, $4, $5)",
                club_id,
                u.id,
                role,
                u.created_at,
                invited_by
            )
            .execute(&mut *self.tx)
            .await?;
        }
        let owner = users
            .iter()
            .find(|u| u.id == self.owner_id)
            .map_or("", |u| u.username.as_str());
        self.report.entity(
            "club",
            None,
            Some(1),
            format!(
                "\"{}\" ({slug}), owner {owner}, {} members",
                self.club_name,
                users.len()
            ),
        );
        Ok(())
    }

    async fn insert_club_with_fresh_slug(
        &mut self,
        created_at: Option<time::OffsetDateTime>,
    ) -> Result<i64, ImportError> {
        let base = slug_base(&self.club_name);
        for _ in 0..SLUG_ATTEMPTS {
            let slug = format!("{base}-{}", slug_suffix());
            let id = sqlx::query_scalar!(
                "INSERT INTO clubs (slug, name, timezone, created_by, created_at)
                 VALUES ($1, $2, $3, $4, COALESCE($5, now()))
                 ON CONFLICT (slug) DO NOTHING
                 RETURNING id",
                slug,
                self.club_name,
                self.args.timezone,
                self.owner_id,
                created_at
            )
            .fetch_optional(&mut *self.tx)
            .await?;
            if let Some(id) = id {
                return Ok(id);
            }
        }
        Err(ImportError::Other(anyhow::anyhow!(
            "no free club slug for {base:?} after {SLUG_ATTEMPTS} attempts"
        )))
    }

    /// Imports every invite into the club, single-use and without expiry, with its id. A used
    /// invite gets its redemption; an unused one is revoked at import time.
    pub(crate) async fn import_invites(&mut self) -> Result<(), ImportError> {
        let invites = self.legacy.invites.clone();
        let created: HashMap<i64, time::OffsetDateTime> = self
            .legacy
            .users
            .iter()
            .map(|u| (u.id, u.created_at))
            .collect();
        let (mut used, mut unused) = (0, 0);
        for invite in &invites {
            let code = match normalize_code(&invite.code) {
                Some(code) if code == invite.code => code,
                Some(code) => {
                    self.report.normalised("invites.code");
                    code
                }
                None => {
                    self.report.normalised("invites.code");
                    format!("LEGACY{}", invite.id)
                }
            };
            let created_by = invite.created_by_id.filter(|id| self.user_ids.contains(id));
            let redeemer = invite.used_by_id.filter(|id| self.user_ids.contains(id));
            let is_used = invite.used_by_id.is_some();
            sqlx::query!(
                "INSERT INTO club_invites (id, club_id, code, created_by, created_at, expires_at,
                                           max_uses, use_count, revoked_at)
                 VALUES ($1, $2, $3, $4, $5, NULL, 1, $6, CASE WHEN $7 THEN NULL ELSE now() END)",
                invite.id,
                self.club_id,
                code,
                created_by,
                invite.created_at,
                i32::from(is_used),
                is_used
            )
            .execute(&mut *self.tx)
            .await?;
            if is_used {
                used += 1;
                let used_at = invite
                    .used_at
                    .or_else(|| redeemer.and_then(|id| created.get(&id).copied()))
                    .unwrap_or(invite.created_at);
                sqlx::query!(
                    "INSERT INTO club_invite_uses (invite_id, user_id, used_at) VALUES ($1, $2, $3)",
                    invite.id,
                    redeemer,
                    used_at
                )
                .execute(&mut *self.tx)
                .await?;
            } else {
                unused += 1;
            }
        }
        sqlx::query!(
            "SELECT setval(pg_get_serial_sequence('club_invites', 'id'),
                           (SELECT max(id) FROM club_invites))"
        )
        .fetch_one(&mut *self.tx)
        .await?;
        self.report.entity(
            "invites",
            Some(invites.len() as u64),
            Some(invites.len() as u64),
            format!("({used} used, {unused} unused imported as revoked)"),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_follow_club_creation() {
        assert_eq!(slug_base("Thursday Readers"), "thursday-readers");
        assert_eq!(slug_base("Bücherwürmer & Co."), "buecherwuermer-co");
        assert_eq!(slug_base("Café Noir"), "cafe-noir");
        assert_eq!(slug_base("!!!"), "club");
        assert_eq!(slug_base(&"a".repeat(80)).len(), 50);
        assert!(valid_slug(&format!(
            "{}-{}",
            slug_base("Thursday Readers"),
            slug_suffix()
        )));
        assert!(!valid_slug("Thursday"));
        assert!(!valid_slug("-x"));
    }

    #[test]
    fn invite_codes_are_stored_normalised() {
        assert_eq!(
            normalize_code("DEMOTEST2026").as_deref(),
            Some("DEMOTEST2026")
        );
        assert_eq!(normalize_code("ab-cd ef").as_deref(), Some("ABCDEF"));
        assert_eq!(normalize_code(" - ").as_deref(), None);
        assert_eq!(normalize_code(&"x".repeat(200)).map(|c| c.len()), Some(128));
    }
}

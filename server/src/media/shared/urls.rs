//! Media public ids, storage keys and URLs (docs/phase1/conventions.md §8). Every package and the
//! importer build keys and URLs through these, so they agree on the layout.

use crate::auth::random;

/// The 32 characters of a public id: lowercase letters and the digits 2–7 (RFC 4648 base32).
const PUBLIC_ID_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
const PUBLIC_ID_LEN: usize = 26;

/// A new media public id: 26 random base32 characters (130 bits), `[a-z0-9]{26}`.
pub fn new_public_id() -> String {
    let mut bytes = [0u8; PUBLIC_ID_LEN];
    random::fill(&mut bytes);
    // 256 is a multiple of 32, so masking keeps the distribution uniform.
    bytes
        .iter()
        .map(|b| PUBLIC_ID_ALPHABET[usize::from(b & 31)] as char)
        .collect()
}

/// `"covers/<pp>/<public_id>"` or `"avatars/<pp>/<public_id>"`, where `<pp>` is the first two
/// characters of the public id. `kind` is `media.kind` (`cover` or `avatar`).
pub fn storage_prefix(kind: &str, public_id: &str) -> String {
    let shard = public_id.get(..2).unwrap_or(public_id);
    format!("{kind}s/{shard}/{public_id}")
}

/// `storage_prefix(kind, public_id) + "/" + variant`.
pub fn object_key(kind: &str, public_id: &str, variant: &str) -> String {
    format!("{}/{variant}", storage_prefix(kind, public_id))
}

/// `"/media/{public_id}/{variant}"`, same-origin and immutable.
pub fn media_url(public_id: &str, variant: &str) -> String {
    format!("/media/{public_id}/{variant}")
}

/// The large (`l`) and thumbnail (`s`) cover URLs.
pub fn cover_urls(public_id: Option<&str>) -> (Option<String>, Option<String>) {
    match public_id {
        Some(id) => (Some(media_url(id, "l")), Some(media_url(id, "s"))),
        None => (None, None),
    }
}

/// The avatar URL (variant `a`).
pub fn avatar_url(public_id: Option<&str>) -> Option<String> {
    public_id.map(|id| media_url(id, "a"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn public_ids_match_the_schema_check() {
        let pattern = regex::Regex::new("^[a-z0-9]{26}$").unwrap();
        let ids: HashSet<String> = (0..1000).map(|_| new_public_id()).collect();
        assert_eq!(ids.len(), 1000);
        assert!(ids.iter().all(|id| pattern.is_match(id)));
    }

    #[test]
    fn keys_are_sharded_by_the_first_two_characters() {
        let id = "kq3x7abcdefghijklmnopqrstu";
        assert_eq!(
            storage_prefix("cover", id),
            "covers/kq/kq3x7abcdefghijklmnopqrstu"
        );
        assert_eq!(
            storage_prefix("avatar", id),
            "avatars/kq/kq3x7abcdefghijklmnopqrstu"
        );
        assert_eq!(
            object_key("cover", id, "s"),
            "covers/kq/kq3x7abcdefghijklmnopqrstu/s"
        );
    }

    #[test]
    fn urls_are_same_origin() {
        assert_eq!(media_url("abc", "l"), "/media/abc/l");
        assert_eq!(
            cover_urls(Some("abc")),
            (
                Some("/media/abc/l".to_owned()),
                Some("/media/abc/s".to_owned())
            )
        );
        assert_eq!(cover_urls(None), (None, None));
        assert_eq!(avatar_url(Some("abc")).as_deref(), Some("/media/abc/a"));
        assert_eq!(avatar_url(None), None);
    }
}

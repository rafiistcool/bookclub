//! The `{work_id}` path parameter: numeric ids, `OL…W` and legacy keys
//! (docs/phase1/conventions.md §7.1, docs/phase1/spec/catalog.md §2.1).

use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use super::books::follow_merges;

static OL_WORK_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new("^OL[0-9]+W$").unwrap());
static CLUB_WORK_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new("^BC[a-f0-9]{10}$").unwrap());
static ISBN_WORK_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("^ISBN(?:[0-9]{9}[0-9Xx]|[0-9]{13})$").unwrap());
static GOOGLE_WORK_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("^GB[A-Za-z0-9_-]{1,40}$").unwrap());

/// A reference to a work as it appears in a URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkRef {
    /// `works.id`.
    Id(i64),
    /// `works.ol_work_id`, e.g. `OL27448W`.
    OpenLibrary(String),
    /// `legacy_work_keys.legacy_key`: `/works/BC…`, `/works/ISBN…`, or `sha256:<hex>` for `GB…`.
    Legacy(String),
}

impl WorkRef {
    /// Parses a `{work_id}`. Canonicalises an ISBN key's `x` to `X`; a GB key becomes
    /// `sha256:<hex of SHA-256 of "/works/GB…">` (case-sensitive). `None` for anything else,
    /// including a numeric id that overflows i64.
    pub fn parse(raw: &str) -> Option<WorkRef> {
        if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) {
            return raw.parse().ok().map(WorkRef::Id);
        }
        if OL_WORK_ID.is_match(raw) {
            return Some(WorkRef::OpenLibrary(raw.to_owned()));
        }
        let canonical = canonical_work_id(raw);
        if CLUB_WORK_ID.is_match(&canonical)
            || ISBN_WORK_ID.is_match(&canonical)
            || GOOGLE_WORK_ID.is_match(&canonical)
        {
            return Some(WorkRef::Legacy(legacy_key(&canonical)));
        }
        None
    }
}

/// The legacy `canonical_work_id`: an `ISBN…` id gets an uppercase `X`; nothing else changes.
pub fn canonical_work_id(work_id: &str) -> String {
    if let Some(prefix) = work_id.get(..4)
        && work_id.len() > 4
        && prefix.eq_ignore_ascii_case("isbn")
    {
        let candidate = format!("ISBN{}", work_id[4..].to_uppercase());
        if ISBN_WORK_ID.is_match(&candidate) {
            return candidate;
        }
    }
    work_id.to_owned()
}

/// The `legacy_work_keys.legacy_key` for a legacy work id (the last segment of the legacy
/// `ol_work_key`): `/works/<canonical id>`, except that a Google volume id is never stored and
/// becomes `sha256:<hex>` of its `/works/GB…` key. The importer and [`WorkRef::parse`] share it.
pub fn legacy_key(work_id: &str) -> String {
    let key = format!("/works/{}", canonical_work_id(work_id));
    if GOOGLE_WORK_ID.is_match(&key["/works/".len()..]) {
        let digest = Sha256::digest(key.as_bytes());
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        format!("sha256:{hex}")
    } else {
        key
    }
}

/// The current (not merged-away) local work `r` points at, if any.
pub async fn resolve_local(conn: &mut PgConnection, r: &WorkRef) -> sqlx::Result<Option<i64>> {
    let start = match r {
        WorkRef::Id(id) => Some(*id),
        WorkRef::OpenLibrary(ol_id) => {
            sqlx::query_scalar!("SELECT id FROM works WHERE ol_work_id = $1", ol_id)
                .fetch_optional(&mut *conn)
                .await?
        }
        WorkRef::Legacy(key) => {
            sqlx::query_scalar!(
                "SELECT work_id FROM legacy_work_keys WHERE legacy_key = $1",
                key
            )
            .fetch_optional(&mut *conn)
            .await?
        }
    };
    match start {
        Some(id) => follow_merges(conn, id).await,
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha256_key(work_key: &str) -> String {
        let digest = Sha256::digest(work_key.as_bytes());
        format!(
            "sha256:{}",
            digest
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )
    }

    #[test]
    fn numeric_ids() {
        assert_eq!(WorkRef::parse("42"), Some(WorkRef::Id(42)));
        assert_eq!(WorkRef::parse("007"), Some(WorkRef::Id(7)));
        assert_eq!(
            WorkRef::parse("9223372036854775807"),
            Some(WorkRef::Id(i64::MAX))
        );
        assert_eq!(WorkRef::parse("9223372036854775808"), None);
        assert_eq!(WorkRef::parse("-1"), None);
        assert_eq!(WorkRef::parse("1.5"), None);
        assert_eq!(WorkRef::parse(""), None);
    }

    #[test]
    fn open_library_ids() {
        assert_eq!(
            WorkRef::parse("OL27448W"),
            Some(WorkRef::OpenLibrary("OL27448W".into()))
        );
        // Nothing else is case-normalised, as in the legacy app.
        assert_eq!(WorkRef::parse("ol27448w"), None);
        assert_eq!(WorkRef::parse("OL27448M"), None);
        assert_eq!(WorkRef::parse("OLW"), None);
    }

    #[test]
    fn club_and_isbn_keys_are_canonical_legacy_keys() {
        assert_eq!(
            WorkRef::parse("BCdeadbeef01"),
            Some(WorkRef::Legacy("/works/BCdeadbeef01".into()))
        );
        assert_eq!(WorkRef::parse("BCDEADBEEF01"), None);
        assert_eq!(
            WorkRef::parse("ISBN9780316769488"),
            Some(WorkRef::Legacy("/works/ISBN9780316769488".into()))
        );
        // isbn080442957x → ISBN080442957X (verified legacy behaviour).
        assert_eq!(
            WorkRef::parse("isbn080442957x"),
            Some(WorkRef::Legacy("/works/ISBN080442957X".into()))
        );
        assert_eq!(WorkRef::parse("ISBN12345"), None);
    }

    #[test]
    fn google_keys_are_hashed_case_sensitively() {
        let parsed = WorkRef::parse("GBzyTCAlFPjgYC").unwrap();
        assert_eq!(parsed, WorkRef::Legacy(sha256_key("/works/GBzyTCAlFPjgYC")));
        let WorkRef::Legacy(key) = &parsed else {
            unreachable!()
        };
        assert!(
            regex::Regex::new("^sha256:[0-9a-f]{64}$")
                .unwrap()
                .is_match(key)
        );
        assert_ne!(
            WorkRef::parse("GBzytcalfpjgyc").unwrap(),
            parsed,
            "volume ids are case-sensitive"
        );
        assert_eq!(WorkRef::parse(&format!("GB{}", "a".repeat(41))), None);
        assert_eq!(WorkRef::parse("GB"), None);
    }

    #[test]
    fn other_keys_are_not_work_refs() {
        for raw in ["foo", "works/OL1W", "/works/OL1W", "BCdeadbeef0", "OL1W "] {
            assert_eq!(WorkRef::parse(raw), None, "{raw}");
        }
    }

    #[test]
    fn legacy_keys_for_the_importer() {
        assert_eq!(legacy_key("OL5W"), "/works/OL5W");
        assert_eq!(legacy_key("isbn080442957x"), "/works/ISBN080442957X");
        assert_eq!(legacy_key("foo"), "/works/foo");
        assert_eq!(legacy_key("GBabc"), sha256_key("/works/GBabc"));
        assert_eq!(canonical_work_id("ISBNabc"), "ISBNabc");
        assert_eq!(canonical_work_id("isbn"), "isbn");
    }
}

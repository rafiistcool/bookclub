//! Legal texts and terms acceptance (#53).
//!
//! - **The texts.** The operator keeps six HTML fragments in `LEGAL_DIR`,
//!   `{imprint,privacy,terms}.{de,en}.html`, as their generator exports them, plus
//!   `manifest.json`: `{"terms_version": "2026-11-01", "updated": {"imprint": "2026-11-01", …}}`.
//!   They are read once, at startup ([`LegalTexts::configure`], from `Config::from_lookup`), so a
//!   change needs a restart. The HTML is the operator's own and is served as it is.
//! - **Serving.** `GET /api/legal/{doc}?locale=de|en` is public. `{doc}` is the fixed
//!   [`LegalDoc`] list, so no request names a file. A document the operator has in one language
//!   only is served in that one, and the answer says which.
//! - **Without `LEGAL_DIR`.** DEV_MODE serves the marked placeholders of
//!   `server/legal-placeholder/` (compiled in), without a terms version, so a sign-up needs no
//!   acceptance; production serves nothing.
//! - **Production rules.** Outside DEV_MODE, startup refuses placeholder files in `LEGAL_DIR`,
//!   and refuses `SIGNUP_MODE=open` unless all six files are present: strangers may only sign up
//!   once the imprint, the privacy policy and the terms are online.
//! - **Terms acceptance.** Whenever the operator's terms are configured, a sign-up must send the
//!   current `terms_version` ([`accepted_terms`]); `UserOut.needs_terms` asks every other account
//!   once ([`needs_terms`]); `POST /api/auth/terms` records it.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::Json;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use time::Date;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;
use crate::auth::rules;
use crate::config::{Config, SignupMode};
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiPath, ApiQuery, Validate, Validator};

/// The languages of the legal texts. Each one falls back to the other.
const LOCALES: [&str; 2] = ["de", "en"];
/// The language `GET /api/legal/{doc}` serves when the request names none (the app's default).
const DEFAULT_LOCALE: &str = "en";
/// The attribute that marks the placeholder texts; production refuses a file that carries it.
pub const PLACEHOLDER_MARKER: &str = "data-legal-placeholder";
/// The largest legal text file `LEGAL_DIR` may hold.
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MANIFEST: &str = "manifest.json";

/// A legal document. A fixed list: `{doc}` never names a file outside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LegalDoc {
    /// The imprint (Impressum, § 5 DDG).
    Imprint,
    /// The privacy policy (Datenschutzerklärung).
    Privacy,
    /// The terms of use (Nutzungsbedingungen).
    Terms,
}

impl LegalDoc {
    /// Every document, in the order `/api/config.legal` lists them.
    pub const ALL: [LegalDoc; 3] = [LegalDoc::Imprint, LegalDoc::Privacy, LegalDoc::Terms];

    /// The name in paths and file names: `imprint`, `privacy` or `terms`.
    pub fn as_str(self) -> &'static str {
        match self {
            LegalDoc::Imprint => "imprint",
            LegalDoc::Privacy => "privacy",
            LegalDoc::Terms => "terms",
        }
    }

    /// The document named `name` exactly; `None` for anything else.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|doc| doc.as_str() == name)
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// One text as it is served.
#[derive(Clone)]
struct Text {
    html: String,
    /// It carries [`PLACEHOLDER_MARKER`].
    placeholder: bool,
}

/// The legal texts of this instance, read at startup: the operator's (`LEGAL_DIR`), the DEV_MODE
/// placeholders, or none.
#[derive(Clone)]
pub struct LegalTexts {
    /// `texts[doc][locale]`, the locale in [`LOCALES`] order.
    texts: [[Option<Text>; 2]; 3],
    /// The `updated` date of each document from the manifest.
    updated: [Option<Date>; 3],
    /// The current terms version; set only while the terms are served.
    terms_version: Option<Arc<str>>,
}

impl std::fmt::Debug for LegalTexts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LegalTexts")
            .field("documents", &self.documents())
            .field("missing_files", &self.missing_files())
            .field("terms_version", &self.terms_version)
            .finish_non_exhaustive()
    }
}

/// `LEGAL_DIR/manifest.json`. Unknown keys are refused, so a misspelt key fails at startup.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    #[serde(default)]
    terms_version: Option<String>,
    #[serde(default)]
    updated: Updated,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Updated {
    #[serde(default)]
    imprint: Option<Date>,
    #[serde(default)]
    privacy: Option<Date>,
    #[serde(default)]
    terms: Option<Date>,
}

impl LegalTexts {
    /// The texts for this configuration, with the production rules applied:
    ///
    /// - `dir` set: the operator's files ([`LegalTexts::load`]);
    /// - unset: the placeholders in DEV_MODE, otherwise none.
    ///
    /// Outside DEV_MODE, a placeholder file is refused, and so is `SIGNUP_MODE=open` while any
    /// of the six files is missing. Errors name `LEGAL_DIR` or `SIGNUP_MODE` and the file.
    pub fn configure(dir: Option<&Path>, dev_mode: bool, signup_mode: SignupMode) -> Result<Self> {
        let texts = match dir {
            Some(dir) => Self::load(dir)?,
            None if dev_mode => Self::placeholders(),
            None => Self::none(),
        };
        if dev_mode {
            return Ok(texts);
        }
        if let Some(file) = texts.placeholder_files().first() {
            bail!(
                "LEGAL_DIR/{file} is a development placeholder: put the operator's own legal \
                 texts there (or set DEV_MODE=true)"
            );
        }
        let missing = texts.missing_files();
        if signup_mode == SignupMode::Open && !missing.is_empty() {
            bail!(
                "SIGNUP_MODE=open needs the legal texts outside DEV_MODE: LEGAL_DIR must hold \
                 the imprint, privacy policy and terms in German and English (missing: {})",
                missing.join(", ")
            );
        }
        Ok(texts)
    }

    /// No texts: production without `LEGAL_DIR`.
    pub fn none() -> Self {
        Self {
            texts: Default::default(),
            updated: [None; 3],
            terms_version: None,
        }
    }

    /// The DEV_MODE placeholders, compiled in from `server/legal-placeholder/`. They carry no
    /// terms version: there is nothing to accept, so sign-up and `needs_terms` behave as without
    /// terms. (`LEGAL_DIR=legal-placeholder` loads the same files with their manifest, which
    /// has a terms version, to try the acceptance in development.)
    pub fn placeholders() -> Self {
        let text = |html: &str| {
            Some(Text {
                html: html.to_owned(),
                placeholder: true,
            })
        };
        Self {
            texts: [
                [
                    text(include_str!("../legal-placeholder/imprint.de.html")),
                    text(include_str!("../legal-placeholder/imprint.en.html")),
                ],
                [
                    text(include_str!("../legal-placeholder/privacy.de.html")),
                    text(include_str!("../legal-placeholder/privacy.en.html")),
                ],
                [
                    text(include_str!("../legal-placeholder/terms.de.html")),
                    text(include_str!("../legal-placeholder/terms.en.html")),
                ],
            ],
            updated: [None; 3],
            terms_version: None,
        }
    }

    /// Reads the operator's texts from `dir`. `manifest.json` is required; each text file is
    /// optional (a missing one is served in the other language, or not at all), but one that
    /// exists must be UTF-8, not empty and at most 512 KiB. Once a terms file exists, the
    /// manifest must name the `terms_version`: 1 to 64 letters, digits, `.`, `_` or `-`.
    pub fn load(dir: &Path) -> Result<Self> {
        if !dir.is_dir() {
            bail!("LEGAL_DIR {} is not a directory", dir.display());
        }
        let manifest_path = dir.join(MANIFEST);
        let raw = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("LEGAL_DIR: cannot read {}", manifest_path.display()))?;
        let manifest: Manifest = serde_json::from_str(&raw).with_context(|| {
            format!(
                "LEGAL_DIR: {} must be {{\"terms_version\": \"…\", \"updated\": {{\"imprint\": \
                 \"YYYY-MM-DD\", \"privacy\": \"YYYY-MM-DD\", \"terms\": \"YYYY-MM-DD\"}}}}",
                manifest_path.display()
            )
        })?;
        if let Some(version) = &manifest.terms_version
            && !valid_terms_version(version)
        {
            bail!(
                "LEGAL_DIR/{MANIFEST}: terms_version must be 1 to 64 letters, digits, dots, \
                 dashes or underscores, not {version:?}"
            );
        }

        let mut texts: [[Option<Text>; 2]; 3] = Default::default();
        for doc in LegalDoc::ALL {
            for (at, locale) in LOCALES.iter().enumerate() {
                texts[doc.index()][at] = read_text(dir, &file_name(doc, locale))?;
            }
        }
        let has_terms = texts[LegalDoc::Terms.index()].iter().any(Option::is_some);
        let terms_version = match (has_terms, manifest.terms_version) {
            (true, Some(version)) => Some(Arc::from(version)),
            (true, None) => bail!(
                "LEGAL_DIR/{MANIFEST} has no terms_version, but the terms are there: sign-up \
                 records which version people accepted"
            ),
            (false, _) => None,
        };
        let updated = manifest.updated;
        Ok(Self {
            texts,
            updated: [updated.imprint, updated.privacy, updated.terms],
            terms_version,
        })
    }

    /// The documents served in at least one language, for `/api/config.legal`.
    pub fn documents(&self) -> Vec<LegalDoc> {
        LegalDoc::ALL
            .into_iter()
            .filter(|doc| self.texts[doc.index()].iter().any(Option::is_some))
            .collect()
    }

    /// The file names of the six texts that are missing.
    pub fn missing_files(&self) -> Vec<String> {
        self.files()
            .filter(|(_, text)| text.is_none())
            .map(|(name, _)| name)
            .collect()
    }

    /// The file names of the texts that are placeholders.
    pub fn placeholder_files(&self) -> Vec<String> {
        self.files()
            .filter(|(_, text)| text.is_some_and(|text| text.placeholder))
            .map(|(name, _)| name)
            .collect()
    }

    fn files(&self) -> impl Iterator<Item = (String, Option<&Text>)> {
        LegalDoc::ALL.into_iter().flat_map(move |doc| {
            LOCALES.iter().enumerate().map(move |(at, locale)| {
                (file_name(doc, locale), self.texts[doc.index()][at].as_ref())
            })
        })
    }

    /// The current terms version: set while the operator's terms are served, `None` otherwise
    /// (no terms, or the DEV_MODE placeholders).
    pub fn terms_version(&self) -> Option<&str> {
        self.terms_version.as_deref()
    }

    /// `doc` in `locale`, or in the other language when the operator has none in `locale`.
    /// Returns the language served and the HTML.
    pub fn text(&self, doc: LegalDoc, locale: &str) -> Option<(&'static str, &str)> {
        let wanted = LOCALES.iter().position(|l| *l == locale).unwrap_or(0);
        [wanted, 1 - wanted].into_iter().find_map(|at| {
            self.texts[doc.index()][at]
                .as_ref()
                .map(|text| (LOCALES[at], text.html.as_str()))
        })
    }

    /// The date the operator last changed `doc` (`manifest.json`), if given.
    pub fn updated(&self, doc: LegalDoc) -> Option<Date> {
        self.updated[doc.index()]
    }
}

fn file_name(doc: LegalDoc, locale: &str) -> String {
    format!("{}.{locale}.html", doc.as_str())
}

/// 1 to 64 of `[A-Za-z0-9._-]`, as `users_terms_version_format` requires.
fn valid_terms_version(version: &str) -> bool {
    (1..=64).contains(&version.len())
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The text file `name` in `dir`: `None` when it does not exist.
fn read_text(dir: &Path, name: &str) -> Result<Option<Text>> {
    let path = dir.join(name);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("LEGAL_DIR: cannot read {name}")),
    };
    if !metadata.is_file() {
        bail!("LEGAL_DIR/{name} is not a file");
    }
    if metadata.len() > MAX_FILE_BYTES {
        bail!("LEGAL_DIR/{name} is larger than 512 KiB");
    }
    let bytes = std::fs::read(&path).with_context(|| format!("LEGAL_DIR: cannot read {name}"))?;
    let html =
        String::from_utf8(bytes).with_context(|| format!("LEGAL_DIR/{name} is not UTF-8"))?;
    if html.trim().is_empty() {
        bail!("LEGAL_DIR/{name} is empty");
    }
    Ok(Some(Text {
        placeholder: html.contains(PLACEHOLDER_MARKER),
        html,
    }))
}

/// The legal routes: `GET /api/legal/{doc}`.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_document))
}

/// `?locale=` of `GET /api/legal/{doc}`.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LegalQuery {
    /// `de` or `en` (default `en`).
    #[serde(default)]
    pub locale: Option<String>,
}

impl Validate for LegalQuery {
    fn validate(&mut self, v: &mut Validator) {
        if let Some(locale) = &mut self.locale {
            v.check(
                rules::allowed_choice(locale, &rules::LOCALES),
                rules::LOCALE_RULE,
            );
        }
    }
}

/// A legal document as the operator provided it.
#[derive(Debug, Serialize, ToSchema)]
pub struct LegalDocumentOut {
    pub doc: LegalDoc,
    /// The language served, `de` or `en`: the requested one, or the other when the operator has
    /// the document in that language only.
    pub locale: String,
    /// True when `locale` is not the language the request asked for.
    pub fallback: bool,
    /// When the operator last changed the document (`manifest.json`), `YYYY-MM-DD`; null when
    /// not given.
    pub updated: Option<Date>,
    /// The document as an HTML fragment, exactly as the operator's file holds it.
    pub html: String,
}

/// The imprint, the privacy policy or the terms of use, in German or English. Public: these
/// pages must open signed out. The DEV_MODE placeholders are marked as such.
#[utoipa::path(
    get,
    path = "/api/legal/{doc}",
    operation_id = "legal_get_document",
    tag = "legal",
    params(
        ("doc" = LegalDoc, Path, description = "`imprint`, `privacy` or `terms`"),
        LegalQuery
    ),
    responses(
        (status = 200, body = LegalDocumentOut, description = "The document; `fallback` when it is served in the other language"),
        (status = 400, body = ErrorBody, description = "`locale must be one of: en, de`"),
        (status = 404, body = ErrorBody, description = "`Not found`: no such document, or the operator has not provided it")
    )
)]
pub async fn get_document(
    State(state): State<AppState>,
    ApiPath(doc): ApiPath<String>,
    ApiQuery(query): ApiQuery<LegalQuery>,
) -> ApiResult<Json<LegalDocumentOut>> {
    let doc = LegalDoc::parse(&doc).ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    let wanted = query.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
    let legal = &state.config.legal;
    let (locale, html) = legal
        .text(doc, wanted)
        .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    Ok(Json(LegalDocumentOut {
        doc,
        locale: locale.to_owned(),
        fallback: locale != wanted,
        updated: legal.updated(doc),
        html: html.to_owned(),
    }))
}

/// The terms version a sign-up or an acceptance records: the current one, when `given` is it.
///
/// - No terms configured (none, or the DEV_MODE placeholders): `Ok(None)`, `given` is ignored.
/// - Terms configured: `given` must be the current version, else 400 `msg::TERMS_CHANGED`
///   (missing, or a page loaded before the operator published a new version).
pub fn accepted_terms<'a>(config: &'a Config, given: Option<&str>) -> ApiResult<Option<&'a str>> {
    match config.legal.terms_version() {
        None => Ok(None),
        Some(current) if given == Some(current) => Ok(Some(current)),
        Some(_) => Err(AppError::bad_request(msg::TERMS_CHANGED)),
    }
}

tokio::task_local! {
    /// The current terms version of the configuration serving this request ([`terms_scope`]).
    static CURRENT_TERMS: Option<Arc<str>>;
}

/// Middleware (`lib.rs`): runs every request with its configuration's terms version in reach of
/// [`needs_terms`]. `auth::user_out` builds `UserOut.needs_terms` from it: its frozen signature
/// takes no configuration, and MED's avatar endpoints call it too.
pub async fn terms_scope(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let current = state.config.legal.terms_version.clone();
    CURRENT_TERMS.scope(current, next.run(request)).await
}

/// `UserOut.needs_terms` for an account that accepted `accepted` last: true while the
/// operator's terms are configured and that is not their current version (imported accounts,
/// sign-ups from before the terms, everyone after a new version). False outside a request.
pub fn needs_terms(accepted: Option<&str>) -> bool {
    CURRENT_TERMS
        .try_with(|current| current.as_deref().is_some_and(|v| accepted != Some(v)))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_FILES: [&str; 6] = [
        "imprint.de.html",
        "imprint.en.html",
        "privacy.de.html",
        "privacy.en.html",
        "terms.de.html",
        "terms.en.html",
    ];

    fn dir_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            std::fs::write(dir.path().join(name), content).unwrap();
        }
        dir
    }

    fn complete() -> tempfile::TempDir {
        let mut files: Vec<(&str, &str)> = ALL_FILES.iter().map(|f| (*f, "<p>Text</p>")).collect();
        files.push((
            MANIFEST,
            r#"{"terms_version": "2026-11-01", "updated": {"terms": "2026-11-01"}}"#,
        ));
        dir_with(&files)
    }

    fn error(result: Result<LegalTexts>) -> String {
        format!("{:#}", result.unwrap_err())
    }

    #[test]
    fn a_complete_directory_serves_everything() {
        let dir = complete();
        let texts = LegalTexts::configure(Some(dir.path()), false, SignupMode::Open).unwrap();
        assert_eq!(texts.documents(), LegalDoc::ALL);
        assert!(texts.missing_files().is_empty());
        assert_eq!(texts.terms_version(), Some("2026-11-01"));
        assert_eq!(
            texts.updated(LegalDoc::Terms),
            Some(time::macros::date!(2026 - 11 - 01))
        );
        assert_eq!(texts.updated(LegalDoc::Imprint), None);
        assert_eq!(
            texts.text(LegalDoc::Privacy, "de"),
            Some(("de", "<p>Text</p>"))
        );
    }

    #[test]
    fn a_missing_language_falls_back_to_the_other() {
        let dir = dir_with(&[
            ("imprint.de.html", "<p>Impressum</p>"),
            ("privacy.en.html", "<p>Privacy</p>"),
            (MANIFEST, "{}"),
        ]);
        let texts = LegalTexts::load(dir.path()).unwrap();
        assert_eq!(texts.documents(), [LegalDoc::Imprint, LegalDoc::Privacy]);
        assert_eq!(
            texts.text(LegalDoc::Imprint, "en"),
            Some(("de", "<p>Impressum</p>"))
        );
        assert_eq!(
            texts.text(LegalDoc::Privacy, "de"),
            Some(("en", "<p>Privacy</p>"))
        );
        assert_eq!(texts.text(LegalDoc::Terms, "en"), None);
        // No terms: no version to accept, whatever the manifest says.
        assert_eq!(texts.terms_version(), None);
        assert_eq!(texts.missing_files().len(), 4);
    }

    #[test]
    fn open_sign_up_outside_dev_mode_needs_all_six_files() {
        let dir = dir_with(&[("imprint.de.html", "<p>x</p>"), (MANIFEST, "{}")]);
        let message = error(LegalTexts::configure(
            Some(dir.path()),
            false,
            SignupMode::Open,
        ));
        assert!(message.contains("SIGNUP_MODE=open"), "{message}");
        assert!(message.contains("terms.en.html"), "{message}");
        assert!(!message.contains("imprint.de.html"), "{message}");
        // Invite mode may go live with a partial set; DEV_MODE with anything.
        assert!(LegalTexts::configure(Some(dir.path()), false, SignupMode::Invite).is_ok());
        assert!(LegalTexts::configure(Some(dir.path()), true, SignupMode::Open).is_ok());
        let message = error(LegalTexts::configure(None, false, SignupMode::Open));
        assert!(message.contains("SIGNUP_MODE=open"), "{message}");
        let none = LegalTexts::configure(None, false, SignupMode::Invite).unwrap();
        assert!(none.documents().is_empty());
        assert_eq!(none.missing_files(), ALL_FILES);
    }

    #[test]
    fn dev_mode_without_a_directory_serves_the_placeholders() {
        let texts = LegalTexts::configure(None, true, SignupMode::Open).unwrap();
        assert_eq!(texts.documents(), LegalDoc::ALL);
        assert_eq!(texts.terms_version(), None);
        assert_eq!(texts.placeholder_files(), ALL_FILES);
        let (_, html) = texts.text(LegalDoc::Terms, "de").unwrap();
        assert!(html.contains(PLACEHOLDER_MARKER));
    }

    #[test]
    fn production_refuses_the_placeholders() {
        let placeholder_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("legal-placeholder");
        let message = error(LegalTexts::configure(
            Some(&placeholder_dir),
            false,
            SignupMode::Invite,
        ));
        assert!(message.contains("placeholder"), "{message}");
        // In DEV_MODE the directory works, with the manifest's terms version.
        let texts = LegalTexts::configure(Some(&placeholder_dir), true, SignupMode::Open).unwrap();
        assert_eq!(texts.terms_version(), Some("placeholder"));
        assert!(texts.missing_files().is_empty());
    }

    #[test]
    fn the_manifest_is_checked() {
        let dir = dir_with(&[("terms.de.html", "<p>x</p>")]);
        assert!(error(LegalTexts::load(dir.path())).contains("manifest.json"));

        for (manifest, expected) in [
            ("not json", "manifest.json must be"),
            (r#"{"termsversion": "1"}"#, "manifest.json must be"),
            (
                r#"{"terms_version": "1", "updated": {"imprint": "1.11.2026"}}"#,
                "manifest.json must be",
            ),
            (r#"{"terms_version": "v 1"}"#, "terms_version must be"),
            (r#"{"terms_version": ""}"#, "terms_version must be"),
            (r#"{"updated": {}}"#, "has no terms_version"),
        ] {
            let dir = dir_with(&[("terms.de.html", "<p>x</p>"), (MANIFEST, manifest)]);
            let message = error(LegalTexts::load(dir.path()));
            assert!(message.contains(expected), "{manifest}: {message}");
        }
    }

    #[test]
    fn bad_files_are_refused() {
        let message = error(LegalTexts::load(Path::new("/definitely/not/here")));
        assert!(message.contains("LEGAL_DIR"), "{message}");

        let dir = dir_with(&[("imprint.de.html", "  \n"), (MANIFEST, "{}")]);
        assert!(error(LegalTexts::load(dir.path())).contains("imprint.de.html is empty"));

        let dir = dir_with(&[(MANIFEST, "{}")]);
        std::fs::write(dir.path().join("privacy.en.html"), [0xff, 0xfe]).unwrap();
        assert!(error(LegalTexts::load(dir.path())).contains("privacy.en.html is not UTF-8"));

        let dir = dir_with(&[(MANIFEST, "{}")]);
        std::fs::create_dir(dir.path().join("terms.en.html")).unwrap();
        assert!(error(LegalTexts::load(dir.path())).contains("terms.en.html is not a file"));

        let dir = dir_with(&[(MANIFEST, "{}")]);
        let big = "x".repeat(MAX_FILE_BYTES as usize + 1);
        std::fs::write(dir.path().join("imprint.en.html"), big).unwrap();
        assert!(error(LegalTexts::load(dir.path())).contains("larger than 512 KiB"));
    }

    #[test]
    fn documents_parse_only_by_their_exact_name() {
        assert_eq!(LegalDoc::parse("terms"), Some(LegalDoc::Terms));
        for name in ["Terms", "terms.de.html", "../manifest", "", "impressum"] {
            assert_eq!(LegalDoc::parse(name), None, "{name}");
        }
    }

    #[test]
    fn terms_versions_match_the_column_check() {
        for ok in ["1", "2026-11-01", "v1.2_final", &"x".repeat(64)] {
            assert!(valid_terms_version(ok), "{ok}");
        }
        for bad in ["", "v 1", "ä", "1/2", &"x".repeat(65)] {
            assert!(!valid_terms_version(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn needs_terms_follows_the_request_scope() {
        assert!(!needs_terms(None), "outside a request nothing is required");
        CURRENT_TERMS
            .scope(Some(Arc::from("2")), async {
                assert!(needs_terms(None));
                assert!(needs_terms(Some("1")));
                assert!(!needs_terms(Some("2")));
            })
            .await;
        CURRENT_TERMS
            .scope(None, async {
                assert!(!needs_terms(None));
                assert!(!needs_terms(Some("1")));
            })
            .await;
    }
}

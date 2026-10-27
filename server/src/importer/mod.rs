//! The legacy SQLite importer (#38, owner IMP): `bookclub-server import-legacy` and
//! `rematch-legacy` (docs/phase1/legacy-mapping.md).
//!
//! [`run`] reads the whole legacy database into memory (read-only), then writes the target in
//! **one** Postgres transaction, in foreign-key order: users → avatars → club → memberships →
//! invites → works and editions → … A failure rolls everything back; a dry run rolls back at the
//! end and writes no media bytes. Legacy ids are kept (each sequence is advanced with `setval`),
//! except work ids: legacy books map onto works through `legacy_work_keys`.

mod books;
mod club;
mod report;
mod source;
mod users;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use regex::Regex;
use sqlx::{PgPool, Postgres, Transaction};

pub use self::books::{BookTarget, MatchKind};
use self::report::ReportBuilder;
pub use self::report::{EntityLine, IsbnLine, IsbnOutcome};
pub use self::source::{LegacyDb, parse_timestamp, read_conn};
use crate::catalog::shared::lookup::CatalogLookup;
use crate::media::shared::MediaStore;
use crate::text::{char_len, py_strip};

/// Flags of `import-legacy` (legacy-mapping.md §0). Flattened into the CLI by `main.rs`, so IMP
/// adds flags here without touching it.
#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// The legacy SQLite database (opened read-only).
    #[arg(value_name = "bookclub.db")]
    pub source: PathBuf,
    /// Username that becomes the club owner; everyone else becomes a member.
    #[arg(long)]
    pub owner: String,
    /// The club's name (the legacy BOOKCLUB_NAME), e.g. "Thursday Readers".
    #[arg(long)]
    pub club_name: String,
    /// IANA zone (the legacy BOOKCLUB_TZ), for the club and every imported user.
    #[arg(long)]
    pub timezone: String,
    /// Default: the slugified club name, "-" and 4 random characters.
    #[arg(long)]
    pub club_slug: Option<String>,
    /// Run everything in one transaction, print the report, and roll back.
    #[arg(long)]
    pub dry_run: bool,
    /// No DNB or Open Library lookups; ISBN rows are kept as bare facts (rematch later).
    #[arg(long)]
    pub offline: bool,
    /// An unavailable source does not abort; those rows are kept as bare facts.
    #[arg(long)]
    pub allow_unmatched: bool,
    /// Wait before retrying a throttled lookup, instead of the host cooldown (tests set 0).
    #[arg(long, hide = true)]
    pub retry_wait_ms: Option<u64>,
}

/// Flags of `rematch-legacy`.
#[derive(Debug, Clone, clap::Args)]
pub struct RematchArgs {
    /// Resolve and report, then roll back.
    #[arg(long)]
    pub dry_run: bool,
}

/// What an import or rematch did, one line per entity. `main.rs` prints it with `Display`.
///
/// `lines` is the same for a dry run and the real run of one database (unless an upstream record
/// changed in between, or the club slug is random), so the two can be diffed; the structured
/// fields carry the same facts for tests and tools.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// The rendered report.
    pub lines: Vec<String>,
    /// Everything was rolled back.
    pub dry_run: bool,
    /// One line per entity: source rows → target rows.
    pub entities: Vec<EntityLine>,
    /// Repaired values per `table.column` (legacy-mapping.md §0 "Normalising").
    pub normalised: BTreeMap<String, u64>,
    /// Every legacy book merged into another one's work.
    pub merges: Vec<String>,
    /// Matched ISBN rows whose year or pages now differ from what the legacy API showed.
    pub differences: Vec<String>,
    /// Every legacy ISBN key and its outcome.
    pub isbns: Vec<IsbnLine>,
    /// Time spent waiting out throttled lookups.
    pub lookup_wait: Duration,
    /// Where each legacy book went (legacy book id → work, edition, match).
    pub books: HashMap<i64, BookTarget>,
}

impl Report {
    /// The entity line named `name`.
    pub fn entity(&self, name: &str) -> Option<&EntityLine> {
        self.entities.iter().find(|e| e.name == name)
    }

    /// How many values of `table.column` were repaired.
    pub fn normalised(&self, column: &str) -> u64 {
        self.normalised.get(column).copied().unwrap_or(0)
    }

    fn from_builder(b: ReportBuilder, dry_run: bool, books: HashMap<i64, BookTarget>) -> Report {
        Report {
            lines: b.lines(),
            dry_run,
            entities: b.entities,
            normalised: b.normalised,
            merges: b.merges,
            differences: b.differences,
            isbns: b.isbns,
            lookup_wait: b.lookup_wait,
            books,
        }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.dry_run {
            writeln!(
                f,
                "DRY RUN: everything below was rolled back, and no media bytes were written."
            )?;
        }
        for line in &self.lines {
            writeln!(f, "{line}")?;
        }
        Ok(())
    }
}

/// Why an import failed. `TargetNotEmpty` exits with code 2, everything else with 1.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("target database already has users")]
    TargetNotEmpty,
    #[error("legacy database: {0}")]
    Source(String),
    #[error("catalog lookup: {0}")]
    Upstream(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// The state of one import run: the transaction, the injected services, the legacy rows and
/// what has been mapped so far.
pub(crate) struct Importer {
    tx: Transaction<'static, Postgres>,
    lookup: Arc<dyn CatalogLookup>,
    media: Arc<dyn MediaStore>,
    args: Args,
    legacy: LegacyDb,
    report: ReportBuilder,
    /// The trimmed `--club-name`.
    club_name: String,
    owner_id: i64,
    club_id: i64,
    /// Imported user ids.
    user_ids: HashSet<i64>,
    /// Legacy book id → its work.
    books: HashMap<i64, BookTarget>,
    /// Storage prefixes written to the media store, removed again when the import fails.
    written_media: Vec<String>,
}

/// Imports the legacy database named in `args` into an empty target (legacy-mapping.md).
///
/// Refuses a target that has users (`TargetNotEmpty`, nothing written). With `--dry-run` the
/// transaction is rolled back and no media bytes are written; the report is the same.
pub async fn run(
    db: PgPool,
    lookup: Arc<dyn CatalogLookup>,
    media: Arc<dyn MediaStore>,
    args: Args,
) -> Result<Report, ImportError> {
    let club_name = check_args(&args)?;
    let path = args.source.clone();
    let legacy = tokio::task::spawn_blocking(move || source::read(&path))
        .await
        .map_err(|e| ImportError::Other(anyhow::anyhow!("the legacy reader failed: {e}")))??;
    let owner_id = check_users(&legacy, &args.owner)?;

    let mut tx = db.begin().await?;
    // No account may appear while the import runs (a sign-up on a live server).
    sqlx::query!("LOCK TABLE users IN EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    let has_users = sqlx::query_scalar!(r#"SELECT EXISTS (SELECT 1 FROM users) AS "any!""#)
        .fetch_one(&mut *tx)
        .await?;
    if has_users {
        return Err(ImportError::TargetNotEmpty);
    }

    let dry_run = args.dry_run;
    let mut imp = Importer {
        tx,
        lookup,
        media,
        args,
        legacy,
        report: ReportBuilder::default(),
        club_name,
        owner_id,
        club_id: 0,
        user_ids: HashSet::new(),
        books: HashMap::new(),
        written_media: Vec::new(),
    };
    let outcome = imp.import_all().await;
    let Importer {
        tx,
        media,
        report,
        books,
        written_media,
        ..
    } = imp;
    let finished = match outcome {
        Ok(()) if dry_run => tx.rollback().await.map_err(ImportError::from),
        Ok(()) => tx.commit().await.map_err(ImportError::from),
        Err(error) => {
            let _ = tx.rollback().await;
            Err(error)
        }
    };
    if let Err(error) = finished {
        remove_written(media.as_ref(), &written_media).await;
        return Err(error);
    }
    Ok(Report::from_builder(report, dry_run, books))
}

impl Importer {
    /// Every step, in foreign-key order (legacy-mapping.md §0 "Insert order").
    async fn import_all(&mut self) -> Result<(), ImportError> {
        self.import_users().await?;
        self.import_club().await?;
        self.import_invites().await?;
        self.import_books().await?;
        Ok(())
    }
}

/// Media written before a failure are removed again: nothing refers to them after the rollback.
/// (legacy-mapping.md §0 accepts orphans; removing them is cheap, and a failure is logged.)
async fn remove_written(media: &dyn MediaStore, prefixes: &[String]) {
    for prefix in prefixes {
        if let Err(error) = media.delete_prefix(prefix).await {
            tracing::warn!(%prefix, %error, "cannot remove an orphaned legacy avatar");
        }
    }
}

/// Validates the flags; returns the trimmed club name.
fn check_args(args: &Args) -> Result<String, ImportError> {
    let name = py_strip(&args.club_name).to_owned();
    if name.is_empty() || char_len(&name) > 80 {
        return Err(ImportError::Other(anyhow::anyhow!(
            "--club-name must have 1 to 80 characters"
        )));
    }
    if crate::tz::parse_zone(&args.timezone).is_err() {
        return Err(ImportError::Other(anyhow::anyhow!(
            "--timezone {:?} is not an IANA time zone (e.g. Europe/Berlin)",
            args.timezone
        )));
    }
    if let Some(slug) = &args.club_slug
        && !club::valid_slug(slug)
    {
        return Err(ImportError::Other(anyhow::anyhow!(
            "--club-slug {slug:?} must be 3 to 64 lowercase letters, digits and inner hyphens"
        )));
    }
    Ok(name)
}

static USERNAME: LazyLock<Regex> = LazyLock::new(|| Regex::new("^[a-z0-9_]{2,32}$").unwrap());

/// Checks every username against the target's rule (any other value aborts, listing the rows)
/// and returns the id of `--owner`.
fn check_users(legacy: &LegacyDb, owner: &str) -> Result<i64, ImportError> {
    let bad: Vec<String> = legacy
        .users
        .iter()
        .filter(|u| !USERNAME.is_match(&u.username))
        .map(|u| format!("users id {}: {:?}", u.id, u.username))
        .collect();
    if !bad.is_empty() {
        return Err(ImportError::Source(format!(
            "usernames outside ^[a-z0-9_]{{2,32}}$: {}",
            bad.join(", ")
        )));
    }
    let wanted = owner.trim().to_lowercase();
    legacy
        .users
        .iter()
        .find(|u| u.username == wanted)
        .map(|u| u.id)
        .ok_or_else(|| {
            ImportError::Source(format!(
                "--owner {owner:?} is not a user of the legacy database"
            ))
        })
}

/// Re-resolves `source='legacy'` editions by ISBN (legacy-mapping.md §3.3). Not implemented yet
/// (IMP part 2).
pub async fn rematch(
    _db: PgPool,
    _lookup: Arc<dyn CatalogLookup>,
    _args: RematchArgs,
) -> Result<Report, ImportError> {
    Err(ImportError::Other(anyhow::anyhow!("not implemented")))
}

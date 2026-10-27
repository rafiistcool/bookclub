//! The legacy SQLite importer (#38, owner IMP): `bookclub-server import-legacy` and
//! `rematch-legacy` (docs/phase1/legacy-mapping.md). Wave 1 stub with the frozen entry points.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use sqlx::PgPool;

use crate::catalog::shared::lookup::CatalogLookup;
use crate::media::shared::MediaStore;

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
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub lines: Vec<String>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

/// Imports the legacy database named in `args`. W1 stub.
pub async fn run(
    _db: PgPool,
    _lookup: Arc<dyn CatalogLookup>,
    _media: Arc<dyn MediaStore>,
    _args: Args,
) -> Result<Report, ImportError> {
    Err(ImportError::Other(anyhow::anyhow!("not implemented")))
}

/// Re-resolves `source='legacy'` editions by ISBN (legacy-mapping.md §3.3). W1 stub.
pub async fn rematch(
    _db: PgPool,
    _lookup: Arc<dyn CatalogLookup>,
    _args: RematchArgs,
) -> Result<Report, ImportError> {
    Err(ImportError::Other(anyhow::anyhow!("not implemented")))
}

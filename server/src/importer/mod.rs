//! The legacy SQLite importer (#38, owner IMP): `bookclub-server import-legacy` and
//! `rematch-legacy` (docs/phase1/legacy-mapping.md).
//!
//! [`run`] reads the whole legacy database into memory (read-only), then writes the target in
//! **one** Postgres transaction, in foreign-key order (legacy-mapping.md §0 "Insert order"):
//! users → avatars → club → memberships → invites → works and editions → shelf → layouts →
//! favourites → picks → milestones → posts → reactions → quotes → votes → nominations → ballots →
//! events → search history. A failure rolls everything back; a dry run rolls back at the end and
//! writes no media bytes. Legacy ids are kept (each sequence is advanced with `setval`), except
//! work ids: legacy books map onto works through `legacy_work_keys`.
//!
//! [`rematch`] (`rematch-legacy`) re-resolves the legacy bare-facts editions by ISBN later.

mod books;
mod club;
mod events;
mod layout;
mod personal;
mod picks;
mod posts;
mod rematch;
mod report;
mod shelf;
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
pub use self::rematch::rematch;
use self::report::ReportBuilder;
pub use self::report::{EntityLine, IsbnLine, IsbnOutcome};
pub use self::source::{LegacyDb, parse_timestamp, read_conn};
use crate::catalog::shared::lookup::CatalogLookup;
use crate::media::shared::MediaStore;
use crate::text::{char_len, py_strip, truncate_chars};

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
    /// IANA zone (the legacy BOOKCLUB_TZ), for the club and every imported user. An old name
    /// such as `Europe/Kiev` is stored as the current one (`Europe/Kyiv`).
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
    /// Wait before retrying a throttled lookup, instead of the host cooldown (tests set 0).
    #[arg(long, hide = true)]
    pub retry_wait_ms: Option<u64>,
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
    /// Rows skipped or changed one by one (dropped references, skipped pick notes, layouts
    /// imported as the default, `quote_added` events without their quote, …).
    pub notes: Vec<String>,
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

    fn from_rematch(b: ReportBuilder, dry_run: bool) -> Report {
        Report::from_builder(b, dry_run, HashMap::new())
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
            notes: b.notes,
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

/// The refusal of a target that already has invites (sign-up invites minted with `beta-invite`
/// before the import): their ids and codes could collide with the legacy invites, which keep
/// theirs. Exits with 1, like every error but `TargetNotEmpty`.
pub const TARGET_HAS_INVITES: &str = "target database already has invites (minted with \
     beta-invite?); import into an empty database and mint invites after import-legacy";

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
    /// Imported shelf entry ids per user (after merges).
    entries: HashMap<i64, HashSet<i64>>,
    /// Legacy shelf id → the entry it became (itself, or the winner it merged into).
    entry_of: HashMap<i64, i64>,
    /// Imported pick id → its work.
    picks: HashMap<i64, i64>,
    /// Imported milestone id → its pick.
    milestones: HashMap<i64, i64>,
    /// Ids of the imported `book_posts` rows (kept as `posts` ids).
    kept_posts: HashSet<i64>,
    /// Legacy `club_pick_posts` id → the `posts` row that stands for it (its mirror copy or the
    /// new row), for event payloads.
    pick_post_of: HashMap<i64, i64>,
    /// The imported quotes, for `quote_added` events.
    quotes: Vec<personal::QuoteFacts>,
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
    mut args: Args,
) -> Result<Report, ImportError> {
    let club_name = check_args(&mut args)?;
    let path = args.source.clone();
    let legacy = tokio::task::spawn_blocking(move || source::read(&path))
        .await
        .map_err(|e| ImportError::Other(anyhow::anyhow!("the legacy reader failed: {e}")))??;
    let owner_id = check_users(&legacy, &args.owner)?;

    let mut tx = db.begin().await?;
    // No account or invite may appear while the import runs (a sign-up or `beta-invite` on a
    // live server).
    sqlx::query!("LOCK TABLE users, club_invites IN EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    let has_users = sqlx::query_scalar!(r#"SELECT EXISTS (SELECT 1 FROM users) AS "any!""#)
        .fetch_one(&mut *tx)
        .await?;
    if has_users {
        return Err(ImportError::TargetNotEmpty);
    }
    // Invites keep their legacy ids (and codes), so any invite already there would collide with
    // one. Without users the only possible ones are `beta-invite` sign-up invites.
    let has_invites =
        sqlx::query_scalar!(r#"SELECT EXISTS (SELECT 1 FROM club_invites) AS "any!""#)
            .fetch_one(&mut *tx)
            .await?;
    if has_invites {
        return Err(ImportError::Other(anyhow::Error::msg(TARGET_HAS_INVITES)));
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
        entries: HashMap::new(),
        entry_of: HashMap::new(),
        picks: HashMap::new(),
        milestones: HashMap::new(),
        kept_posts: HashSet::new(),
        pick_post_of: HashMap::new(),
        quotes: Vec::new(),
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
        self.import_shelf().await?;
        self.import_layouts().await?;
        self.import_favorites().await?;
        self.import_picks().await?;
        self.import_milestones().await?;
        // book_posts with their ids, setval, then the new pick notes with fresh ids.
        let fates = self.import_posts().await?;
        self.import_reactions(&fates).await?;
        self.import_quotes().await?;
        // Votes without winners, nominations, the winners, ballots.
        self.import_votes().await?;
        self.import_events().await?;
        self.import_search_history().await?;
        self.report.entity(
            "push_subscriptions",
            Some(self.legacy.push_subscriptions),
            None,
            "(Phase 4)",
        );
        self.advance_sequences().await?;
        Ok(())
    }

    /// Advances the identity sequence of every table whose ids were kept and that has not had
    /// its `setval` yet (users, invites and posts have), so new rows continue after the highest
    /// imported id.
    async fn advance_sequences(&mut self) -> Result<(), ImportError> {
        sqlx::query!(
            "SELECT
                    setval(pg_get_serial_sequence('shelf_entries', 'id'), (SELECT max(id) FROM shelf_entries)) AS shelf_entries,
                    setval(pg_get_serial_sequence('club_picks', 'id'), (SELECT max(id) FROM club_picks)) AS club_picks,
                    setval(pg_get_serial_sequence('milestones', 'id'), (SELECT max(id) FROM milestones)) AS milestones,
                    setval(pg_get_serial_sequence('quotes', 'id'), (SELECT max(id) FROM quotes)) AS quotes,
                    setval(pg_get_serial_sequence('next_up_votes', 'id'), (SELECT max(id) FROM next_up_votes)) AS next_up_votes,
                    setval(pg_get_serial_sequence('next_up_nominations', 'id'), (SELECT max(id) FROM next_up_nominations)) AS next_up_nominations,
                    setval(pg_get_serial_sequence('events', 'id'), (SELECT max(id) FROM events)) AS events,
                    setval(pg_get_serial_sequence('search_history', 'id'), (SELECT max(id) FROM search_history)) AS search_history"
        )
        .fetch_one(&mut *self.tx)
        .await?;
        Ok(())
    }

    /// `value` cut to `max` characters; a cut is counted as a repair of `column`.
    fn fit(&mut self, column: &str, value: &str, max: usize) -> String {
        if char_len(value) > max {
            self.report.normalised(column);
            truncate_chars(value, max).to_owned()
        } else {
            value.to_owned()
        }
    }

    /// `value` when it lies in `range`, else NULL (counted as a repair of `column`).
    fn in_range(
        &mut self,
        column: &str,
        value: Option<i64>,
        range: std::ops::RangeInclusive<i64>,
    ) -> Option<i32> {
        match value {
            Some(v) if range.contains(&v) => i32::try_from(v).ok(),
            Some(_) => {
                self.report.normalised(column);
                None
            }
            None => None,
        }
    }

    /// Whether `user` is an imported user.
    fn has_user(&self, user: i64) -> bool {
        self.user_ids.contains(&user)
    }

    /// Notes one row that was not imported because a reference it needs is missing (the legacy
    /// schema has foreign keys, but SQLite does not enforce them by default).
    fn dangling(&mut self, table: &str, id: i64, what: &str) {
        self.report
            .notes
            .push(format!("{table} {id}: {what}; not imported"));
    }
}

/// `(1 a, 2 b)` from the non-zero counts with their labels, or `""` when every count is zero.
pub(crate) fn detail(parts: &[(u64, &str)]) -> String {
    let parts = counted(parts);
    if parts.is_empty() {
        String::new()
    } else {
        format!("({})", parts.join(", "))
    }
}

/// `(head)`, or `(head; 1 a, 2 b)` with the non-zero counts.
pub(crate) fn detail_after(head: &str, parts: &[(u64, &str)]) -> String {
    let parts = counted(parts);
    if parts.is_empty() {
        format!("({head})")
    } else {
        format!("({head}; {})", parts.join(", "))
    }
}

fn counted(parts: &[(u64, &str)]) -> Vec<String> {
    parts
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, label)| format!("{n} {label}"))
        .collect()
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

/// Validates the flags and puts the current name of `--timezone` in `args` (an old name such as
/// `Europe/Kiev` becomes `Europe/Kyiv`); returns the trimmed club name.
fn check_args(args: &mut Args) -> Result<String, ImportError> {
    let name = py_strip(&args.club_name).to_owned();
    if name.is_empty() || char_len(&name) > 80 {
        return Err(ImportError::Other(anyhow::anyhow!(
            "--club-name must have 1 to 80 characters"
        )));
    }
    match crate::tz::canonical_zone_name(&args.timezone) {
        Ok(zone) => args.timezone = zone,
        Err(_) => {
            return Err(ImportError::Other(anyhow::anyhow!(
                "--timezone {:?} is not an IANA time zone (e.g. Europe/Berlin)",
                args.timezone
            )));
        }
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

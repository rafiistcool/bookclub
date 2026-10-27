//! `book_posts` and `club_pick_posts` → `posts`, `book_post_reactions` and `post_reactions` →
//! `post_reactions` (legacy-mapping.md §9).
//!
//! Only the real pick notes become club posts: a `book_posts` root with `legacy_post_id` (the
//! mirror copy of a legacy pick note) and its replies, and an unmirrored `club_pick_posts` row
//! written after the last mirroring. Every other root, including diary entries the legacy app
//! stamped with the current pick, becomes a diary entry with its replies.

use std::collections::{HashMap, HashSet};

use time::OffsetDateTime;

use super::shelf::{parse_status, status_str};
use super::source::{LegacyPost, LegacyReaction};
use super::{ImportError, Importer, detail, detail_after};

/// The six reactions `post_reactions.emoji` accepts.
pub(crate) const EMOJI: [&str; 6] = [
    "\u{2764}\u{fe0f}",
    "\u{1f44d}",
    "\u{1f602}",
    "\u{1f62e}",
    "\u{1f525}",
    "\u{1f4da}",
];

/// What became of a legacy `club_pick_posts` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickPostFate {
    /// Its mirror copy in `book_posts` is canonical (`Some` when that copy was imported).
    Mirrored(Option<i64>),
    /// Unmirrored, but below a mirrored id: its diary copy was deleted.
    Skipped,
    /// Written after the last mirroring: a new club post with this id.
    New(i64),
    /// Not imported (a dangling reference or an empty body).
    Dropped,
}

/// One `book_posts` row as it will be written.
struct PostRow {
    id: i64,
    work_id: i64,
    author_id: Option<i64>,
    parent_id: Option<i64>,
    /// `Some(pick)` for a club post.
    pick_id: Option<i64>,
    milestone_id: Option<i64>,
    body: String,
    spoiler_upto: Option<i32>,
    progress_at: Option<i32>,
    status_at: Option<&'static str>,
    deleted_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: Option<OffsetDateTime>,
}

impl Importer {
    /// Imports `book_posts` with their ids (roots first), advances the posts sequence, then
    /// imports the new unmirrored pick notes with fresh ids above every kept one. Returns what
    /// became of each legacy pick note, for the reactions and the event payloads.
    pub(crate) async fn import_posts(&mut self) -> Result<HashMap<i64, PickPostFate>, ImportError> {
        let posts = self.legacy.posts.clone();
        let pick_posts = self.legacy.pick_posts.clone();
        let milestone_of_note: HashMap<i64, Option<i64>> =
            pick_posts.iter().map(|n| (n.id, n.milestone_id)).collect();

        // Pass 1: the rows whose book and author exist.
        let mut usable: HashMap<i64, (&LegacyPost, i64, Option<i64>)> = HashMap::new();
        let mut dropped = 0u64;
        for p in &posts {
            let Some(book) = self.books.get(&p.book_id).copied() else {
                dropped += 1;
                self.dangling(
                    "book_posts",
                    p.id,
                    &format!("book {} does not exist", p.book_id),
                );
                continue;
            };
            let author = if self.has_user(p.author_id) {
                Some(p.author_id)
            } else if p.deleted_at.is_some() {
                None
            } else {
                dropped += 1;
                self.dangling(
                    "book_posts",
                    p.id,
                    &format!("author {} does not exist", p.author_id),
                );
                continue;
            };
            usable.insert(p.id, (p, book.work_id, author));
        }

        // Pass 2: replies need a top-level parent on the same work; others become roots.
        let mut parent_of: HashMap<i64, Option<i64>> = HashMap::new();
        let mut reattached = 0u64;
        for p in &posts {
            let Some((_, work_id, _)) = usable.get(&p.id) else {
                continue;
            };
            let parent = p.parent_id.filter(|parent| {
                usable.get(parent).is_some_and(|(row, parent_work, _)| {
                    row.parent_id.is_none() && parent_work == work_id && *parent != p.id
                })
            });
            if p.parent_id.is_some() && parent.is_none() {
                reattached += 1;
                self.report.normalised("book_posts.parent_id");
                self.report.notes.push(format!(
                    "book_posts {}: its parent {} is not a top-level post on the same book; \
                     imported as a top-level post",
                    p.id,
                    p.parent_id.unwrap_or_default()
                ));
            }
            parent_of.insert(p.id, parent);
        }

        // The scope of each root: a mirror copy of a pick note is a club post of its pick.
        let mut scope_of: HashMap<i64, (Option<i64>, Option<i64>)> = HashMap::new();
        for p in &posts {
            if !matches!(parent_of.get(&p.id), Some(None)) {
                continue;
            }
            let scope = match (p.legacy_post_id, p.pick_id) {
                (Some(note), Some(pick)) if self.picks.contains_key(&pick) => {
                    let milestone = milestone_of_note
                        .get(&note)
                        .copied()
                        .flatten()
                        .filter(|m| self.milestones.get(m) == Some(&pick));
                    (Some(pick), milestone)
                }
                (Some(_), _) => {
                    self.report.notes.push(format!(
                        "book_posts {}: the copy of pick note {} names no imported pick; \
                         imported as a diary entry",
                        p.id,
                        p.legacy_post_id.unwrap_or_default()
                    ));
                    (None, None)
                }
                (None, _) => (None, None),
            };
            scope_of.insert(p.id, scope);
        }

        let mut rows = Vec::new();
        let (mut diary, mut club, mut stamped) = (0u64, 0u64, 0u64);
        for p in &posts {
            let Some((_, work_id, author)) = usable.get(&p.id).copied() else {
                continue;
            };
            let parent_id = parent_of[&p.id];
            let root = parent_id.unwrap_or(p.id);
            let (pick_id, root_milestone) = scope_of.get(&root).copied().unwrap_or((None, None));
            let milestone_id = if parent_id.is_none() {
                root_milestone
            } else {
                None
            };
            if pick_id.is_none() {
                diary += 1;
                if p.pick_id.is_some() {
                    stamped += 1;
                }
            } else {
                club += 1;
            }
            let status_at = match p.status_at.as_deref() {
                None => None,
                Some(s) => match parse_status(s) {
                    Some(status) => Some(status_str(status)),
                    None => {
                        self.report.normalised("book_posts.status_at");
                        None
                    }
                },
            };
            let progress_at = self.in_range("book_posts.progress_at", p.progress_at, 0..=100);
            let mut deleted_at = p.deleted_at;
            let mut body = String::new();
            let mut spoiler_upto = None;
            if deleted_at.is_none() {
                body = self.fit("book_posts.body", &p.body, 1000);
                spoiler_upto = self.in_range("book_posts.spoiler_upto", p.spoiler_upto, 0..=100);
                if body.is_empty() {
                    // A live post needs a body: an empty one is a tombstone.
                    self.report.normalised("book_posts.body");
                    deleted_at = Some(p.updated_at.unwrap_or(p.created_at));
                    spoiler_upto = None;
                }
            } else if !p.body.is_empty() || p.spoiler_upto.is_some() {
                self.report.normalised("book_posts.body");
            }
            rows.push(PostRow {
                id: p.id,
                work_id,
                author_id: author,
                parent_id,
                pick_id,
                milestone_id,
                body,
                spoiler_upto,
                progress_at,
                status_at,
                deleted_at,
                created_at: p.created_at,
                updated_at: p.updated_at,
            });
        }
        if stamped > 0 {
            self.report.normalised_by("book_posts.pick_id", stamped);
        }
        // Parents before their replies (posts_parent_fk is immediate).
        rows.sort_by_key(|r| (r.parent_id.is_some(), r.id));
        for r in &rows {
            let club_id = r.pick_id.map(|_| self.club_id);
            sqlx::query!(
                "INSERT INTO posts (id, work_id, author_id, parent_id, club_id, pick_id,
                                    milestone_id, visibility, body, spoiler_upto, progress_at,
                                    status_at, deleted_at, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, 'audience', $8, $9, $10, $11, $12, $13, $14)",
                r.id,
                r.work_id,
                r.author_id,
                r.parent_id,
                club_id,
                r.pick_id,
                r.milestone_id,
                r.body,
                r.spoiler_upto,
                r.progress_at,
                r.status_at,
                r.deleted_at,
                r.created_at,
                r.updated_at
            )
            .execute(&mut *self.tx)
            .await?;
            self.kept_posts.insert(r.id);
        }
        // New rows must take ids above every kept one.
        sqlx::query!(
            "SELECT setval(pg_get_serial_sequence('posts', 'id'), (SELECT max(id) FROM posts))"
        )
        .fetch_one(&mut *self.tx)
        .await?;
        self.report.entity(
            "book_posts",
            Some(posts.len() as u64),
            Some(rows.len() as u64),
            detail_after(
                &format!("{diary} diary entries, {club} club posts"),
                &[
                    (stamped, "stamped pick ids dropped from diary entries"),
                    (reattached, "replies re-attached as top-level posts"),
                    (dropped, "with a dangling reference"),
                ],
            ),
        );

        // The legacy pick notes.
        let mut fates: HashMap<i64, PickPostFate> = HashMap::new();
        let mirrored: HashSet<i64> = posts.iter().filter_map(|p| p.legacy_post_id).collect();
        let last_mirrored = mirrored.iter().max().copied();
        for p in &posts {
            if let Some(note) = p.legacy_post_id {
                let copy = self.kept_posts.contains(&p.id).then_some(p.id);
                fates.insert(note, PickPostFate::Mirrored(copy));
                if let Some(copy) = copy {
                    self.pick_post_of.insert(note, copy);
                }
            }
        }
        let mut pending: Vec<_> = pick_posts
            .iter()
            .filter(|n| !mirrored.contains(&n.id))
            .collect();
        // Python's mirror order.
        pending.sort_by_key(|n| (n.created_at, n.id));
        let (mut skipped, mut new, mut dropped_notes) = (Vec::new(), Vec::new(), 0u64);
        for n in pending {
            if last_mirrored.is_some_and(|last| n.id <= last) {
                skipped.push(n.id);
                fates.insert(n.id, PickPostFate::Skipped);
                self.report.notes.push(format!(
                    "club_pick_posts {}: not mirrored, but below the last mirrored note ({}): its \
                     diary copy was deleted; skipped",
                    n.id,
                    last_mirrored.unwrap_or_default()
                ));
                continue;
            }
            let Some(work_id) = self.picks.get(&n.pick_id).copied() else {
                dropped_notes += 1;
                fates.insert(n.id, PickPostFate::Dropped);
                self.dangling(
                    "club_pick_posts",
                    n.id,
                    &format!("pick {} does not exist", n.pick_id),
                );
                continue;
            };
            if !self.has_user(n.author_id) {
                dropped_notes += 1;
                fates.insert(n.id, PickPostFate::Dropped);
                self.dangling(
                    "club_pick_posts",
                    n.id,
                    &format!("author {} does not exist", n.author_id),
                );
                continue;
            }
            let body = self.fit("club_pick_posts.body", &n.body, 1000);
            if body.is_empty() {
                dropped_notes += 1;
                fates.insert(n.id, PickPostFate::Dropped);
                self.dangling("club_pick_posts", n.id, "the body is empty");
                continue;
            }
            let spoiler_upto =
                self.in_range("club_pick_posts.spoiler_upto", n.spoiler_upto, 0..=100);
            let milestone_id = n
                .milestone_id
                .filter(|m| self.milestones.get(m) == Some(&n.pick_id));
            let id = sqlx::query_scalar!(
                "INSERT INTO posts (work_id, author_id, club_id, pick_id, milestone_id, visibility,
                                    body, spoiler_upto, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, 'audience', $6, $7, $8, $9)
                 RETURNING id",
                work_id,
                n.author_id,
                self.club_id,
                n.pick_id,
                milestone_id,
                body,
                spoiler_upto,
                n.created_at,
                n.updated_at
            )
            .fetch_one(&mut *self.tx)
            .await?;
            new.push(n.id);
            fates.insert(n.id, PickPostFate::New(id));
            self.pick_post_of.insert(n.id, id);
        }
        let mirrored_notes = pick_posts
            .iter()
            .filter(|n| mirrored.contains(&n.id))
            .count() as u64;
        let mut parts = Vec::new();
        if mirrored_notes > 0 {
            parts.push(format!(
                "{mirrored_notes} mirrored: their book_posts copy is canonical"
            ));
        }
        if !skipped.is_empty() {
            parts.push(format!(
                "{} skipped ({}): their diary copy was deleted",
                skipped.len(),
                ids(&skipped)
            ));
        }
        if !new.is_empty() {
            parts.push(format!(
                "{} new ({}): imported as club posts with new ids above the kept ones",
                new.len(),
                ids(&new)
            ));
        }
        if dropped_notes > 0 {
            parts.push(format!("{dropped_notes} not imported"));
        }
        self.report.entity(
            "club_pick_posts",
            Some(pick_posts.len() as u64),
            Some(new.len() as u64),
            if parts.is_empty() {
                String::new()
            } else {
                format!("({})", parts.join("; "))
            },
        );
        Ok(fates)
    }

    /// Imports the diary reactions of every imported post, and the legacy reactions of the new
    /// pick notes. A mirrored note takes its reactions from its diary copy only.
    pub(crate) async fn import_reactions(
        &mut self,
        fates: &HashMap<i64, PickPostFate>,
    ) -> Result<(), ImportError> {
        let diary = self.legacy.post_reactions.clone();
        let mut twins: HashSet<(i64, i64, String)> = HashSet::new();
        let (mut written, mut dropped) = (0u64, 0u64);
        for r in &diary {
            twins.insert((r.post_id, r.user_id, r.emoji.clone()));
            if !self.kept_posts.contains(&r.post_id) {
                dropped += 1;
                self.dangling(
                    "book_post_reactions",
                    r.id,
                    &format!("post {} was not imported", r.post_id),
                );
                continue;
            }
            if self
                .insert_reaction("book_post_reactions", r, r.post_id)
                .await?
            {
                written += 1;
            } else {
                dropped += 1;
            }
        }
        self.report.entity(
            "book_post_reactions",
            Some(diary.len() as u64),
            Some(written),
            detail(&[(dropped, "dropped")]),
        );

        let legacy = self.legacy.pick_post_reactions.clone();
        let (mut copied, mut duplicates, mut untwinned, mut on_skipped, mut lost) =
            (0u64, 0u64, 0u64, 0u64, 0u64);
        for r in &legacy {
            match fates.get(&r.post_id).copied() {
                Some(PickPostFate::Mirrored(copy)) => {
                    let twin = copy
                        .is_some_and(|copy| twins.contains(&(copy, r.user_id, r.emoji.clone())));
                    if twin {
                        duplicates += 1;
                    } else {
                        untwinned += 1;
                    }
                }
                Some(PickPostFate::Skipped) => on_skipped += 1,
                Some(PickPostFate::New(post)) => {
                    if self.insert_reaction("post_reactions", r, post).await? {
                        copied += 1;
                    } else {
                        lost += 1;
                    }
                }
                Some(PickPostFate::Dropped) | None => {
                    lost += 1;
                    self.dangling(
                        "post_reactions",
                        r.id,
                        &format!("note {} was not imported", r.post_id),
                    );
                }
            }
        }
        self.report.entity(
            "post_reactions",
            Some(legacy.len() as u64),
            Some(copied),
            detail(&[
                (duplicates, "of mirrored notes already on the diary copy"),
                (
                    untwinned,
                    "of mirrored notes not on the diary copy, dropped",
                ),
                (on_skipped, "of skipped notes"),
                (lost, "dropped"),
            ]),
        );
        Ok(())
    }

    /// Inserts one reaction on `post`; false when its user does not exist or it duplicates
    /// another. An emoji outside the six aborts the import.
    async fn insert_reaction(
        &mut self,
        table: &str,
        r: &LegacyReaction,
        post: i64,
    ) -> Result<bool, ImportError> {
        if !EMOJI.contains(&r.emoji.as_str()) {
            return Err(ImportError::Source(format!(
                "{table} id {}, column emoji: {:?} is not one of the six reactions",
                r.id, r.emoji
            )));
        }
        if !self.has_user(r.user_id) {
            self.dangling(table, r.id, &format!("user {} does not exist", r.user_id));
            return Ok(false);
        }
        let inserted = sqlx::query!(
            "INSERT INTO post_reactions (post_id, user_id, emoji, created_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT DO NOTHING",
            post,
            r.user_id,
            r.emoji,
            r.created_at
        )
        .execute(&mut *self.tx)
        .await?
        .rows_affected();
        Ok(inserted == 1)
    }
}

/// `1, 2, 3`.
fn ids(ids: &[i64]) -> String {
    ids.iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

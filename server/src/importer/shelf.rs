//! `shelf` → `shelf_entries` (legacy-mapping.md §4), `physical_shelf_layouts` (§5) and
//! `user_favorites` (§6): the personal shelf, with the collisions of legacy books that became one
//! work resolved by `catalog::shared::merge_rules` (§3.4).

use std::collections::{BTreeMap, HashMap};

use super::layout::{default_layout, normalise_layout};
use super::{ImportError, Importer, detail};
use crate::catalog::shared::merge_rules::{ShelfFacts, merge_shelf_fields, shelf_winner};
use crate::dto::{ShelfStatus, Visibility};

/// A stored shelf status, or `None` for a value the target does not know.
pub(crate) fn parse_status(status: &str) -> Option<ShelfStatus> {
    match status {
        "want_to_read" => Some(ShelfStatus::WantToRead),
        "currently_reading" => Some(ShelfStatus::CurrentlyReading),
        "finished" => Some(ShelfStatus::Finished),
        "did_not_finish" => Some(ShelfStatus::DidNotFinish),
        _ => None,
    }
}

/// `shelf_entries.status` of a status.
pub(crate) fn status_str(status: ShelfStatus) -> &'static str {
    match status {
        ShelfStatus::WantToRead => "want_to_read",
        ShelfStatus::CurrentlyReading => "currently_reading",
        ShelfStatus::Finished => "finished",
        ShelfStatus::DidNotFinish => "did_not_finish",
    }
}

fn visibility_str(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Audience => "audience",
        Visibility::OnlyMe => "only_me",
    }
}

/// One legacy shelf row, normalised for its status.
struct Entry {
    user_id: i64,
    work_id: i64,
    book_id: i64,
    /// The legacy position, the sort key of the renumbering.
    position: i64,
    shelf_position: Option<i32>,
    facts: ShelfFacts,
}

impl Importer {
    /// Imports every shelf row with its id. Values a row's status does not allow are dropped
    /// (counted); rows of one user whose books became one work merge into the winner of
    /// `merge_rules::shelf_winner`; positions are renumbered 0..n-1 per (user, status).
    pub(crate) async fn import_shelf(&mut self) -> Result<(), ImportError> {
        let rows = self.legacy.shelf.clone();
        let mut groups: BTreeMap<(i64, i64), Vec<Entry>> = BTreeMap::new();
        let mut dropped = 0u64;
        for row in &rows {
            let status = parse_status(&row.status).ok_or_else(|| {
                ImportError::Source(format!(
                    "shelf id {}, column status: unknown status {:?}",
                    row.id, row.status
                ))
            })?;
            if !self.has_user(row.user_id) {
                dropped += 1;
                self.dangling(
                    "shelf",
                    row.id,
                    &format!("user {} does not exist", row.user_id),
                );
                continue;
            }
            let Some(book) = self.books.get(&row.book_id).copied() else {
                dropped += 1;
                self.dangling(
                    "shelf",
                    row.id,
                    &format!("book {} does not exist", row.book_id),
                );
                continue;
            };
            let finished = status == ShelfStatus::Finished;
            let rating = match row.rating {
                Some(r) if finished && (1..=5).contains(&r) => Some(r as i32),
                Some(_) => {
                    self.report.normalised("shelf.rating");
                    None
                }
                None => None,
            };
            let take = if row.take.is_empty() {
                String::new()
            } else if finished {
                self.fit("shelf.take", &row.take, 140)
            } else {
                self.report.normalised("shelf.take");
                String::new()
            };
            let dnf_reason = if row.dnf_reason.is_empty() {
                String::new()
            } else if status == ShelfStatus::DidNotFinish {
                self.fit("shelf.dnf_reason", &row.dnf_reason, 200)
            } else {
                self.report.normalised("shelf.dnf_reason");
                String::new()
            };
            let progress = match row.progress {
                Some(p) if status == ShelfStatus::CurrentlyReading && (0..=100).contains(&p) => {
                    Some(p as i32)
                }
                Some(_) => {
                    self.report.normalised("shelf.progress");
                    None
                }
                None => None,
            };
            let shelf_position = match row.shelf_position {
                Some(p) if (0..=i64::from(i32::MAX)).contains(&p) => Some(p as i32),
                Some(_) => {
                    self.report.normalised("shelf.shelf_position");
                    None
                }
                None => None,
            };
            groups
                .entry((row.user_id, book.work_id))
                .or_default()
                .push(Entry {
                    user_id: row.user_id,
                    work_id: book.work_id,
                    book_id: row.book_id,
                    position: row.position,
                    shelf_position,
                    facts: ShelfFacts {
                        id: row.id,
                        status,
                        updated_at: row.updated_at,
                        rating,
                        rating_visibility: Visibility::Audience,
                        take,
                        dnf_reason,
                        progress,
                        started_at: row.started_at,
                        finished_at: row.finished_at,
                        // Only ISBN keys identify an edition (§3.3).
                        edition_id: book.edition_id,
                    },
                });
        }

        let usernames: HashMap<i64, String> = self
            .legacy
            .users
            .iter()
            .map(|u| (u.id, u.username.clone()))
            .collect();
        let mut kept: Vec<Entry> = Vec::new();
        let mut merged = 0u64;
        for (_, mut entries) in groups {
            // Best first: the winner, then the losers in the order they fill its gaps.
            entries.sort_by(|a, b| shelf_winner(&b.facts, &a.facts));
            let mut entries = entries.into_iter();
            let Some(mut winner) = entries.next() else {
                continue;
            };
            for loser in entries {
                merge_shelf_fields(&mut winner.facts, &loser.facts);
                merged += 1;
                self.entry_of.insert(loser.facts.id, winner.facts.id);
                self.report.merges.push(format!(
                    "shelf {} → shelf {} ({}): books {} and {} are one work",
                    loser.facts.id,
                    winner.facts.id,
                    usernames.get(&winner.user_id).map_or("?", String::as_str),
                    loser.book_id,
                    winner.book_id
                ));
            }
            self.entry_of.insert(winner.facts.id, winner.facts.id);
            kept.push(winner);
        }

        // Dense positions per (user, status), in the legacy (position, id) order.
        kept.sort_by_key(|e| {
            (
                e.user_id,
                status_str(e.facts.status),
                e.position,
                e.facts.id,
            )
        });
        let mut column: Option<(i64, ShelfStatus)> = None;
        let mut next = 0i32;
        for e in &kept {
            if column != Some((e.user_id, e.facts.status)) {
                column = Some((e.user_id, e.facts.status));
                next = 0;
            }
            let f = &e.facts;
            sqlx::query!(
                "INSERT INTO shelf_entries (id, user_id, work_id, edition_id, status, position,
                                            shelf_position, rating, rating_visibility, take,
                                            dnf_reason, progress, updated_at, started_at,
                                            finished_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
                f.id,
                e.user_id,
                e.work_id,
                f.edition_id,
                status_str(f.status),
                next,
                e.shelf_position,
                f.rating,
                visibility_str(f.rating_visibility),
                f.take,
                f.dnf_reason,
                f.progress,
                f.updated_at,
                f.started_at,
                f.finished_at
            )
            .execute(&mut *self.tx)
            .await?;
            next += 1;
            self.entries.entry(e.user_id).or_default().insert(f.id);
        }
        self.report.entity(
            "shelf",
            Some(rows.len() as u64),
            Some(kept.len() as u64),
            detail(&[(merged, "merged"), (dropped, "with a dangling reference")]),
        );
        Ok(())
    }

    /// Imports every layout: normalised, merged entries re-mapped, foreign ids pruned; a layout
    /// that is not JSON or that the current schema refuses becomes the default (counted).
    pub(crate) async fn import_layouts(&mut self) -> Result<(), ImportError> {
        let layouts = self.legacy.layouts.clone();
        let (mut written, mut defaulted, mut pruned, mut dropped) = (0u64, 0u64, 0u64, 0u64);
        for l in &layouts {
            if !self.has_user(l.user_id) {
                dropped += 1;
                self.dangling(
                    "physical_shelf_layouts",
                    l.user_id,
                    "the user does not exist",
                );
                continue;
            }
            let merges: Vec<(i64, i64)> = self
                .legacy
                .shelf
                .iter()
                .filter(|row| row.user_id == l.user_id)
                .filter_map(|row| {
                    let winner = *self.entry_of.get(&row.id)?;
                    (winner != row.id).then_some((row.id, winner))
                })
                .collect();
            let owned = self.entries.get(&l.user_id).cloned().unwrap_or_default();
            let layout = match normalise_layout(&l.layout_json, &merges, &owned) {
                Ok(layout) => {
                    pruned += layout.pruned as u64;
                    layout.layout
                }
                Err(reason) => {
                    defaulted += 1;
                    self.report.normalised("physical_shelf_layouts.layout_json");
                    self.report.notes.push(format!(
                        "physical_shelf_layouts {}: {reason}; imported as the default layout",
                        l.user_id
                    ));
                    default_layout()
                }
            };
            sqlx::query!(
                "INSERT INTO physical_shelf_layouts (user_id, layout, updated_at)
                 VALUES ($1, $2, now())",
                l.user_id,
                layout
            )
            .execute(&mut *self.tx)
            .await?;
            written += 1;
        }
        self.report.entity(
            "physical_shelf_layouts",
            Some(layouts.len() as u64),
            Some(written),
            detail(&[
                (defaulted, "imported as the default"),
                (pruned, "entry ids pruned"),
                (dropped, "with a dangling reference"),
            ]),
        );
        Ok(())
    }

    /// Imports every favourite onto its work. Two favourites of one work keep the lower
    /// position; positions are compacted to 1..n per user (counted when they change).
    pub(crate) async fn import_favorites(&mut self) -> Result<(), ImportError> {
        let favorites = self.legacy.favorites.clone();
        let mut best: BTreeMap<(i64, i64), (i64, i64, i64)> = BTreeMap::new();
        let (mut merged, mut dropped) = (0u64, 0u64);
        for f in &favorites {
            if !self.has_user(f.user_id) {
                dropped += 1;
                self.dangling(
                    "user_favorites",
                    f.id,
                    &format!("user {} does not exist", f.user_id),
                );
                continue;
            }
            let Some(book) = self.books.get(&f.book_id).copied() else {
                dropped += 1;
                self.dangling(
                    "user_favorites",
                    f.id,
                    &format!("book {} does not exist", f.book_id),
                );
                continue;
            };
            let candidate = (f.position, f.id, f.book_id);
            match best.get_mut(&(f.user_id, book.work_id)) {
                Some(current) => {
                    merged += 1;
                    let (keep, lose) = if candidate < *current {
                        (candidate, *current)
                    } else {
                        (*current, candidate)
                    };
                    self.report.merges.push(format!(
                        "user_favorites {} → user_favorites {}: books {} and {} are one work",
                        lose.1, keep.1, lose.2, keep.2
                    ));
                    *current = keep;
                }
                None => {
                    best.insert((f.user_id, book.work_id), candidate);
                }
            }
        }
        let mut per_user: BTreeMap<i64, Vec<(i64, i64, i64)>> = BTreeMap::new();
        for ((user, work), (position, id, _)) in best {
            per_user.entry(user).or_default().push((position, id, work));
        }
        let mut written = 0u64;
        for (user, mut favorites) in per_user {
            favorites.sort();
            for (index, (position, id, work)) in favorites.into_iter().enumerate() {
                let new_position = index as i64 + 1;
                if new_position > 3 {
                    dropped += 1;
                    self.report.notes.push(format!(
                        "user_favorites {id}: user {user} already has 3 favourites; not imported"
                    ));
                    continue;
                }
                if new_position != position {
                    self.report.normalised("user_favorites.position");
                }
                sqlx::query!(
                    "INSERT INTO user_favorites (user_id, work_id, position) VALUES ($1, $2, $3)",
                    user,
                    work,
                    new_position as i32
                )
                .execute(&mut *self.tx)
                .await?;
                written += 1;
            }
        }
        self.report.entity(
            "user_favorites",
            Some(favorites.len() as u64),
            Some(written),
            detail(&[(merged, "merged"), (dropped, "dropped")]),
        );
        Ok(())
    }
}

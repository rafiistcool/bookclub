//! Club data: `club_picks` (legacy-mapping.md §7), `pick_milestones` → `milestones` (§8) and the
//! votes (§11: `next_up_votes`, `next_up_nominations`, `next_up_ballots`).

use std::collections::{BTreeMap, HashMap, HashSet};

use time::OffsetDateTime;

use super::{ImportError, Importer, detail};
use crate::text::py_strip;

/// For rows with an open end (`ended_at` NULL): which of them to end, and when. All but the open
/// row with the highest id end when the next row (by id) started, or now when there is none.
fn extra_open<T>(
    rows: &[T],
    id: impl Fn(&T) -> i64,
    started: impl Fn(&T) -> OffsetDateTime,
    ended: impl Fn(&T) -> Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> HashMap<i64, OffsetDateTime> {
    let mut sorted: Vec<&T> = rows.iter().collect();
    sorted.sort_by_key(|r| id(r));
    let open: Vec<i64> = sorted
        .iter()
        .filter(|r| ended(r).is_none())
        .map(|r| id(r))
        .collect();
    let Some(last) = open.last().copied() else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for open_id in open.into_iter().filter(|i| *i != last) {
        let next = sorted
            .iter()
            .find(|r| id(r) > open_id)
            .map_or(now, |r| started(r));
        out.insert(open_id, next);
    }
    out
}

impl Importer {
    /// Imports every pick into the club with its id. Extra open picks are ended (counted), so
    /// the club has one current pick; `reminder_sent_at` is kept.
    pub(crate) async fn import_picks(&mut self) -> Result<(), ImportError> {
        let picks = self.legacy.picks.clone();
        let now = OffsetDateTime::now_utc();
        let mut kept = Vec::new();
        let mut dropped = 0u64;
        for p in &picks {
            match self.books.get(&p.book_id) {
                Some(book) => kept.push((p.clone(), book.work_id)),
                None => {
                    dropped += 1;
                    self.dangling(
                        "club_picks",
                        p.id,
                        &format!("book {} does not exist", p.book_id),
                    );
                }
            }
        }
        let ends = extra_open(
            &kept,
            |(p, _)| p.id,
            |(p, _)| p.started_at,
            |(p, _)| p.ended_at,
            now,
        );
        for (p, work_id) in &kept {
            let ended_at = match ends.get(&p.id) {
                Some(end) => {
                    self.report.normalised("club_picks.ended_at");
                    self.report.notes.push(format!(
                        "club_picks {}: another pick is open too; ended when the next one started",
                        p.id
                    ));
                    Some(*end)
                }
                None => p.ended_at,
            };
            let set_by = p.set_by_id.filter(|u| self.has_user(*u));
            let note = self.fit("club_picks.note", &p.note, 280);
            sqlx::query!(
                "INSERT INTO club_picks (id, club_id, work_id, set_by, note, started_at, ended_at,
                                         meeting_at, reminder_sent_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                p.id,
                self.club_id,
                work_id,
                set_by,
                note,
                p.started_at,
                ended_at,
                p.meeting_at,
                p.reminder_sent_at
            )
            .execute(&mut *self.tx)
            .await?;
            self.picks.insert(p.id, *work_id);
        }
        self.report.entity(
            "club_picks",
            Some(picks.len() as u64),
            Some(kept.len() as u64),
            detail(&[
                (ends.len() as u64, "extra open picks ended"),
                (dropped, "with a dangling reference"),
            ]),
        );
        Ok(())
    }

    /// Imports every milestone with its id: chapters become a `chapter` range (none: a
    /// title-only checkpoint), a backwards range is swapped (counted), positions are renumbered
    /// 0..n-1 per pick.
    pub(crate) async fn import_milestones(&mut self) -> Result<(), ImportError> {
        let mut milestones = self.legacy.milestones.clone();
        milestones.sort_by_key(|m| (m.pick_id, m.position, m.id));
        let (mut written, mut dropped, mut swapped) = (0u64, 0u64, 0u64);
        let mut pick: Option<i64> = None;
        let mut next = 0i32;
        for m in &milestones {
            if !self.picks.contains_key(&m.pick_id) {
                dropped += 1;
                self.dangling(
                    "pick_milestones",
                    m.id,
                    &format!("pick {} does not exist", m.pick_id),
                );
                continue;
            }
            if pick != Some(m.pick_id) {
                pick = Some(m.pick_id);
                next = 0;
            }
            let mut title = self.fit("pick_milestones.title", py_strip(&m.title), 120);
            if title.is_empty() {
                self.report.normalised("pick_milestones.title");
                title = "Untitled".to_owned();
            }
            let mut from = self.in_range(
                "pick_milestones.chapter_from",
                m.chapter_from,
                0..=i64::from(i32::MAX),
            );
            let mut to = self.in_range(
                "pick_milestones.chapter_to",
                m.chapter_to,
                0..=i64::from(i32::MAX),
            );
            if let (Some(a), Some(b)) = (from, to)
                && a > b
            {
                swapped += 1;
                self.report.normalised("pick_milestones.chapter_range");
                (from, to) = (Some(b), Some(a));
            }
            let unit = (from.is_some() || to.is_some()).then_some("chapter");
            let created_by = m.created_by_id.filter(|u| self.has_user(*u));
            sqlx::query!(
                "INSERT INTO milestones (id, club_id, pick_id, title, unit, range_from, range_to,
                                         due_at, position, created_by, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                m.id,
                self.club_id,
                m.pick_id,
                title,
                unit,
                from,
                to,
                m.due_at,
                next,
                created_by,
                m.created_at
            )
            .execute(&mut *self.tx)
            .await?;
            next += 1;
            written += 1;
            self.milestones.insert(m.id, m.pick_id);
        }
        self.report.entity(
            "pick_milestones",
            Some(milestones.len() as u64),
            Some(written),
            detail(&[
                (swapped, "backwards ranges swapped"),
                (dropped, "with a dangling reference"),
            ]),
        );
        Ok(())
    }

    /// Imports the votes (without winners), their nominations (one per work: the earliest wins,
    /// §3.4), then the winners re-pointed through the merges, then the ballots.
    pub(crate) async fn import_votes(&mut self) -> Result<(), ImportError> {
        let votes = self.legacy.votes.clone();
        let now = OffsetDateTime::now_utc();
        let ends = extra_open(&votes, |v| v.id, |v| v.started_at, |v| v.ended_at, now);
        for v in &votes {
            let ended_at = match ends.get(&v.id) {
                Some(end) => {
                    self.report.normalised("next_up_votes.ended_at");
                    self.report.notes.push(format!(
                        "next_up_votes {}: another vote is open too; ended when the next one started",
                        v.id
                    ));
                    Some(*end)
                }
                None => v.ended_at,
            };
            sqlx::query!(
                "INSERT INTO next_up_votes (id, club_id, started_at, ended_at, closes_at,
                                            winner_nomination_id)
                 VALUES ($1, $2, $3, $4, $5, NULL)",
                v.id,
                self.club_id,
                v.started_at,
                ended_at,
                v.closes_at
            )
            .execute(&mut *self.tx)
            .await?;
        }
        let vote_ids: HashSet<i64> = votes.iter().map(|v| v.id).collect();

        // Nominations: per (vote, work), the earliest (created_at, id) is kept.
        let nominations = self.legacy.nominations.clone();
        let mut groups: BTreeMap<(i64, i64), Vec<&super::source::LegacyNomination>> =
            BTreeMap::new();
        let mut dropped_nominations = 0u64;
        for n in &nominations {
            if !vote_ids.contains(&n.vote_id) {
                dropped_nominations += 1;
                self.dangling(
                    "next_up_nominations",
                    n.id,
                    &format!("vote {} does not exist", n.vote_id),
                );
                continue;
            }
            let Some(book) = self.books.get(&n.book_id).copied() else {
                dropped_nominations += 1;
                self.dangling(
                    "next_up_nominations",
                    n.id,
                    &format!("book {} does not exist", n.book_id),
                );
                continue;
            };
            groups.entry((n.vote_id, book.work_id)).or_default().push(n);
        }
        // Legacy nomination id → the nomination it became, and that one's vote.
        let mut nomination_of: HashMap<i64, i64> = HashMap::new();
        let mut vote_of: HashMap<i64, i64> = HashMap::new();
        let mut merged = 0u64;
        let mut kept = Vec::new();
        for ((vote_id, work_id), mut group) in groups {
            group.sort_by_key(|n| (n.created_at, n.id));
            let winner = group[0];
            for loser in &group[1..] {
                merged += 1;
                nomination_of.insert(loser.id, winner.id);
                self.report.merges.push(format!(
                    "next_up_nominations {} → next_up_nominations {} (vote {vote_id}): books {} and {} are one work",
                    loser.id, winner.id, loser.book_id, winner.book_id
                ));
            }
            nomination_of.insert(winner.id, winner.id);
            vote_of.insert(winner.id, vote_id);
            kept.push((winner.clone(), work_id));
        }
        kept.sort_by_key(|(n, _)| n.id);
        for (n, work_id) in &kept {
            let nominated_by = n.nominated_by_id.filter(|u| self.has_user(*u));
            sqlx::query!(
                "INSERT INTO next_up_nominations (id, club_id, vote_id, work_id, nominated_by,
                                                  created_at)
                 VALUES ($1, $2, $3, $4, $5, $6)",
                n.id,
                self.club_id,
                n.vote_id,
                work_id,
                nominated_by,
                n.created_at
            )
            .execute(&mut *self.tx)
            .await?;
        }

        // The winners, now that their nominations exist.
        let mut lost_winners = 0u64;
        for v in &votes {
            let Some(legacy_winner) = v.winner_nomination_id else {
                continue;
            };
            let winner = nomination_of
                .get(&legacy_winner)
                .copied()
                .filter(|n| vote_of.get(n) == Some(&v.id));
            match winner {
                Some(nomination) => {
                    sqlx::query!(
                        "UPDATE next_up_votes SET winner_nomination_id = $2 WHERE id = $1",
                        v.id,
                        nomination
                    )
                    .execute(&mut *self.tx)
                    .await?;
                }
                None => {
                    lost_winners += 1;
                    self.report.normalised("next_up_votes.winner_nomination_id");
                    self.report.notes.push(format!(
                        "next_up_votes {}: the winner {legacy_winner} is not a nomination of this \
                         vote; imported without a winner",
                        v.id
                    ));
                }
            }
        }

        let ballots = self.legacy.ballots.clone();
        let (mut ballots_written, mut ballots_dropped) = (0u64, 0u64);
        for b in &ballots {
            let nomination = nomination_of
                .get(&b.nomination_id)
                .copied()
                .filter(|n| vote_of.get(n) == Some(&b.vote_id));
            let Some(nomination) = nomination else {
                ballots_dropped += 1;
                self.dangling(
                    "next_up_ballots",
                    b.id,
                    &format!(
                        "nomination {} is not one of vote {}",
                        b.nomination_id, b.vote_id
                    ),
                );
                continue;
            };
            if !self.has_user(b.user_id) {
                ballots_dropped += 1;
                self.dangling(
                    "next_up_ballots",
                    b.id,
                    &format!("user {} does not exist", b.user_id),
                );
                continue;
            }
            let moved = nomination != b.nomination_id;
            let inserted = sqlx::query!(
                "INSERT INTO next_up_ballots (club_id, vote_id, user_id, nomination_id, created_at)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT DO NOTHING",
                self.club_id,
                b.vote_id,
                b.user_id,
                nomination,
                b.created_at
            )
            .execute(&mut *self.tx)
            .await?
            .rows_affected();
            if inserted == 1 {
                ballots_written += 1;
                if moved {
                    self.report.notes.push(format!(
                        "next_up_ballots {}: moved to nomination {nomination} with its merge",
                        b.id
                    ));
                }
            } else {
                ballots_dropped += 1;
                self.report.notes.push(format!(
                    "next_up_ballots {}: user {} already has a ballot in vote {}; not imported",
                    b.id, b.user_id, b.vote_id
                ));
            }
        }

        self.report.entity(
            "next_up_votes",
            Some(votes.len() as u64),
            Some(votes.len() as u64),
            detail(&[
                (ends.len() as u64, "extra open votes ended"),
                (lost_winners, "winners not a nomination of their vote"),
            ]),
        );
        self.report.entity(
            "next_up_nominations",
            Some(nominations.len() as u64),
            Some(kept.len() as u64),
            detail(&[
                (merged, "merged"),
                (dropped_nominations, "with a dangling reference"),
            ]),
        );
        self.report.entity(
            "next_up_ballots",
            Some(ballots.len() as u64),
            Some(ballots_written),
            detail(&[(ballots_dropped, "dropped")]),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    #[test]
    fn extra_open_rows_end_when_the_next_one_started() {
        let now = datetime!(2026-10-01 0:00 UTC);
        let rows = [
            (1, datetime!(2026-01-01 0:00 UTC), None),
            (
                2,
                datetime!(2026-02-01 0:00 UTC),
                Some(datetime!(2026-02-10 0:00 UTC)),
            ),
            (3, datetime!(2026-03-01 0:00 UTC), None),
            (4, datetime!(2026-04-01 0:00 UTC), None),
        ];
        let ends = extra_open(&rows, |r| r.0, |r| r.1, |r| r.2, now);
        assert_eq!(ends.len(), 2);
        assert_eq!(ends[&1], datetime!(2026-02-01 0:00 UTC));
        assert_eq!(ends[&3], datetime!(2026-04-01 0:00 UTC));
        assert!(!ends.contains_key(&4));
        assert!(extra_open(&rows[..2], |r| r.0, |r| r.1, |r| r.2, now).is_empty());
    }
}

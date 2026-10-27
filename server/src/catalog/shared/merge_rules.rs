//! The collision rules for two works becoming one, used by the importer and `merge_works`
//! (docs/phase1/conventions.md §7.6, docs/phase1/legacy-mapping.md §3.4). Pure functions.

use std::cmp::Ordering;

use serde_json::Value;
use time::OffsetDateTime;

use crate::dto::{ShelfStatus, Visibility};

/// The fields of a shelf entry the merge rules look at and change.
#[derive(Debug, Clone, PartialEq)]
pub struct ShelfFacts {
    pub id: i64,
    pub status: ShelfStatus,
    pub updated_at: OffsetDateTime,
    pub rating: Option<i32>,
    /// Hides `rating` and `take`; a copied rating or take never becomes more visible.
    pub rating_visibility: Visibility,
    pub take: String,
    pub dnf_reason: String,
    pub progress: Option<i32>,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
    pub edition_id: Option<i64>,
}

/// finished 4 > did_not_finish 3 > currently_reading 2 > want_to_read 1.
pub fn shelf_status_rank(s: ShelfStatus) -> u8 {
    match s {
        ShelfStatus::Finished => 4,
        ShelfStatus::DidNotFinish => 3,
        ShelfStatus::CurrentlyReading => 2,
        ShelfStatus::WantToRead => 1,
    }
}

/// Which entry survives: higher status rank, then newer `updated_at`, then lower id.
/// `Greater` means `a` wins, so `entries.iter().max_by(|a, b| shelf_winner(a, b))` is the winner.
pub fn shelf_winner(a: &ShelfFacts, b: &ShelfFacts) -> Ordering {
    shelf_status_rank(a.status)
        .cmp(&shelf_status_rank(b.status))
        .then(a.updated_at.cmp(&b.updated_at))
        .then(b.id.cmp(&a.id))
}

/// Copies the loser's rating, take, dnf_reason, progress, started_at, finished_at, edition_id into
/// the winner where the winner's value is NULL/'' and the loser's value is allowed for the
/// winner's status (the shelf_entries CHECKs and the reading-date rules). A copied rating or take
/// makes the winner's `rating_visibility` `only_me` if either entry had it; a copied start never
/// lies after the finish.
pub fn merge_shelf_fields(winner: &mut ShelfFacts, loser: &ShelfFacts) {
    let status = winner.status;
    let finished = status == ShelfStatus::Finished;
    let mut copied_private = false;
    if finished && winner.rating.is_none() && loser.rating.is_some() {
        winner.rating = loser.rating;
        copied_private = true;
    }
    if finished && winner.take.is_empty() && !loser.take.is_empty() {
        winner.take = loser.take.clone();
        copied_private = true;
    }
    if copied_private && loser.rating_visibility == Visibility::OnlyMe {
        winner.rating_visibility = Visibility::OnlyMe;
    }
    if status == ShelfStatus::DidNotFinish && winner.dnf_reason.is_empty() {
        winner.dnf_reason = loser.dnf_reason.clone();
    }
    if status == ShelfStatus::CurrentlyReading && winner.progress.is_none() {
        winner.progress = loser.progress;
    }
    if status != ShelfStatus::WantToRead && winner.started_at.is_none() {
        winner.started_at = loser.started_at;
    }
    if finished && winner.finished_at.is_none() {
        winner.finished_at = loser.finished_at;
    }
    if winner.edition_id.is_none() {
        winner.edition_id = loser.edition_id;
    }
    if let (Some(start), Some(finish)) = (winner.started_at, winner.finished_at)
        && start > finish
    {
        winner.started_at = Some(finish);
    }
}

fn is_entry(item: &Value, id: i64) -> bool {
    item.as_i64() == Some(id)
}

/// Replaces the loser entry id by the winner's in a physical-shelf layout (`rows` items and
/// `spines` keys), dropping the loser where the winner is already placed or already has a
/// spine. Anything that is not the documented shape is left alone.
pub fn remap_layout(layout: &mut Value, loser_entry: i64, winner_entry: i64) {
    if loser_entry == winner_entry {
        return;
    }
    if let Some(rows) = layout.get_mut("rows").and_then(Value::as_array_mut) {
        let placed = rows
            .iter()
            .filter_map(Value::as_array)
            .flatten()
            .any(|item| is_entry(item, winner_entry));
        for row in rows.iter_mut().filter_map(Value::as_array_mut) {
            if placed {
                row.retain(|item| !is_entry(item, loser_entry));
            } else {
                for item in row.iter_mut().filter(|item| is_entry(item, loser_entry)) {
                    *item = Value::from(winner_entry);
                }
            }
        }
    }
    if let Some(spines) = layout.get_mut("spines").and_then(Value::as_object_mut)
        && let Some(spine) = spines.remove(&loser_entry.to_string())
    {
        spines.entry(winner_entry.to_string()).or_insert(spine);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use time::macros::datetime;

    use super::*;

    fn entry(id: i64, status: ShelfStatus) -> ShelfFacts {
        ShelfFacts {
            id,
            status,
            updated_at: datetime!(2025-01-01 12:00 UTC),
            rating: None,
            rating_visibility: Visibility::Audience,
            take: String::new(),
            dnf_reason: String::new(),
            progress: None,
            started_at: None,
            finished_at: None,
            edition_id: None,
        }
    }

    #[test]
    fn status_ranks() {
        use ShelfStatus::*;
        let ranks: Vec<u8> = [Finished, DidNotFinish, CurrentlyReading, WantToRead]
            .into_iter()
            .map(shelf_status_rank)
            .collect();
        assert_eq!(ranks, [4, 3, 2, 1]);
    }

    #[test]
    fn winner_by_status_then_recency_then_id() {
        let finished = entry(5, ShelfStatus::Finished);
        let reading = ShelfFacts {
            updated_at: datetime!(2026-01-01 0:00 UTC),
            ..entry(1, ShelfStatus::CurrentlyReading)
        };
        assert_eq!(shelf_winner(&finished, &reading), Ordering::Greater);
        assert_eq!(shelf_winner(&reading, &finished), Ordering::Less);

        let newer = ShelfFacts {
            updated_at: datetime!(2025-06-01 0:00 UTC),
            ..entry(9, ShelfStatus::Finished)
        };
        assert_eq!(shelf_winner(&newer, &finished), Ordering::Greater);

        let twin = entry(3, ShelfStatus::Finished);
        assert_eq!(
            shelf_winner(&twin, &finished),
            Ordering::Greater,
            "lower id wins"
        );
        let all = [finished.clone(), reading, newer.clone(), twin];
        assert_eq!(all.iter().max_by(|a, b| shelf_winner(a, b)).unwrap().id, 9);
    }

    #[test]
    fn merge_copies_only_what_the_winner_status_allows() {
        let mut winner = entry(1, ShelfStatus::Finished);
        let loser = ShelfFacts {
            rating: Some(4),
            take: "Loved it".into(),
            dnf_reason: "boring".into(),
            progress: Some(40),
            started_at: Some(datetime!(2024-01-01 0:00 UTC)),
            finished_at: Some(datetime!(2024-02-01 0:00 UTC)),
            edition_id: Some(77),
            ..entry(2, ShelfStatus::CurrentlyReading)
        };
        merge_shelf_fields(&mut winner, &loser);
        assert_eq!(winner.rating, Some(4));
        assert_eq!(winner.take, "Loved it");
        assert_eq!(winner.dnf_reason, "", "only for did_not_finish");
        assert_eq!(winner.progress, None, "only for currently_reading");
        assert_eq!(winner.started_at, loser.started_at);
        assert_eq!(winner.finished_at, loser.finished_at);
        assert_eq!(winner.edition_id, Some(77));

        let mut want = entry(3, ShelfStatus::WantToRead);
        merge_shelf_fields(&mut want, &loser);
        assert_eq!(
            (
                want.rating,
                want.started_at,
                want.finished_at,
                want.edition_id
            ),
            (None, None, None, Some(77))
        );

        let mut dnf = entry(4, ShelfStatus::DidNotFinish);
        merge_shelf_fields(&mut dnf, &loser);
        assert_eq!(dnf.dnf_reason, "boring");
        assert_eq!(dnf.finished_at, None);
        assert_eq!(dnf.rating, None);

        let mut reading = entry(5, ShelfStatus::CurrentlyReading);
        merge_shelf_fields(&mut reading, &loser);
        assert_eq!(reading.progress, Some(40));
    }

    #[test]
    fn merge_keeps_the_winner_values_and_privacy() {
        let mut winner = ShelfFacts {
            rating: Some(2),
            take: "Meh".into(),
            edition_id: Some(1),
            ..entry(1, ShelfStatus::Finished)
        };
        let loser = ShelfFacts {
            rating: Some(5),
            take: "Wow".into(),
            rating_visibility: Visibility::OnlyMe,
            edition_id: Some(2),
            ..entry(2, ShelfStatus::Finished)
        };
        merge_shelf_fields(&mut winner, &loser);
        assert_eq!((winner.rating, winner.take.as_str()), (Some(2), "Meh"));
        assert_eq!(winner.edition_id, Some(1));
        assert_eq!(
            winner.rating_visibility,
            Visibility::Audience,
            "nothing copied"
        );

        // A private rating copied into a public entry stays private.
        let mut winner = ShelfFacts {
            take: "Public take".into(),
            ..entry(1, ShelfStatus::Finished)
        };
        merge_shelf_fields(&mut winner, &loser);
        assert_eq!(winner.rating, Some(5));
        assert_eq!(winner.rating_visibility, Visibility::OnlyMe);
    }

    #[test]
    fn a_copied_start_never_follows_the_finish() {
        let mut winner = ShelfFacts {
            finished_at: Some(datetime!(2024-01-10 0:00 UTC)),
            ..entry(1, ShelfStatus::Finished)
        };
        let loser = ShelfFacts {
            started_at: Some(datetime!(2024-03-01 0:00 UTC)),
            ..entry(2, ShelfStatus::CurrentlyReading)
        };
        merge_shelf_fields(&mut winner, &loser);
        assert_eq!(winner.started_at, winner.finished_at);
    }

    #[test]
    fn layouts_point_at_the_winner() {
        let mut layout = json!({
            "rows": [[7, "decor:plant"], [3]],
            "material": "oak",
            "spines": {"7": {"color": "#ab3c45"}, "3": {"color": "#000000"}},
            "decorations": {"decor:plant": {"kind": "plant"}}
        });
        remap_layout(&mut layout, 7, 9);
        assert_eq!(layout["rows"], json!([[9, "decor:plant"], [3]]));
        assert_eq!(
            layout["spines"],
            json!({"9": {"color": "#ab3c45"}, "3": {"color": "#000000"}})
        );

        // The winner is already placed and has a spine: the loser disappears.
        remap_layout(&mut layout, 3, 9);
        assert_eq!(layout["rows"], json!([[9, "decor:plant"], []]));
        assert_eq!(layout["spines"], json!({"9": {"color": "#ab3c45"}}));
        assert_eq!(
            layout["decorations"],
            json!({"decor:plant": {"kind": "plant"}})
        );

        // Unknown shapes are left alone.
        let mut odd = json!({"rows": "nope"});
        remap_layout(&mut odd, 1, 2);
        assert_eq!(odd, json!({"rows": "nope"}));
    }
}

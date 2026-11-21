//! The per-status rules and the two shelf orders (docs/phase1/spec/shelf.md §2.3–§2.5), as pure
//! functions over the values they change. `entries` loads the rows, applies these rules and
//! writes the result back in one transaction.
//!
//! - [`apply_finish_fields`], [`apply_progress`] and [`apply_reading_dates`] port
//!   `shelf_ops.py` field by field. [`apply_page`] is new: progress in pages
//!   (docs/phase3/notes/reading-trust.md).
//! - [`column_order`] is the `position` half of `place_item`: the order of one (user, status)
//!   column after the item is inserted.
//! - [`shelf_order`] is `place_on_shelf`: the free order across every status.
//! - [`restore_reading_dates`] and [`shelf_index`] are new: the Undo of a removal puts the
//!   entry back with its dates and at its place in the free order.

use time::OffsetDateTime;

use crate::dto::ShelfStatus;
use crate::text::py_strip;

/// The fields of a shelf entry that a status change rewrites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusFields {
    pub rating: Option<i32>,
    pub take: String,
    pub dnf_reason: String,
    pub progress: Option<i32>,
    /// The page the reader is on; only with a known page count, and `progress` is derived from it.
    pub current_page: Option<i32>,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
}

impl StatusFields {
    /// A new entry: no rating, take, reason, progress, page or dates.
    pub fn empty() -> Self {
        Self {
            rating: None,
            take: String::new(),
            dnf_reason: String::new(),
            progress: None,
            current_page: None,
            started_at: None,
            finished_at: None,
        }
    }
}

/// The status declaration order (want, reading, finished, DNF). Unarranged entries sort by it in
/// the free shelf order; the shelf list sorts by the status text instead.
pub fn declaration_index(status: ShelfStatus) -> i64 {
    match status {
        ShelfStatus::WantToRead => 0,
        ShelfStatus::CurrentlyReading => 1,
        ShelfStatus::Finished => 2,
        ShelfStatus::DidNotFinish => 3,
    }
}

/// `apply_finish_fields`: a finished book keeps its rating and its trimmed take, a DNF book its
/// trimmed reason, and every other value is cleared. A rating sent with another status is
/// silently dropped.
pub fn apply_finish_fields(
    fields: &mut StatusFields,
    status: ShelfStatus,
    rating: Option<i32>,
    take: Option<&str>,
    dnf_reason: Option<&str>,
) {
    match status {
        ShelfStatus::Finished => {
            fields.rating = rating;
            fields.take = py_strip(take.unwrap_or_default()).to_owned();
            fields.dnf_reason = String::new();
        }
        ShelfStatus::DidNotFinish => {
            fields.rating = None;
            fields.take = String::new();
            fields.dnf_reason = py_strip(dnf_reason.unwrap_or_default()).to_owned();
        }
        ShelfStatus::WantToRead | ShelfStatus::CurrentlyReading => {
            fields.rating = None;
            fields.take = String::new();
            fields.dnf_reason = String::new();
        }
    }
}

/// `apply_progress`: progress is kept only while the book is being read, and so is the page.
pub fn apply_progress(fields: &mut StatusFields, status: ShelfStatus, progress: Option<i32>) {
    if status == ShelfStatus::CurrentlyReading {
        fields.progress = progress;
    } else {
        fields.progress = None;
        fields.current_page = None;
    }
}

/// The percent read at `page` of `pages` (both at least 0 and 1): the nearest whole percent, but
/// 100 only on the last page and 1 from the first page on, so a page that was read never shows
/// as nothing and a page short of the end never as finished.
pub fn page_percent(page: i32, pages: i32) -> i32 {
    let pages = pages.max(1);
    let page = page.clamp(0, pages);
    if page == pages {
        return 100;
    }
    if page == 0 {
        return 0;
    }
    let rounded = (i64::from(page) * 100 + i64::from(pages) / 2) / i64::from(pages);
    // 0 < page < pages, so the result lies in 1..=99 and fits.
    i32::try_from(rounded.clamp(1, 99)).unwrap_or(99)
}

/// `apply_page`: the reader's page while reading a book of `pages` pages. With a page count the
/// page is clamped to it and `progress` becomes [`page_percent`]; without one (or without a
/// page) the page is dropped and `progress` stays as it is: the percent holds the position.
pub fn apply_page(
    fields: &mut StatusFields,
    status: ShelfStatus,
    page: Option<i32>,
    pages: Option<i32>,
) {
    let pages = pages.filter(|&pages| pages > 0);
    match (page, pages) {
        (Some(page), Some(pages)) if status == ShelfStatus::CurrentlyReading => {
            let page = page.clamp(0, pages);
            fields.current_page = Some(page);
            fields.progress = Some(page_percent(page, pages));
        }
        _ => fields.current_page = None,
    }
}

/// `apply_reading_dates`: stamps `started_at` and `finished_at` for an entry that lands in
/// `status`. `changed` is whether it came from another status.
///
/// - Want to read: a move there clears both dates; otherwise they stay.
/// - Any other status sets a missing `started_at` (a re-read keeps the first start).
/// - Finished sets `finished_at` on a move there, or when it is missing.
/// - Reading and DNF clear `finished_at`.
pub fn apply_reading_dates(
    fields: &mut StatusFields,
    status: ShelfStatus,
    changed: bool,
    now: OffsetDateTime,
) {
    if status == ShelfStatus::WantToRead {
        if changed {
            fields.started_at = None;
            fields.finished_at = None;
        }
        return;
    }
    if fields.started_at.is_none() {
        fields.started_at = Some(now);
    }
    if status == ShelfStatus::Finished {
        if changed || fields.finished_at.is_none() {
            fields.finished_at = Some(now);
        }
    } else {
        fields.finished_at = None;
    }
}

/// The reading dates of an entry that the Undo of its removal puts back in `status`: the removed
/// entry's own dates where the status keeps them (want to read: none; reading and DNF: the
/// start; finished: both), none later than `now`, and the start not after the finish (as the
/// PATCH's `finished_on` keeps it). A date the removed entry did not have stays empty: unlike
/// [`apply_reading_dates`], nothing is stamped, so the entry comes back as it was.
pub fn restore_reading_dates(
    fields: &mut StatusFields,
    status: ShelfStatus,
    started_at: Option<OffsetDateTime>,
    finished_at: Option<OffsetDateTime>,
    now: OffsetDateTime,
) {
    let (started_at, finished_at) = match status {
        ShelfStatus::WantToRead => (None, None),
        ShelfStatus::CurrentlyReading | ShelfStatus::DidNotFinish => (started_at, None),
        ShelfStatus::Finished => (started_at, finished_at),
    };
    let finished_at = finished_at.map(|at| at.min(now));
    let mut started_at = started_at.map(|at| at.min(now));
    if let (Some(started), Some(finished)) = (started_at, finished_at)
        && started > finished
    {
        started_at = Some(finished);
    }
    fields.started_at = started_at;
    fields.finished_at = finished_at;
}

/// The column order after `place_item`: `siblings` are the other entries of the target column
/// ordered by `(position, id)`; the item goes in at `position`, clamped to the column. Each id's
/// index in the result is its new `position`.
pub fn column_order(siblings: &[i64], item: i64, position: i64) -> Vec<i64> {
    let mut order: Vec<i64> = siblings.iter().copied().filter(|&id| id != item).collect();
    let at = position.clamp(0, order.len() as i64) as usize;
    order.insert(at, item);
    order
}

/// One entry as the free shelf order sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShelfSlot {
    pub id: i64,
    pub status: ShelfStatus,
    pub position: i32,
    pub shelf_position: Option<i32>,
}

/// `place_on_shelf`: the user's whole shelf in its free order after moving `item` to `position`
/// (clamped). Unarranged entries (no `shelf_position`) come first in status declaration order,
/// then the arranged ones by `shelf_position`; ties by `position`, then id. Each id's index in
/// the result is its new `shelf_position`, so every entry is arranged afterwards.
pub fn shelf_order(slots: &[ShelfSlot], item: i64, position: i64) -> Vec<i64> {
    let mut sorted: Vec<&ShelfSlot> = slots.iter().collect();
    sorted.sort_by_key(|slot| free_order_key(slot));
    let mut order: Vec<i64> = sorted
        .into_iter()
        .map(|slot| slot.id)
        .filter(|&id| id != item)
        .collect();
    let at = position.clamp(0, order.len() as i64) as usize;
    order.insert(at, item);
    order
}

/// Where an entry sorts in the free order (see [`shelf_order`]).
fn free_order_key(slot: &ShelfSlot) -> (bool, i64, i32, i64) {
    (
        slot.shelf_position.is_some(),
        slot.shelf_position
            .map_or_else(|| declaration_index(slot.status), i64::from),
        slot.position,
        slot.id,
    )
}

/// The index `slot` takes among the other entries of `slots` in the free order: the number of
/// them that sort before it. The Undo of a removal passes the removed entry's stored
/// `shelf_position`, which [`shelf_order`] then turns back into its old place. A removal does
/// not compact the free order, and an entry added meanwhile is unarranged and sorts first, so
/// counting (not reusing the stored number) is what keeps the order as it was.
pub fn shelf_index(slots: &[ShelfSlot], slot: &ShelfSlot) -> i64 {
    let key = free_order_key(slot);
    let before = slots
        .iter()
        .filter(|other| other.id != slot.id && free_order_key(other) < key)
        .count();
    i64::try_from(before).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    const NOW: OffsetDateTime = datetime!(2026-10-27 10:00 UTC);
    const EARLIER: OffsetDateTime = datetime!(2026-01-02 08:00 UTC);

    fn noted() -> StatusFields {
        StatusFields {
            rating: Some(4),
            take: "Fine".into(),
            dnf_reason: "Too grim".into(),
            progress: Some(50),
            current_page: None,
            started_at: Some(EARLIER),
            finished_at: Some(EARLIER),
        }
    }

    #[test]
    fn finish_fields_follow_the_status() {
        let mut fields = noted();
        apply_finish_fields(
            &mut fields,
            ShelfStatus::Finished,
            Some(5),
            Some("  Witchy and sad  "),
            Some("ignored"),
        );
        assert_eq!(
            (
                fields.rating,
                fields.take.as_str(),
                fields.dnf_reason.as_str()
            ),
            (Some(5), "Witchy and sad", "")
        );

        apply_finish_fields(
            &mut fields,
            ShelfStatus::DidNotFinish,
            Some(5),
            Some("x"),
            Some("  Too grim  "),
        );
        assert_eq!(
            (
                fields.rating,
                fields.take.as_str(),
                fields.dnf_reason.as_str()
            ),
            (None, "", "Too grim")
        );

        for status in [ShelfStatus::WantToRead, ShelfStatus::CurrentlyReading] {
            let mut fields = noted();
            apply_finish_fields(&mut fields, status, Some(5), Some("x"), Some("y"));
            assert_eq!(
                (
                    fields.rating,
                    fields.take.as_str(),
                    fields.dnf_reason.as_str()
                ),
                (None, "", "")
            );
        }

        let mut fields = noted();
        apply_finish_fields(&mut fields, ShelfStatus::Finished, None, None, None);
        assert_eq!((fields.rating, fields.take.as_str()), (None, ""));
    }

    #[test]
    fn a_page_sets_the_percent_and_is_clamped_to_the_page_count() {
        let mut fields = StatusFields::empty();
        apply_page(
            &mut fields,
            ShelfStatus::CurrentlyReading,
            Some(208),
            Some(334),
        );
        assert_eq!(
            (fields.current_page, fields.progress),
            (Some(208), Some(62))
        );
        apply_page(
            &mut fields,
            ShelfStatus::CurrentlyReading,
            Some(999),
            Some(334),
        );
        assert_eq!(
            (fields.current_page, fields.progress),
            (Some(334), Some(100))
        );
        apply_page(
            &mut fields,
            ShelfStatus::CurrentlyReading,
            Some(-3),
            Some(334),
        );
        assert_eq!((fields.current_page, fields.progress), (Some(0), Some(0)));
    }

    #[test]
    fn without_a_page_count_the_percent_holds_the_position() {
        let mut fields = StatusFields::empty();
        fields.progress = Some(40);
        fields.current_page = Some(12);
        apply_page(&mut fields, ShelfStatus::CurrentlyReading, Some(120), None);
        assert_eq!((fields.current_page, fields.progress), (None, Some(40)));
        apply_page(&mut fields, ShelfStatus::Finished, Some(120), Some(300));
        assert_eq!(fields.current_page, None);
    }

    #[test]
    fn the_page_percent_is_whole_and_honest_at_both_ends() {
        assert_eq!(page_percent(207, 334), 62);
        assert_eq!(page_percent(218, 334), 65);
        assert_eq!(page_percent(1, 334), 1, "a page read is never 0 %");
        assert_eq!(page_percent(333, 334), 99, "a page short is never 100 %");
        assert_eq!(page_percent(334, 334), 100);
        assert_eq!(page_percent(0, 334), 0);
        assert_eq!(page_percent(1, 1), 100);
    }

    #[test]
    fn progress_is_kept_only_while_reading() {
        let mut fields = StatusFields::empty();
        apply_progress(&mut fields, ShelfStatus::CurrentlyReading, Some(35));
        assert_eq!(fields.progress, Some(35));
        fields.current_page = Some(70);
        apply_progress(&mut fields, ShelfStatus::Finished, Some(35));
        assert_eq!(fields.progress, None);
        assert_eq!(fields.current_page, None);
    }

    #[test]
    fn reading_dates_follow_the_transitions() {
        // A new entry (changed = false), per status.
        let stamped = |status| {
            let mut fields = StatusFields::empty();
            apply_reading_dates(&mut fields, status, false, NOW);
            (fields.started_at, fields.finished_at)
        };
        assert_eq!(stamped(ShelfStatus::WantToRead), (None, None));
        assert_eq!(stamped(ShelfStatus::CurrentlyReading), (Some(NOW), None));
        assert_eq!(stamped(ShelfStatus::Finished), (Some(NOW), Some(NOW)));
        assert_eq!(stamped(ShelfStatus::DidNotFinish), (Some(NOW), None));

        // A re-read keeps the first start and gets a new finish.
        let mut fields = noted();
        apply_reading_dates(&mut fields, ShelfStatus::CurrentlyReading, true, NOW);
        assert_eq!(
            (fields.started_at, fields.finished_at),
            (Some(EARLIER), None)
        );
        apply_reading_dates(&mut fields, ShelfStatus::Finished, true, NOW);
        assert_eq!(
            (fields.started_at, fields.finished_at),
            (Some(EARLIER), Some(NOW))
        );

        // Back to want to read clears both; staying there keeps them.
        let mut fields = noted();
        apply_reading_dates(&mut fields, ShelfStatus::WantToRead, false, NOW);
        assert_eq!(
            (fields.started_at, fields.finished_at),
            (Some(EARLIER), Some(EARLIER))
        );
        apply_reading_dates(&mut fields, ShelfStatus::WantToRead, true, NOW);
        assert_eq!((fields.started_at, fields.finished_at), (None, None));

        // An unchanged finished entry keeps its date; a cleared one is stamped again.
        let mut fields = noted();
        apply_reading_dates(&mut fields, ShelfStatus::Finished, false, NOW);
        assert_eq!(fields.finished_at, Some(EARLIER));
        fields.finished_at = None;
        apply_reading_dates(&mut fields, ShelfStatus::Finished, false, NOW);
        assert_eq!(fields.finished_at, Some(NOW));
    }

    #[test]
    fn restored_dates_follow_the_status_and_never_lie_ahead() {
        let later = datetime!(2026-11-01 10:00 UTC);
        let restored = |status, started, finished| {
            let mut fields = noted();
            restore_reading_dates(&mut fields, status, started, finished, NOW);
            (fields.started_at, fields.finished_at)
        };
        let both = (Some(EARLIER), Some(EARLIER));
        assert_eq!(
            restored(ShelfStatus::WantToRead, both.0, both.1),
            (None, None)
        );
        for status in [ShelfStatus::CurrentlyReading, ShelfStatus::DidNotFinish] {
            assert_eq!(restored(status, both.0, both.1), (Some(EARLIER), None));
        }
        assert_eq!(restored(ShelfStatus::Finished, both.0, both.1), both);
        // Nothing is stamped: a date the entry did not have stays empty.
        assert_eq!(restored(ShelfStatus::Finished, None, None), (None, None));
        // A date in the future is now; a start after the finish is the finish.
        assert_eq!(
            restored(ShelfStatus::Finished, Some(later), Some(later)),
            (Some(NOW), Some(NOW))
        );
        assert_eq!(
            restored(ShelfStatus::Finished, Some(NOW), Some(EARLIER)),
            (Some(EARLIER), Some(EARLIER))
        );
        assert_eq!(
            restored(ShelfStatus::CurrentlyReading, Some(later), None),
            (Some(NOW), None)
        );
    }

    #[test]
    fn column_order_inserts_at_the_clamped_position() {
        assert_eq!(column_order(&[], 9, 0), vec![9]);
        assert_eq!(column_order(&[1, 2, 3], 9, 0), vec![9, 1, 2, 3]);
        assert_eq!(column_order(&[1, 2, 3], 9, 2), vec![1, 2, 9, 3]);
        assert_eq!(column_order(&[1, 2, 3], 9, 99), vec![1, 2, 3, 9]);
        assert_eq!(column_order(&[1, 2, 3], 9, -4), vec![9, 1, 2, 3]);
        // The item itself is never counted twice.
        assert_eq!(column_order(&[1, 9, 2], 9, 1), vec![1, 9, 2]);
    }

    fn slot(id: i64, status: ShelfStatus, position: i32, shelf: Option<i32>) -> ShelfSlot {
        ShelfSlot {
            id,
            status,
            position,
            shelf_position: shelf,
        }
    }

    #[test]
    fn shelf_order_puts_unarranged_entries_first_in_declaration_order() {
        let slots = [
            slot(1, ShelfStatus::WantToRead, 0, None),
            slot(2, ShelfStatus::CurrentlyReading, 0, None),
            slot(3, ShelfStatus::Finished, 0, None),
        ];
        assert_eq!(shelf_order(&slots, 3, 0), vec![3, 1, 2]);
        assert_eq!(shelf_order(&slots, 3, 99), vec![1, 2, 3]);

        // Arranged entries keep their order after the unarranged ones; an unarranged DNF
        // entry sorts after an unarranged finished one, and ties go by position, then id.
        let slots = [
            slot(1, ShelfStatus::WantToRead, 0, Some(1)),
            slot(2, ShelfStatus::CurrentlyReading, 0, Some(0)),
            slot(3, ShelfStatus::DidNotFinish, 0, None),
            slot(4, ShelfStatus::Finished, 1, None),
            slot(5, ShelfStatus::Finished, 0, None),
        ];
        assert_eq!(shelf_order(&slots, 2, 2), vec![5, 4, 2, 3, 1]);
    }

    #[test]
    fn shelf_index_counts_the_entries_before_a_stored_place() {
        // Arranged as [1, 2, 3, 4] (ranks 0..3); 3 was removed, leaving a gap, and 6 was added
        // meanwhile (unarranged, so it sorts first).
        let slots = [
            slot(1, ShelfStatus::WantToRead, 0, Some(0)),
            slot(2, ShelfStatus::Finished, 0, Some(1)),
            slot(4, ShelfStatus::Finished, 1, Some(3)),
            slot(6, ShelfStatus::WantToRead, 0, None),
            slot(9, ShelfStatus::Finished, 2, None),
        ];
        let restored = slot(9, ShelfStatus::Finished, 2, Some(2));
        let index = shelf_index(&slots, &restored);
        assert_eq!(index, 3);
        assert_eq!(shelf_order(&slots, 9, index), vec![6, 1, 2, 9, 4]);
        // At the front and past the end.
        assert_eq!(
            shelf_index(&slots, &slot(9, ShelfStatus::Finished, 2, Some(0))),
            2
        );
        assert_eq!(
            shelf_index(&slots, &slot(9, ShelfStatus::Finished, 2, Some(50))),
            4
        );
    }
}

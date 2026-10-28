//! Writing activity events (docs/phase1/conventions.md §15). Frozen.

use serde_json::{Map, Value};
use sqlx::PgConnection;

/// The legacy event kinds (docs/phase1/spec/club.md §2.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    MemberJoined,
    PickSet,
    PickCleared,
    NotePosted,
    Reaction,
    Nominated,
    Voted,
    VoteClosed,
    MilestoneAdded,
    ShelfAdded,
    ShelfMoved,
    ShelfFinished,
    ShelfDnf,
    ShelfRemoved,
    Progress,
    QuoteAdded,
}

impl EventKind {
    /// Every kind, in `events.kind` CHECK order.
    pub const ALL: [EventKind; 16] = [
        EventKind::MemberJoined,
        EventKind::PickSet,
        EventKind::PickCleared,
        EventKind::NotePosted,
        EventKind::Reaction,
        EventKind::Nominated,
        EventKind::Voted,
        EventKind::VoteClosed,
        EventKind::MilestoneAdded,
        EventKind::ShelfAdded,
        EventKind::ShelfMoved,
        EventKind::ShelfFinished,
        EventKind::ShelfDnf,
        EventKind::ShelfRemoved,
        EventKind::Progress,
        EventKind::QuoteAdded,
    ];

    /// Personal events have no club (`club_id` NULL) and are joined into club feeds at read time.
    pub fn is_personal(self) -> bool {
        matches!(
            self,
            EventKind::ShelfAdded
                | EventKind::ShelfMoved
                | EventKind::ShelfFinished
                | EventKind::ShelfDnf
                | EventKind::ShelfRemoved
                | EventKind::Progress
                | EventKind::QuoteAdded
        )
    }

    /// The stored `events.kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::MemberJoined => "member_joined",
            EventKind::PickSet => "pick_set",
            EventKind::PickCleared => "pick_cleared",
            EventKind::NotePosted => "note_posted",
            EventKind::Reaction => "reaction",
            EventKind::Nominated => "nominated",
            EventKind::Voted => "voted",
            EventKind::VoteClosed => "vote_closed",
            EventKind::MilestoneAdded => "milestone_added",
            EventKind::ShelfAdded => "shelf_added",
            EventKind::ShelfMoved => "shelf_moved",
            EventKind::ShelfFinished => "shelf_finished",
            EventKind::ShelfDnf => "shelf_dnf",
            EventKind::ShelfRemoved => "shelf_removed",
            EventKind::Progress => "progress",
            EventKind::QuoteAdded => "quote_added",
        }
    }
}

/// An event to record.
#[derive(Debug, Clone)]
pub struct NewEvent {
    pub club_id: Option<i64>,
    pub actor_id: Option<i64>,
    pub kind: EventKind,
    pub work_id: Option<i64>,
    pub pick_id: Option<i64>,
    pub payload: Map<String, Value>,
}

/// Records `ev` in the caller's transaction and returns its id. Drops null payload values (top
/// level); panics unless `club_id.is_none() == kind.is_personal()` (a programming error; the
/// `events_scope` CHECK enforces the same in the database).
pub async fn record(conn: &mut PgConnection, ev: NewEvent) -> sqlx::Result<i64> {
    assert_eq!(
        ev.club_id.is_none(),
        ev.kind.is_personal(),
        "activity event {} needs club_id {}",
        ev.kind.as_str(),
        if ev.kind.is_personal() {
            "None"
        } else {
            "Some"
        }
    );
    let payload: Map<String, Value> = ev
        .payload
        .into_iter()
        .filter(|(_, value)| !value.is_null())
        .collect();
    sqlx::query_scalar!(
        "INSERT INTO events (club_id, actor_id, kind, work_id, pick_id, payload)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id",
        ev.club_id,
        ev.actor_id,
        ev.kind.as_str(),
        ev.work_id,
        ev.pick_id,
        Value::Object(payload)
    )
    .fetch_one(conn)
    .await
}

/// Drops `rating` and `take` from every `shelf_finished` event (of `actor`, when given, on
/// `work_ids`, when given) whose actor has no `audience` shelf entry for the event's work.
///
/// The invariant it keeps: a stored `shelf_finished` payload carries a rating or take only while
/// its actor's entry for that work exists and is `audience` (security review 2026-10-28 #2).
/// The feeds also redact by the current entry at read time, but a payload outlives its entry:
/// without this, a rating written while only-me (or before a removal) came back the moment the
/// actor added the work again with the default visibility, or a merge moved the event onto an
/// audience entry. The shelf calls it when an entry goes or becomes only-me, the work merge
/// before and after it moves the events, and the importer once for every event.
pub async fn hide_finished_ratings(
    conn: &mut PgConnection,
    actor: Option<i64>,
    work_ids: Option<&[i64]>,
) -> sqlx::Result<u64> {
    let done = sqlx::query!(
        "UPDATE events e SET payload = e.payload - 'rating' - 'take'
         WHERE e.kind = 'shelf_finished'
           AND (e.payload ? 'rating' OR e.payload ? 'take')
           AND ($1::int8 IS NULL OR e.actor_id = $1)
           AND ($2::int8[] IS NULL OR e.work_id = ANY($2))
           AND NOT EXISTS (
               SELECT 1 FROM shelf_entries s
               WHERE s.user_id = e.actor_id AND s.work_id = e.work_id
                 AND s.rating_visibility = 'audience')",
        actor,
        work_ids as Option<&[i64]>
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_kinds_match_the_schema_check() {
        let personal: Vec<&str> = EventKind::ALL
            .into_iter()
            .filter(|kind| kind.is_personal())
            .map(EventKind::as_str)
            .collect();
        // events_scope in 0001_init.sql.
        assert_eq!(
            personal,
            [
                "shelf_added",
                "shelf_moved",
                "shelf_finished",
                "shelf_dnf",
                "shelf_removed",
                "progress",
                "quote_added"
            ]
        );
    }

    #[test]
    fn all_lists_every_kind_once() {
        use EventKind::*;
        for (i, kind) in EventKind::ALL.into_iter().enumerate() {
            // Exhaustive: a new variant does not compile until it has its place in ALL.
            let place = match kind {
                MemberJoined => 0,
                PickSet => 1,
                PickCleared => 2,
                NotePosted => 3,
                Reaction => 4,
                Nominated => 5,
                Voted => 6,
                VoteClosed => 7,
                MilestoneAdded => 8,
                ShelfAdded => 9,
                ShelfMoved => 10,
                ShelfFinished => 11,
                ShelfDnf => 12,
                ShelfRemoved => 13,
                Progress => 14,
                QuoteAdded => 15,
            };
            assert_eq!(place, i, "{kind:?}");
        }
    }
}

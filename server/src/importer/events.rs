//! `events` → `events` (legacy-mapping.md §12): club kinds get the imported club, personal kinds
//! none; payload references are re-mapped (`post_id` to the post that stands for the legacy pick
//! note, `author` to `author_id`, a `quote_id` added to `quote_added`).

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};
use time::{Duration, OffsetDateTime};

use super::personal::QuoteFacts;
use super::{ImportError, Importer, detail_after};
use crate::activity::{EventKind, seal_finished_ratings};

/// A `quote_added` event and the quote it was written with lie at most this far apart.
const QUOTE_WINDOW: Duration = Duration::seconds(5);

/// A legacy `quote_added` event, as it is matched to its quote.
#[derive(Debug, Clone)]
pub(crate) struct QuoteEvent {
    pub id: i64,
    pub actor_id: i64,
    pub book_id: Option<i64>,
    pub created_at: OffsetDateTime,
    pub excerpt: Option<String>,
}

/// Pairs `quote_added` events with the quotes they announced: same actor and book, the nearest
/// `created_at` within 5 s; a matching excerpt (the first 80 characters of the quote as it was
/// first written) only breaks ties, so an edited quote still matches. Each quote and each event
/// is paired at most once.
pub(crate) fn match_quotes(events: &[QuoteEvent], quotes: &[QuoteFacts]) -> HashMap<i64, i64> {
    let mut pairs = Vec::new();
    for ev in events {
        for q in quotes {
            if q.user_id != ev.actor_id || Some(q.book_id) != ev.book_id {
                continue;
            }
            let distance = (q.created_at - ev.created_at).abs();
            if distance > QUOTE_WINDOW {
                continue;
            }
            let excerpt: String = q.body.chars().take(80).collect();
            let differs = ev.excerpt.as_deref() != Some(excerpt.as_str());
            pairs.push((distance, differs, ev.id, q.id));
        }
    }
    pairs.sort();
    let mut matched = HashMap::new();
    let mut used = HashSet::new();
    for (_, _, event, quote) in pairs {
        if matched.contains_key(&event) || used.contains(&quote) {
            continue;
        }
        matched.insert(event, quote);
        used.insert(quote);
    }
    matched
}

fn kind_of(name: &str) -> Option<EventKind> {
    EventKind::ALL.into_iter().find(|k| k.as_str() == name)
}

impl Importer {
    /// Imports every event with its id. An unknown kind aborts; an event whose actor does not
    /// exist is not imported (the target deletes an account's events with it).
    pub(crate) async fn import_events(&mut self) -> Result<(), ImportError> {
        let events = self.legacy.events.clone();
        let mut kinds = Vec::with_capacity(events.len());
        for e in &events {
            let kind = kind_of(&e.kind).ok_or_else(|| {
                ImportError::Source(format!(
                    "events id {}, column kind: unknown event kind {:?}",
                    e.id, e.kind
                ))
            })?;
            kinds.push(kind);
        }
        let user_by_name: HashMap<String, i64> = self
            .legacy
            .users
            .iter()
            .filter(|u| self.user_ids.contains(&u.id))
            .map(|u| (u.username.clone(), u.id))
            .collect();

        let mut payloads = Vec::with_capacity(events.len());
        for e in &events {
            let payload = match serde_json::from_str::<Value>(&e.payload) {
                Ok(Value::Object(map)) => map,
                _ => {
                    self.report.normalised("events.payload");
                    Map::new()
                }
            };
            // As activity::record stores them: without null values.
            let payload: Map<String, Value> =
                payload.into_iter().filter(|(_, v)| !v.is_null()).collect();
            payloads.push(payload);
        }
        let quote_events: Vec<QuoteEvent> = events
            .iter()
            .zip(&kinds)
            .zip(&payloads)
            .filter(|((_, kind), _)| **kind == EventKind::QuoteAdded)
            .filter_map(|((e, _), payload)| {
                Some(QuoteEvent {
                    id: e.id,
                    actor_id: e.actor_id?,
                    book_id: e.book_id,
                    created_at: e.created_at,
                    excerpt: payload
                        .get("excerpt")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })
            })
            .collect();
        let quote_of = match_quotes(&quote_events, &self.quotes);

        let (mut club, mut personal, mut dropped, mut unmatched) = (0u64, 0u64, 0u64, 0u64);
        for ((e, kind), mut payload) in events.iter().zip(kinds).zip(payloads) {
            let Some(actor) = e.actor_id.filter(|a| self.has_user(*a)) else {
                dropped += 1;
                self.dangling(
                    "events",
                    e.id,
                    &format!("actor {:?} does not exist", e.actor_id),
                );
                continue;
            };
            let club_id = (!kind.is_personal()).then_some(self.club_id);
            let work_id = match e.book_id {
                None => None,
                Some(book) => match self.books.get(&book) {
                    Some(target) => Some(target.work_id),
                    None => {
                        self.report.normalised("events.book_id");
                        None
                    }
                },
            };
            let pick_id = match e.pick_id {
                Some(_) if kind.is_personal() => {
                    // A personal event never carries a club's pick (events_personal_no_pick).
                    self.report.normalised("events.pick_id");
                    None
                }
                Some(pick) if !self.picks.contains_key(&pick) => {
                    self.report.normalised("events.pick_id");
                    None
                }
                other => other,
            };
            if matches!(kind, EventKind::NotePosted | EventKind::Reaction)
                && let Some(legacy) = payload.remove("post_id")
            {
                match legacy.as_i64().and_then(|id| self.pick_post_of.get(&id)) {
                    Some(post) => {
                        payload.insert("post_id".into(), Value::from(*post));
                    }
                    None => self.report.normalised("events.payload.post_id"),
                }
            }
            if kind == EventKind::Reaction
                && let Some(author) = payload.remove("author")
            {
                match author.as_str().and_then(|name| user_by_name.get(name)) {
                    Some(id) => {
                        payload.insert("author_id".into(), Value::from(*id));
                    }
                    None => self.report.normalised("events.payload.author"),
                }
            }
            if kind == EventKind::QuoteAdded {
                match quote_of.get(&e.id) {
                    Some(quote) => {
                        payload.insert("quote_id".into(), Value::from(*quote));
                    }
                    None => {
                        unmatched += 1;
                        self.report.notes.push(format!(
                            "events {}: quote_added without a quote of user {actor} on book {} \
                             within 5 s (deleted); imported, shown to its actor only",
                            e.id,
                            e.book_id.map_or("-".to_owned(), |b| b.to_string())
                        ));
                    }
                }
            }
            sqlx::query!(
                "INSERT INTO events (id, club_id, actor_id, kind, work_id, pick_id, payload,
                                     created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                e.id,
                club_id,
                actor,
                kind.as_str(),
                work_id,
                pick_id,
                Value::Object(payload),
                e.created_at
            )
            .execute(&mut *self.tx)
            .await?;
            if club_id.is_some() {
                club += 1;
            } else {
                personal += 1;
            }
        }
        // A finished event's rating and take are sealed to the actor unless its entry is
        // audience (the feeds' invariant, `activity::seal_finished_ratings`): the legacy app
        // redacted only at read time.
        let hidden = seal_finished_ratings(&mut self.tx, None, None).await?;
        self.report
            .normalised_by("events.payload.rating_take", hidden);
        self.report.entity(
            "events",
            Some(events.len() as u64),
            Some(club + personal),
            detail_after(
                &format!("{club} club, {personal} personal"),
                &[
                    (unmatched, "quote_added without their quote"),
                    (dropped, "with a dangling reference"),
                ],
            ),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    fn quote(id: i64, at: OffsetDateTime, body: &str) -> QuoteFacts {
        QuoteFacts {
            id,
            user_id: 1,
            book_id: 7,
            created_at: at,
            body: body.to_owned(),
        }
    }

    fn event(id: i64, at: OffsetDateTime, excerpt: &str) -> QuoteEvent {
        QuoteEvent {
            id,
            actor_id: 1,
            book_id: Some(7),
            created_at: at,
            excerpt: Some(excerpt.to_owned()),
        }
    }

    #[test]
    fn quotes_match_the_nearest_event_within_five_seconds() {
        let t = datetime!(2026-08-01 12:00:00 UTC);
        let quotes = [
            quote(1, t, "Edited later"),
            quote(2, t + Duration::seconds(60), "Second"),
            quote(3, t + Duration::seconds(200), "Far away"),
        ];
        let events = [
            event(10, t + Duration::milliseconds(400), "Original wording"),
            event(11, t + Duration::seconds(61), "Second"),
            event(12, t + Duration::seconds(100), "Far away"),
        ];
        let matched = match_quotes(&events, &quotes);
        assert_eq!(matched.get(&10), Some(&1), "an edited quote still matches");
        assert_eq!(matched.get(&11), Some(&2));
        assert_eq!(matched.get(&12), None, "100 s apart: deleted");
    }

    #[test]
    fn the_excerpt_breaks_ties_and_each_quote_matches_once() {
        let t = datetime!(2026-08-01 12:00:00 UTC);
        let quotes = [quote(1, t, "Alpha"), quote(2, t, "Beta")];
        let events = [
            event(10, t + Duration::seconds(1), "Beta"),
            event(11, t + Duration::seconds(1), "Alpha"),
        ];
        let matched = match_quotes(&events, &quotes);
        assert_eq!(matched.get(&10), Some(&2));
        assert_eq!(matched.get(&11), Some(&1));

        let lone = [event(12, t, "Alpha"), event(13, t, "Alpha")];
        let matched = match_quotes(&lone, &quotes[..1]);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched.get(&12), Some(&1));

        let other_book = QuoteEvent {
            book_id: Some(8),
            ..event(14, t, "Alpha")
        };
        assert!(match_quotes(&[other_book], &quotes).is_empty());
    }
}

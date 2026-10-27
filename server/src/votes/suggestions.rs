//! Candidates for the next vote (docs/phase1/spec/club.md §2.9): books club members want to
//! read, ranked by how many want them, by subjects shared with the club's past picks and by the
//! Open Library rating.

use std::collections::{HashMap, HashSet};

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use sqlx::PgConnection;
use utoipa::ToSchema;

use crate::AppState;
use crate::catalog::shared::books::{BookRef, book_outs};
use crate::clubs::extract::ClubMember;
use crate::dto::BookOut;
use crate::error::{ApiResult, ErrorBody};

/// At most this many suggestions.
const SUGGESTION_LIMIT: usize = 6;
/// The Open Library rating that earns a point.
const GOOD_RATING: f64 = 4.0;

/// Ranked candidates for the next vote.
#[derive(Debug, Serialize, ToSchema)]
pub struct VoteSuggestionsOut {
    /// At most 6, best first.
    pub items: Vec<VoteSuggestionOut>,
}

/// One candidate.
#[derive(Debug, Serialize, ToSchema)]
pub struct VoteSuggestionOut {
    pub book: BookOut,
    pub score: i64,
    /// English reasons, in this order when present: the members who want it
    /// (`On 2 shelves: ada, grace` or `ada wants to read it`), `Like past picks: …` and
    /// `Open Library rating 4.4`.
    pub reasons: Vec<String>,
}

/// A candidate work with what its score is computed from.
#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    work_id: i64,
    title: String,
    /// The usernames of the members who want it, sorted.
    wanters: Vec<String>,
    subjects: Vec<String>,
    ol_rating: Option<f64>,
}

/// The score and reasons of a candidate, given the lowercased subjects of the club's past picks.
fn score(candidate: &Candidate, liked_subjects: &HashSet<String>) -> (i64, Vec<String>) {
    let mut score = 0;
    let mut reasons = Vec::new();
    let names = &candidate.wanters;
    if names.len() >= 2 {
        score += 3 * names.len() as i64;
        reasons.push(format!("On {} shelves: {}", names.len(), names.join(", ")));
    } else if let Some(name) = names.first() {
        score += 1;
        reasons.push(format!("{name} wants to read it"));
    }
    let matched: Vec<&str> = candidate
        .subjects
        .iter()
        .filter(|subject| liked_subjects.contains(&subject.to_lowercase()))
        .map(String::as_str)
        .collect();
    if !matched.is_empty() {
        score += matched.len() as i64;
        let shown: Vec<&str> = matched.iter().take(3).copied().collect();
        reasons.push(format!("Like past picks: {}", shown.join(", ")));
    }
    if let Some(rating) = candidate.ol_rating.filter(|rating| *rating >= GOOD_RATING) {
        score += 1;
        reasons.push(format!("Open Library rating {rating:.1}"));
    }
    (score, reasons)
}

/// The club's candidates: the want-to-read entries of its members (read through
/// `bc_visible_shelf`), excluding the current pick, the open vote's nominations and every past
/// pick.
async fn candidates(
    conn: &mut PgConnection,
    club_id: i64,
    viewer: i64,
) -> sqlx::Result<(Vec<Candidate>, HashSet<String>)> {
    let past_subjects = sqlx::query_scalar!(
        r#"SELECT w.subjects AS "subjects!"
           FROM club_picks p JOIN works w ON w.id = p.work_id
           WHERE p.club_id = $1 AND p.ended_at IS NOT NULL"#,
        club_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let liked: HashSet<String> = past_subjects
        .into_iter()
        .flatten()
        .map(|subject| subject.to_lowercase())
        .collect();
    let rows = sqlx::query!(
        r#"SELECT s.work_id AS "work_id!", u.username, w.title, w.subjects, w.ol_rating
           FROM bc_visible_shelf($1) s
           JOIN club_memberships m ON m.club_id = $2 AND m.user_id = s.user_id
           JOIN users u ON u.id = s.user_id
           JOIN works w ON w.id = s.work_id
           WHERE s.status = 'want_to_read'
             AND NOT EXISTS (SELECT 1 FROM club_picks p
                             WHERE p.club_id = $2 AND p.work_id = s.work_id)
             AND NOT EXISTS (SELECT 1 FROM next_up_nominations n
                             JOIN next_up_votes v ON v.id = n.vote_id
                             WHERE v.club_id = $2 AND v.ended_at IS NULL
                               AND n.work_id = s.work_id)"#,
        viewer,
        club_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut by_work: HashMap<i64, Candidate> = HashMap::new();
    for row in rows {
        by_work
            .entry(row.work_id)
            .or_insert_with(|| Candidate {
                work_id: row.work_id,
                title: row.title,
                wanters: Vec::new(),
                subjects: row.subjects,
                ol_rating: row.ol_rating,
            })
            .wanters
            .push(row.username);
    }
    let candidates = by_work
        .into_values()
        .map(|mut candidate| {
            candidate.wanters.sort();
            candidate
        })
        .collect();
    Ok((candidates, liked))
}

/// Candidates for the next vote (members): books club members want to read, at most 6, ranked
/// by score (3 per member when two or more want it, else 1; 1 per subject shared with a past
/// pick; 1 for an Open Library rating of 4.0 or more), then by title.
#[utoipa::path(
    get,
    path = "/api/clubs/{slug}/vote/suggestions",
    operation_id = "votes_suggestions",
    tag = "votes",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    responses(
        (status = 200, body = VoteSuggestionsOut, description = "The best candidates"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club` (also for a non-member)")
    )
)]
pub async fn suggestions(
    member: ClubMember,
    State(state): State<AppState>,
) -> ApiResult<Json<VoteSuggestionsOut>> {
    let mut conn = state.db.acquire().await?;
    let (candidates, liked) = candidates(&mut conn, member.club.id, member.user.id).await?;
    let mut scored: Vec<(i64, String, i64, Vec<String>)> = candidates
        .iter()
        .map(|candidate| {
            let (score, reasons) = score(candidate, &liked);
            (
                score,
                candidate.title.to_lowercase(),
                candidate.work_id,
                reasons,
            )
        })
        .collect();
    scored.sort_by(|a, b| (-a.0, &a.1, a.2).cmp(&(-b.0, &b.1, b.2)));
    scored.truncate(SUGGESTION_LIMIT);
    let refs: Vec<BookRef> = scored.iter().map(|row| BookRef::work(row.2)).collect();
    let books = book_outs(&mut conn, &refs).await?;
    let items = scored
        .into_iter()
        .filter_map(|(score, _, work_id, reasons)| {
            Some(VoteSuggestionOut {
                book: books.get(&BookRef::work(work_id))?.clone(),
                score,
                reasons,
            })
        })
        .collect();
    Ok(Json(VoteSuggestionsOut { items }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(wanters: &[&str], subjects: &[&str], ol_rating: Option<f64>) -> Candidate {
        Candidate {
            work_id: 1,
            title: "Circe".into(),
            wanters: wanters.iter().map(|s| s.to_string()).collect(),
            subjects: subjects.iter().map(|s| s.to_string()).collect(),
            ol_rating,
        }
    }

    fn liked(subjects: &[&str]) -> HashSet<String> {
        subjects.iter().map(|s| s.to_lowercase()).collect()
    }

    #[test]
    fn shared_wants_count_three_each() {
        let (score, reasons) = score(&candidate(&["ada", "grace"], &[], None), &liked(&[]));
        assert_eq!(score, 6);
        assert_eq!(reasons, ["On 2 shelves: ada, grace"]);
    }

    /// The probed example of docs/phase1/spec/club.md §2.9: every match counts, three are
    /// listed in the book's order and case (duplicates kept), and 4.25 rounds half to even.
    #[test]
    fn subjects_and_rating_follow_the_legacy_rules() {
        let (score, reasons) = score(
            &candidate(&["ada"], &["women", "ROME", "history", "Women"], Some(4.25)),
            &liked(&["Women", "rome", "History"]),
        );
        assert_eq!(score, 1 + 4 + 1);
        assert_eq!(
            reasons,
            [
                "ada wants to read it",
                "Like past picks: women, ROME, history",
                "Open Library rating 4.2"
            ]
        );
    }

    #[test]
    fn a_rating_below_four_earns_nothing() {
        let (points, reasons) = score(&candidate(&["ada"], &[], Some(3.99)), &liked(&[]));
        assert_eq!((points, reasons.len()), (1, 1));
        let (_, reasons) = score(&candidate(&["ada"], &[], Some(4.0)), &liked(&[]));
        assert_eq!(reasons[1], "Open Library rating 4.0");
        let (_, reasons) = score(&candidate(&["ada"], &[], Some(4.35)), &liked(&[]));
        assert_eq!(reasons[1], "Open Library rating 4.3");
    }
}

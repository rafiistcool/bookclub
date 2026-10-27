//! Year in review: personal stats (`GET /api/stats`) and club stats
//! (`GET /api/clubs/{slug}/stats`) (endpoint-map §7 and §9, docs/phase1/spec/diary.md §1.11,
//! §2.4 and R23).
//!
//! Both are built on [`member_years`], so the rules exist once:
//! - an entry counts in the UTC year of `finished_at`, else of `updated_at`;
//! - pages are the entry edition's, else the work's, else 0;
//! - shelves and quotes are read through `bc_visible_shelf` and `bc_visible_quotes`, so other
//!   users' `only_me` ratings and quotes are left out;
//! - means are rounded half to even to two places (Python's `round`).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use time::{OffsetDateTime, UtcOffset};
use utoipa::{IntoParams, ToSchema};

use super::{OwnerQuery, resolve_owner};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::catalog::shared::books::{BookRef, book_outs};
use crate::clubs::extract::ClubMember;
use crate::dto::{BookOut, ShelfStatus};
use crate::error::{ApiResult, ErrorBody};
use crate::extract::{ApiQuery, Validate, Validator};
use crate::text::round_half_even;

/// `?year=`: 2000 to 2100; default the newest year in `years`.
fn check_year(year: Option<i32>, v: &mut Validator) {
    v.range_i64(year.map(i64::from), 2000, 2100);
}

/// The query of `GET /api/stats`.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StatsQuery {
    /// Another user's name (case-insensitive); missing or empty means the caller. Unknown names
    /// and users outside the caller's audience get the same 404 `No member with that name`.
    #[serde(default)]
    pub username: Option<String>,
    /// 2000 to 2100; default the newest year in `years`.
    #[serde(default)]
    pub year: Option<i32>,
}

impl Validate for StatsQuery {
    fn validate(&mut self, v: &mut Validator) {
        check_year(self.year, v);
    }
}

/// The query of `GET /api/clubs/{slug}/stats`.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ClubStatsQuery {
    /// 2000 to 2100; default the newest year in `years`.
    #[serde(default)]
    pub year: Option<i32>,
}

impl Validate for ClubStatsQuery {
    fn validate(&mut self, v: &mut Validator) {
        check_year(self.year, v);
    }
}

/// One reader's year: the legacy `MemberYear`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemberYear {
    pub username: String,
    /// Books finished in the year.
    pub finished: i64,
    /// Books not finished, counted in the year of their last change.
    pub dnf: i64,
    /// The pages of the finished books: the entry's edition's, else the work's.
    pub pages: i64,
    /// The mean rating of the finished books, two places; null without ratings.
    pub average_rating: Option<f64>,
    pub five_stars: i64,
    /// The highest rated finished book (ties: the newest shelf entry).
    pub top_book: Option<BookOut>,
    /// The finished book with the most pages (ties: the oldest shelf entry).
    pub longest_book: Option<BookOut>,
    /// Quotes kept in the year.
    pub quotes: i64,
}

/// Books finished in one month.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct MonthCount {
    /// `YYYY-MM` (UTC).
    pub month: String,
    pub finished: i64,
}

/// A user's year in review.
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonalStatsOut {
    pub username: String,
    /// The year shown.
    pub year: i32,
    /// Every year with a finished or abandoned book, and the current year, newest first.
    pub years: Vec<i32>,
    pub finished: i64,
    pub dnf: i64,
    pub pages: i64,
    pub average_rating: Option<f64>,
    pub five_stars: i64,
    pub top_book: Option<BookOut>,
    pub longest_book: Option<BookOut>,
    pub quotes: i64,
    /// Months with finished books, oldest first.
    pub by_month: Vec<MonthCount>,
}

/// How the club read one pick (all time).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PickStat {
    pub pick_id: i64,
    pub book: BookOut,
    /// Who set the pick; `unknown` when the account is gone.
    pub set_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub ended_at: Option<OffsetDateTime>,
    /// Members with the work on their shelf, in any status.
    pub readers: i64,
    pub finished: i64,
    pub dnf: i64,
    pub average_rating: Option<f64>,
    /// The members' ratings, highest first.
    pub ratings: Vec<i32>,
}

/// A club's year in review: the legacy `StatsOut`, over the club's members and picks.
#[derive(Debug, Serialize, ToSchema)]
pub struct ClubStatsOut {
    pub year: i32,
    /// Every year with a member's finished or abandoned book, and the current year, newest
    /// first.
    pub years: Vec<i32>,
    /// Every member, most books finished first, then by username.
    pub members: Vec<MemberYear>,
    /// The members' finished books per month, oldest first.
    pub by_month: Vec<MonthCount>,
    /// Every pick of the club, newest first.
    pub picks: Vec<PickStat>,
    pub club_finished: i64,
    pub club_pages: i64,
    pub club_average_rating: Option<f64>,
    /// The pick with the best mean rating (ties: more ratings, then the newer pick).
    pub best_pick: Option<PickStat>,
}

/// A user's year in review (`?username=` for someone in the caller's audience). For anyone but
/// the owner, `only_me` ratings and quotes are left out.
#[utoipa::path(
    get,
    path = "/api/stats",
    operation_id = "shelf_personal_stats",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(StatsQuery),
    responses(
        (status = 200, body = PersonalStatsOut, description = "The year in review"),
        (status = 400, body = ErrorBody, description = "A `year` outside 2000..2100"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No member with that name` (also for users outside the caller's audience)")
    )
)]
pub async fn personal_stats(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<StatsQuery>,
) -> ApiResult<Json<PersonalStatsOut>> {
    let mut conn = state.db.acquire().await?;
    let owner = resolve_owner(
        &mut conn,
        &user,
        &OwnerQuery {
            username: query.username,
        },
    )
    .await?;
    let log = ReadingLog::load(&mut conn, user.id, &[owner.id]).await?;
    let year = log.chosen(query.year);
    let member = member_years(&mut conn, &log, &[(owner.id, owner.username)], year)
        .await?
        .pop()
        .map(|(member, _)| member)
        .ok_or_else(|| sqlx::Error::RowNotFound)?;
    Ok(Json(PersonalStatsOut {
        username: member.username,
        year,
        years: log.years(),
        finished: member.finished,
        dnf: member.dnf,
        pages: member.pages,
        average_rating: member.average_rating,
        five_stars: member.five_stars,
        top_book: member.top_book,
        longest_book: member.longest_book,
        quotes: member.quotes,
        by_month: log.by_month(year),
    }))
}

/// The club's year in review: its members and its picks. Members' `only_me` ratings and quotes
/// are left out for everyone but their owner.
#[utoipa::path(
    get,
    path = "/api/clubs/{slug}/stats",
    operation_id = "shelf_club_stats",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug"), ClubStatsQuery),
    responses(
        (status = 200, body = ClubStatsOut, description = "The club's year in review"),
        (status = 400, body = ErrorBody, description = "A `year` outside 2000..2100"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club`")
    )
)]
pub async fn club_stats(
    member: ClubMember,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<ClubStatsQuery>,
) -> ApiResult<Json<ClubStatsOut>> {
    let viewer = member.user.id;
    let club_id = member.club.id;
    let mut conn = state.db.acquire().await?;
    let members: Vec<(i64, String)> = sqlx::query!(
        "SELECT u.id, u.username
         FROM club_memberships m
         JOIN users u ON u.id = m.user_id
         WHERE m.club_id = $1",
        club_id
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|row| (row.id, row.username))
    .collect();
    let member_ids: Vec<i64> = members.iter().map(|(id, _)| *id).collect();
    let log = ReadingLog::load(&mut conn, viewer, &member_ids).await?;
    let year = log.chosen(query.year);

    let mut rows = member_years(&mut conn, &log, &members, year).await?;
    rows.sort_by(|(a, _), (b, _)| {
        b.finished
            .cmp(&a.finished)
            .then_with(|| a.username.cmp(&b.username))
    });
    let club_ratings: Vec<i32> = rows
        .iter()
        .flat_map(|(_, totals)| totals.ratings.iter().copied())
        .collect();
    let club_finished = rows.iter().map(|(member, _)| member.finished).sum();
    let club_pages = rows.iter().map(|(member, _)| member.pages).sum();
    let picks = pick_stats(&mut conn, viewer, club_id).await?;
    let best_pick = best_pick(&picks).cloned();
    Ok(Json(ClubStatsOut {
        year,
        years: log.years(),
        members: rows.into_iter().map(|(member, _)| member).collect(),
        by_month: log.by_month(year),
        picks,
        club_finished,
        club_pages,
        club_average_rating: mean(&club_ratings),
        best_pick,
    }))
}

/// A finished or abandoned shelf entry as the stats count it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Counted {
    id: i64,
    user_id: i64,
    work_id: i64,
    edition_id: Option<i64>,
    finished: bool,
    /// Already redacted for others' `only_me` entries.
    rating: Option<i32>,
    /// The edition's pages, else the work's, else 0.
    pages: i64,
    /// `finished_at`, else `updated_at`.
    stamp: OffsetDateTime,
}

impl Counted {
    fn book(&self) -> BookRef {
        BookRef {
            work_id: self.work_id,
            edition_id: self.edition_id,
        }
    }
}

/// What the stats of some owners need, as one viewer sees it.
#[derive(Debug, Default)]
struct ReadingLog {
    /// The owners' finished and abandoned entries, by id.
    entries: Vec<Counted>,
    /// Visible quotes per (owner, UTC year).
    quotes: HashMap<(i64, i32), i64>,
}

impl ReadingLog {
    async fn load(conn: &mut PgConnection, viewer: i64, owners: &[i64]) -> sqlx::Result<Self> {
        let entries = sqlx::query!(
            r#"SELECT s.id AS "id!", s.user_id AS "user_id!", s.work_id AS "work_id!",
                      s.edition_id, s.status AS "status!: ShelfStatus", s.rating,
                      coalesce(e.pages, w.pages, 0)::int8 AS "pages!",
                      coalesce(s.finished_at, s.updated_at) AS "stamp!"
               FROM bc_visible_shelf($1) AS s
               JOIN works w ON w.id = s.work_id
               LEFT JOIN editions e ON e.id = s.edition_id AND e.work_id = s.work_id
               WHERE s.user_id = ANY($2) AND s.status IN ('finished', 'did_not_finish')
               ORDER BY s.id"#,
            viewer,
            owners
        )
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|row| Counted {
            id: row.id,
            user_id: row.user_id,
            work_id: row.work_id,
            edition_id: row.edition_id,
            finished: row.status == ShelfStatus::Finished,
            rating: row.rating,
            pages: row.pages,
            stamp: row.stamp,
        })
        .collect();
        let quotes = sqlx::query!(
            r#"SELECT q.user_id AS "user_id!",
                      extract(year FROM q.created_at AT TIME ZONE 'UTC')::int4 AS "year!",
                      count(*) AS "count!"
               FROM bc_visible_quotes($1) AS q
               WHERE q.user_id = ANY($2)
               GROUP BY 1, 2"#,
            viewer,
            owners
        )
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|row| ((row.user_id, row.year), row.count))
        .collect();
        Ok(Self { entries, quotes })
    }

    /// Every year with an entry, and the current UTC year, newest first.
    fn years(&self) -> Vec<i32> {
        let mut years: BTreeSet<i32> = self.entries.iter().map(|e| year_of(e.stamp)).collect();
        years.insert(OffsetDateTime::now_utc().year());
        years.into_iter().rev().collect()
    }

    /// `year`, or the newest of [`ReadingLog::years`].
    fn chosen(&self, year: Option<i32>) -> i32 {
        year.unwrap_or_else(|| self.years()[0])
    }

    /// The finished entries of `year` per UTC month, oldest first.
    fn by_month(&self, year: i32) -> Vec<MonthCount> {
        let mut months: BTreeMap<String, i64> = BTreeMap::new();
        for entry in self
            .entries
            .iter()
            .filter(|e| e.finished && year_of(e.stamp) == year)
        {
            let utc = entry.stamp.to_offset(UtcOffset::UTC);
            let month = format!("{:04}-{:02}", utc.year(), u8::from(utc.month()));
            *months.entry(month).or_default() += 1;
        }
        months
            .into_iter()
            .map(|(month, finished)| MonthCount { month, finished })
            .collect()
    }
}

/// The UTC year an entry counts in.
fn year_of(stamp: OffsetDateTime) -> i32 {
    stamp.to_offset(UtcOffset::UTC).year()
}

/// One owner's counts for a year, before the books are loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
struct YearTotals {
    finished: i64,
    dnf: i64,
    pages: i64,
    /// The finished entries' ratings, in entry order.
    ratings: Vec<i32>,
    top: Option<BookRef>,
    longest: Option<BookRef>,
    quotes: i64,
}

/// The legacy `MemberYear` rules over one owner's entries of `year`.
fn year_totals(log: &ReadingLog, owner: i64, year: i32) -> YearTotals {
    let mut totals = YearTotals {
        finished: 0,
        dnf: 0,
        pages: 0,
        ratings: Vec::new(),
        top: None,
        longest: None,
        quotes: log.quotes.get(&(owner, year)).copied().unwrap_or(0),
    };
    let mut top: Option<(i32, i64, BookRef)> = None;
    let mut longest: Option<(i64, BookRef)> = None;
    for entry in log
        .entries
        .iter()
        .filter(|e| e.user_id == owner && year_of(e.stamp) == year)
    {
        if !entry.finished {
            totals.dnf += 1;
            continue;
        }
        totals.finished += 1;
        totals.pages += entry.pages;
        if let Some(rating) = entry.rating.filter(|&rating| rating != 0) {
            totals.ratings.push(rating);
            // max by (rating, entry id): a tie goes to the newest entry.
            if top.is_none_or(|(best, id, _)| (rating, entry.id) > (best, id)) {
                top = Some((rating, entry.id, entry.book()));
            }
        }
        // The first entry with the most pages wins a tie (entries are in id order).
        if entry.pages > 0 && longest.is_none_or(|(most, _)| entry.pages > most) {
            longest = Some((entry.pages, entry.book()));
        }
    }
    totals.top = top.map(|(_, _, book)| book);
    totals.longest = longest.map(|(_, book)| book);
    totals
}

/// The `MemberYear` of each owner for `year`, with the totals it came from (the club sums
/// them), in the order of `owners`.
async fn member_years(
    conn: &mut PgConnection,
    log: &ReadingLog,
    owners: &[(i64, String)],
    year: i32,
) -> sqlx::Result<Vec<(MemberYear, YearTotals)>> {
    let totals: Vec<YearTotals> = owners
        .iter()
        .map(|(owner, _)| year_totals(log, *owner, year))
        .collect();
    let refs: Vec<BookRef> = totals
        .iter()
        .flat_map(|t| t.top.into_iter().chain(t.longest))
        .collect();
    let books = book_outs(conn, &refs).await?;
    let book = |r: Option<BookRef>| r.and_then(|r| books.get(&r).cloned());
    Ok(owners
        .iter()
        .zip(totals)
        .map(|((_, username), totals)| {
            let member = MemberYear {
                username: username.clone(),
                finished: totals.finished,
                dnf: totals.dnf,
                pages: totals.pages,
                average_rating: mean(&totals.ratings),
                five_stars: totals.ratings.iter().filter(|&&r| r == 5).count() as i64,
                top_book: book(totals.top),
                longest_book: book(totals.longest),
                quotes: totals.quotes,
            };
            (member, totals)
        })
        .collect())
}

/// The mean rounded half to even to two places; null without values.
fn mean(values: &[i32]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let sum: i64 = values.iter().map(|&v| i64::from(v)).sum();
    Some(round_half_even(sum as f64 / values.len() as f64, 2))
}

/// Every pick of the club, newest first, with the members' shelves for its work (all time,
/// through `bc_visible_shelf`).
async fn pick_stats(
    conn: &mut PgConnection,
    viewer: i64,
    club_id: i64,
) -> sqlx::Result<Vec<PickStat>> {
    let picks = sqlx::query!(
        r#"SELECT p.id, p.work_id, u.username AS "set_by?", p.started_at, p.ended_at
           FROM club_picks p
           LEFT JOIN users u ON u.id = p.set_by
           WHERE p.club_id = $1
           ORDER BY p.id DESC"#,
        club_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let work_ids: Vec<i64> = picks.iter().map(|pick| pick.work_id).collect();
    let readers = sqlx::query!(
        r#"SELECT s.work_id AS "work_id!", s.status AS "status!: ShelfStatus", s.rating
           FROM bc_visible_shelf($1) AS s
           JOIN club_memberships m ON m.club_id = $2 AND m.user_id = s.user_id
           WHERE s.work_id = ANY($3)
           ORDER BY s.id"#,
        viewer,
        club_id,
        &work_ids
    )
    .fetch_all(&mut *conn)
    .await?;
    let refs: Vec<BookRef> = work_ids.iter().map(|&id| BookRef::work(id)).collect();
    let books = book_outs(conn, &refs).await?;
    Ok(picks
        .into_iter()
        .filter_map(|pick| {
            let book = books.get(&BookRef::work(pick.work_id))?.clone();
            let shelves = readers.iter().filter(|r| r.work_id == pick.work_id);
            let mut stat = PickStat {
                pick_id: pick.id,
                book,
                set_by: pick.set_by.unwrap_or_else(|| "unknown".to_owned()),
                started_at: pick.started_at,
                ended_at: pick.ended_at,
                readers: 0,
                finished: 0,
                dnf: 0,
                average_rating: None,
                ratings: Vec::new(),
            };
            for shelf in shelves {
                stat.readers += 1;
                match shelf.status {
                    ShelfStatus::Finished => {
                        stat.finished += 1;
                        stat.ratings.extend(shelf.rating.filter(|&r| r != 0));
                    }
                    ShelfStatus::DidNotFinish => stat.dnf += 1,
                    _ => {}
                }
            }
            stat.average_rating = mean(&stat.ratings);
            stat.ratings.sort_unstable_by(|a, b| b.cmp(a));
            Some(stat)
        })
        .collect())
}

/// The pick with the highest `(average_rating, number of ratings)`; the first (newest) wins a
/// full tie.
fn best_pick(picks: &[PickStat]) -> Option<&PickStat> {
    let mut best: Option<(&PickStat, f64)> = None;
    for pick in picks {
        let Some(average) = pick.average_rating else {
            continue;
        };
        let better = match best {
            None => true,
            Some((current, current_average)) => {
                average > current_average
                    || (average == current_average && pick.ratings.len() > current.ratings.len())
            }
        };
        if better {
            best = Some((pick, average));
        }
    }
    best.map(|(pick, _)| pick)
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    fn entry(id: i64, finished: bool, rating: Option<i32>, pages: i64) -> Counted {
        Counted {
            id,
            user_id: 1,
            work_id: id * 10,
            edition_id: None,
            finished,
            rating,
            pages,
            stamp: datetime!(2025-03-04 10:00 UTC),
        }
    }

    #[test]
    fn means_round_half_to_even_like_python() {
        assert_eq!(mean(&[]), None);
        assert_eq!(mean(&[5]), Some(5.0));
        // 33/8 = 4.125 → 4.12, 37/8 = 4.625 → 4.62, 35/8 = 4.375 → 4.38 (spec §1.11, probed).
        assert_eq!(mean(&[5, 5, 5, 5, 5, 4, 3, 1]), Some(4.12));
        assert_eq!(mean(&[5, 5, 5, 5, 5, 5, 4, 3]), Some(4.62));
        assert_eq!(mean(&[5, 5, 5, 5, 5, 4, 4, 2]), Some(4.38));
        assert_eq!(mean(&[1, 2]), Some(1.5));
    }

    #[test]
    fn top_book_tie_goes_to_the_newest_entry_and_longest_to_the_oldest() {
        let log = ReadingLog {
            entries: vec![
                entry(1, true, Some(5), 300),
                entry(2, true, Some(5), 300),
                entry(3, true, Some(4), 100),
                entry(4, false, None, 900),
                entry(5, true, None, 0),
            ],
            quotes: HashMap::from([((1, 2025), 3), ((1, 2024), 9)]),
        };
        let totals = year_totals(&log, 1, 2025);
        assert_eq!(totals.finished, 4);
        assert_eq!(totals.dnf, 1);
        assert_eq!(totals.pages, 700);
        assert_eq!(totals.ratings, vec![5, 5, 4]);
        assert_eq!(totals.top, Some(BookRef::work(20)));
        assert_eq!(totals.longest, Some(BookRef::work(10)));
        assert_eq!(totals.quotes, 3);
        assert_eq!(year_totals(&log, 1, 2024).finished, 0);
        assert_eq!(year_totals(&log, 2, 2025).finished, 0);
    }

    #[test]
    fn years_and_months_are_utc() {
        let mut late = entry(1, true, None, 0);
        // 00:30 on 1 January in Berlin is still the old year in UTC.
        late.stamp = datetime!(2025-01-01 00:30 +01:00);
        let log = ReadingLog {
            entries: vec![late, entry(2, false, None, 0)],
            quotes: HashMap::new(),
        };
        let now = OffsetDateTime::now_utc().year();
        let mut expected: Vec<i32> = vec![now, 2025, 2024];
        expected.dedup();
        assert_eq!(log.years(), expected);
        assert_eq!(
            log.by_month(2024),
            vec![MonthCount {
                month: "2024-12".into(),
                finished: 1
            }]
        );
        // DNF entries are not finished books.
        assert_eq!(log.by_month(2025), vec![]);
    }

    fn pick(id: i64, ratings: Vec<i32>) -> PickStat {
        PickStat {
            pick_id: id,
            book: BookOut {
                id,
                ol_work_key: None,
                title: String::new(),
                authors: String::new(),
                year: None,
                cover_url: None,
                cover_thumb_url: None,
                cover_public_id: None,
                pages: None,
                spine_color: None,
                source: crate::dto::CatalogSource::User,
                custom: true,
            },
            set_by: "ada".into(),
            started_at: datetime!(2025-01-01 0:00 UTC),
            ended_at: None,
            readers: 0,
            finished: 0,
            dnf: 0,
            average_rating: mean(&ratings),
            ratings,
        }
    }

    #[test]
    fn best_pick_prefers_the_mean_then_more_ratings_then_the_newer_pick() {
        assert!(best_pick(&[pick(3, vec![])]).is_none());
        let picks = [
            pick(4, vec![4]),
            pick(3, vec![4, 4]),
            pick(2, vec![4, 4]),
            pick(1, vec![5]),
        ];
        assert_eq!(best_pick(&picks).map(|p| p.pick_id), Some(1));
        let picks = [pick(4, vec![4]), pick(3, vec![4, 4]), pick(2, vec![4, 4])];
        assert_eq!(best_pick(&picks).map(|p| p.pick_id), Some(3));
    }
}

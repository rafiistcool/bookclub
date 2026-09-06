"""Reading statistics: per member, per year, and per club pick."""

from collections import defaultdict
from datetime import datetime, timezone

from fastapi import APIRouter, Depends, Query
from pydantic import BaseModel
from sqlalchemy.orm import selectinload
from sqlmodel import Session, select

from app.deps import get_current_user, get_session
from app.models import ClubPick, Quote, ShelfEntry, ShelfStatus, User
from app.schemas import BookOut
from app.serialize import book_out

router = APIRouter(prefix="/api/stats", tags=["stats"])


class MemberYear(BaseModel):
    username: str
    finished: int
    dnf: int
    pages: int
    average_rating: float | None
    five_stars: int
    top_book: BookOut | None = None
    longest_book: BookOut | None = None
    quotes: int = 0


class MonthCount(BaseModel):
    month: str  # YYYY-MM
    finished: int


class PickStat(BaseModel):
    pick_id: int
    book: BookOut
    set_by: str
    started_at: datetime
    ended_at: datetime | None
    readers: int
    finished: int
    dnf: int
    average_rating: float | None
    ratings: list[int]


class StatsOut(BaseModel):
    year: int
    years: list[int]
    members: list[MemberYear]
    by_month: list[MonthCount]
    picks: list[PickStat]
    club_finished: int
    club_pages: int
    club_average_rating: float | None
    best_pick: PickStat | None = None


def _aware(dt: datetime | None) -> datetime | None:
    if dt is None:
        return None
    return dt if dt.tzinfo is not None else dt.replace(tzinfo=timezone.utc)


def _avg(values: list[int]) -> float | None:
    return round(sum(values) / len(values), 2) if values else None


@router.get("", response_model=StatsOut)
def stats(
    year: int | None = Query(default=None, ge=2000, le=2100),
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> StatsOut:
    entries = session.exec(
        select(ShelfEntry)
        .where(
            ShelfEntry.status.in_([ShelfStatus.finished, ShelfStatus.did_not_finish])  # type: ignore[attr-defined]
        )
        .options(selectinload(ShelfEntry.book), selectinload(ShelfEntry.user))
    ).all()
    members = session.exec(select(User).order_by(User.username)).all()
    quotes = session.exec(select(Quote)).all()

    def year_of(entry: ShelfEntry) -> int:
        stamp = _aware(entry.finished_at) or _aware(entry.updated_at) or datetime.now(timezone.utc)
        return stamp.year

    years = sorted({year_of(e) for e in entries} | {datetime.now(timezone.utc).year}, reverse=True)
    chosen = year if year is not None else years[0]

    per_member: dict[int, list[ShelfEntry]] = defaultdict(list)
    for entry in entries:
        if year_of(entry) == chosen:
            per_member[entry.user_id].append(entry)
    quotes_by_user: dict[int, int] = defaultdict(int)
    for quote in quotes:
        if _aware(quote.created_at) and _aware(quote.created_at).year == chosen:  # type: ignore[union-attr]
            quotes_by_user[quote.user_id] += 1

    member_rows: list[MemberYear] = []
    months: dict[str, int] = defaultdict(int)
    club_ratings: list[int] = []
    club_finished = 0
    club_pages = 0
    for user in members:
        rows = per_member.get(user.id or -1, [])
        finished = [r for r in rows if r.status == ShelfStatus.finished]
        dnf = [r for r in rows if r.status == ShelfStatus.did_not_finish]
        ratings = [r.rating for r in finished if r.rating]
        pages = sum((r.book.pages or 0) for r in finished if r.book)
        top = max((r for r in finished if r.rating), key=lambda r: (r.rating or 0, r.id or 0), default=None)
        longest = max((r for r in finished if r.book and r.book.pages), key=lambda r: r.book.pages or 0, default=None)  # type: ignore[union-attr]
        for r in finished:
            stamp = _aware(r.finished_at) or _aware(r.updated_at)
            if stamp:
                months[stamp.strftime("%Y-%m")] += 1
        club_ratings += ratings
        club_finished += len(finished)
        club_pages += pages
        member_rows.append(
            MemberYear(
                username=user.username,
                finished=len(finished),
                dnf=len(dnf),
                pages=pages,
                average_rating=_avg(ratings),
                five_stars=sum(1 for r in ratings if r == 5),
                top_book=book_out(top.book) if top and top.book else None,
                longest_book=book_out(longest.book) if longest and longest.book else None,
                quotes=quotes_by_user.get(user.id or -1, 0),
            )
        )
    member_rows.sort(key=lambda row: (-row.finished, row.username))

    picks = session.exec(
        select(ClubPick)
        .options(selectinload(ClubPick.book), selectinload(ClubPick.set_by))
        .order_by(ClubPick.id.desc())
    ).all()
    pick_rows: list[PickStat] = []
    for pick in picks:
        if pick.book is None:
            continue
        rows = [e for e in entries if e.book_id == pick.book_id]
        all_rows = session.exec(select(ShelfEntry).where(ShelfEntry.book_id == pick.book_id)).all()
        finished_rows = [e for e in rows if e.status == ShelfStatus.finished]
        ratings = [e.rating for e in finished_rows if e.rating]
        pick_rows.append(
            PickStat(
                pick_id=pick.id or 0,
                book=book_out(pick.book),
                set_by=pick.set_by.username if pick.set_by else "unknown",
                started_at=pick.started_at,
                ended_at=pick.ended_at,
                readers=len(all_rows),
                finished=len(finished_rows),
                dnf=sum(1 for e in rows if e.status == ShelfStatus.did_not_finish),
                average_rating=_avg(ratings),
                ratings=sorted(ratings, reverse=True),
            )
        )
    rated = [p for p in pick_rows if p.average_rating is not None]
    best = max(rated, key=lambda p: (p.average_rating or 0, len(p.ratings)), default=None)

    return StatsOut(
        year=chosen,
        years=years,
        members=member_rows,
        by_month=[MonthCount(month=m, finished=c) for m, c in sorted(months.items())],
        picks=pick_rows,
        club_finished=club_finished,
        club_pages=club_pages,
        club_average_rating=_avg(club_ratings),
        best_pick=best,
    )

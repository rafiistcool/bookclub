from datetime import timezone as dt_timezone

from fastapi import APIRouter, BackgroundTasks, Depends, HTTPException, Request
from sqlalchemy.engine import Engine
from sqlalchemy.orm import selectinload
from sqlmodel import Session, col, select

from app import activity
from app.config import get_settings
from app.deps import get_current_user, get_session
from app.models import (
    NOMINATION_LIMIT,
    Book,
    ClubPick,
    NextUpBallot,
    NextUpNomination,
    NextUpVote,
    ShelfEntry,
    ShelfStatus,
    User,
    utcnow,
)
from app.openlibrary import subjects_from_json
from app.pick_ops import apply_club_pick, current_pick, pick_out
from app.schemas import (
    ClubPickSetIn,
    VoteApplyIn,
    VoteApplyOut,
    VoteCastIn,
    VoteDeadlineIn,
    VoteNominateIn,
    VoteNominationOut,
    VoteOut,
    VoteSuggestionOut,
    VoteSuggestionsOut,
)
from app.serialize import book_out
from app.shelf_ops import upsert_book
from app.timezone import meeting_label, meeting_local, parse_meeting, resolved_timezone

router = APIRouter(prefix="/api/vote", tags=["vote"])
SUGGESTION_LIMIT = 6


def _timezone() -> str:
    return resolved_timezone(get_settings().bookclub_tz)


def _open_vote(session: Session) -> NextUpVote | None:
    return session.exec(
        select(NextUpVote)
        .where(NextUpVote.ended_at.is_(None))
        .order_by(NextUpVote.id.desc())
    ).first()


def _nomination_count(session: Session, vote_id: int) -> int:
    return len(
        session.exec(
            select(NextUpNomination).where(NextUpNomination.vote_id == vote_id)
        ).all()
    )


def _nominations(session: Session, vote: NextUpVote) -> list[NextUpNomination]:
    return session.exec(
        select(NextUpNomination)
        .where(NextUpNomination.vote_id == vote.id)
        .options(
            selectinload(NextUpNomination.book),
            selectinload(NextUpNomination.nominated_by),
            selectinload(NextUpNomination.ballots).selectinload(NextUpBallot.user),
        )
        .order_by(NextUpNomination.created_at, NextUpNomination.id)
    ).all()


def _leader(rows: list[NextUpNomination]) -> NextUpNomination | None:
    """Most ballots wins; ties go to the earliest nomination. None when nobody voted."""
    best: NextUpNomination | None = None
    best_votes = 0
    for row in rows:
        votes = len(row.ballots)
        if votes > best_votes:
            best, best_votes = row, votes
    return best


def _aware(dt):
    if dt is not None and dt.tzinfo is None:
        return dt.replace(tzinfo=dt_timezone.utc)
    return dt


def _close_vote(
    session: Session,
    vote: NextUpVote,
    winner: NextUpNomination,
    actor: User,
    *,
    auto: bool,
) -> ClubPick:
    """End the vote and make the winner the club pick (deadline auto-close path)."""
    if winner.book is None:
        raise HTTPException(status_code=500, detail="Nomination is missing its book")
    book = winner.book
    pick = apply_club_pick(
        session,
        actor,
        ClubPickSetIn(
            ol_work_key=book.ol_work_key,
            title=book.title,
            authors=book.authors,
            cover_id=book.cover_id,
            year=book.year,
        ),
        _timezone(),
    )
    vote.ended_at = utcnow()
    vote.winner_nomination_id = winner.id
    session.add(vote)
    activity.record(
        session,
        actor,
        "vote_closed",
        book=book,
        pick=pick,
        votes=len(winner.ballots),
        auto=auto,
    )
    return pick


def maybe_auto_close(
    session: Session,
    *,
    background: BackgroundTasks | None = None,
    engine: Engine | None = None,
) -> ClubPick | None:
    """If the open vote's deadline has passed and someone leads, apply the winner.

    Runs opportunistically on reads (no scheduler in a single-process app).
    The nominator of the winning book is recorded as the pick setter.
    """
    vote = _open_vote(session)
    if vote is None or vote.closes_at is None:
        return None
    if _aware(vote.closes_at) > utcnow():
        return None
    rows = _nominations(session, vote)
    winner = _leader(rows)
    if winner is None or winner.nominated_by is None:
        # Nobody voted: drop the deadline and leave the vote open so the club
        # can decide by hand.
        vote.closes_at = None
        session.add(vote)
        session.commit()
        return None
    actor = winner.nominated_by
    pick = _close_vote(session, vote, winner, actor, auto=True)
    session.commit()
    if engine is not None:
        from app.routers.pick import notify_new_pick

        notify_new_pick(background, engine, session, pick, actor)
    return pick


def _vote_out(session: Session, vote: NextUpVote | None, viewer: User) -> VoteOut:
    tz_name = _timezone()
    members = session.exec(select(User).order_by(User.username)).all()
    if vote is None:
        return VoteOut(
            vote_id=None,
            nominations=[],
            my_vote_id=None,
            nomination_limit=NOMINATION_LIMIT,
            can_nominate=True,
            timezone=tz_name,
            member_count=len(members),
        )
    rows = _nominations(session, vote)
    nominations: list[VoteNominationOut] = []
    my_vote_id = None
    voted: set[str] = set()
    for row in rows:
        if row.book is None:
            raise HTTPException(status_code=500, detail="Nomination is missing its book")
        namer = row.nominated_by.username if row.nominated_by is not None else "unknown"
        voters: list[str] = []
        mine = False
        for ballot in row.ballots:
            if ballot.user is None:
                continue
            voters.append(ballot.user.username)
            voted.add(ballot.user.username)
            if ballot.user_id == viewer.id:
                mine = True
                my_vote_id = row.id
        voters.sort()
        nominations.append(
            VoteNominationOut(
                id=row.id or 0,
                book=book_out(row.book),
                nominated_by=namer,
                votes=len(voters),
                voters=voters,
                mine=mine,
            )
        )
    nominations.sort(key=lambda item: (-item.votes, item.id))
    leader = _leader(rows)
    return VoteOut(
        vote_id=vote.id,
        nominations=nominations,
        my_vote_id=my_vote_id,
        nomination_limit=NOMINATION_LIMIT,
        can_nominate=len(nominations) < NOMINATION_LIMIT,
        timezone=tz_name,
        closes_at=_aware(vote.closes_at),
        closes_local=meeting_local(vote.closes_at, tz_name),
        closes_label=meeting_label(vote.closes_at, tz_name),
        not_voted=[user.username for user in members if user.username not in voted],
        voted_count=len(voted),
        member_count=len(members),
        leader_id=leader.id if leader is not None else None,
    )


def _load_open_nomination(session: Session, nomination_id: int) -> NextUpNomination:
    vote = _open_vote(session)
    if vote is None:
        raise HTTPException(status_code=404, detail="No open next-up vote")
    row = session.exec(
        select(NextUpNomination)
        .where(
            NextUpNomination.id == nomination_id,
            NextUpNomination.vote_id == vote.id,
        )
        .options(
            selectinload(NextUpNomination.book),
            selectinload(NextUpNomination.nominated_by),
            selectinload(NextUpNomination.ballots),
        )
    ).first()
    if row is None:
        raise HTTPException(status_code=404, detail="No nomination with that id")
    return row


@router.get("", response_model=VoteOut)
def get_vote(
    request: Request,
    background: BackgroundTasks,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> VoteOut:
    maybe_auto_close(session, background=background, engine=request.app.state.engine)
    return _vote_out(session, _open_vote(session), me)


@router.patch("", response_model=VoteOut)
def set_deadline(
    payload: VoteDeadlineIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> VoteOut:
    vote = _open_vote(session)
    if vote is None:
        raise HTTPException(status_code=404, detail="Nominate a book first")
    try:
        closes = parse_meeting(payload.closes_at, _timezone())
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    if closes is not None and closes <= utcnow():
        raise HTTPException(status_code=400, detail="The deadline has to be in the future")
    vote.closes_at = closes
    session.add(vote)
    session.commit()
    return _vote_out(session, _open_vote(session), me)


@router.post("/nominations", response_model=VoteOut, status_code=201)
def nominate(
    payload: VoteNominateIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> VoteOut:
    maybe_auto_close(session)
    vote = _open_vote(session)
    if vote is None:
        vote = NextUpVote()
        session.add(vote)
        session.flush()
    if _nomination_count(session, vote.id or 0) >= NOMINATION_LIMIT:
        raise HTTPException(status_code=400, detail="This vote already has enough books")
    book = upsert_book(
        session,
        ol_work_key=payload.ol_work_key,
        title=payload.title,
        authors=payload.authors,
        cover_id=payload.cover_id,
        year=payload.year,
        cover_image_url=payload.cover_url,
    )
    pick = current_pick(session)
    if pick is not None and pick.book_id == book.id:
        raise HTTPException(status_code=400, detail="That book is already the club pick")
    existing = session.exec(
        select(NextUpNomination).where(
            NextUpNomination.vote_id == vote.id,
            NextUpNomination.book_id == book.id,
        )
    ).first()
    if existing is not None:
        raise HTTPException(status_code=400, detail="That book is already nominated")
    session.add(
        NextUpNomination(
            vote_id=vote.id or 0,
            book_id=book.id or 0,
            nominated_by_id=me.id or 0,
        )
    )
    activity.record(session, me, "nominated", book=book)
    session.commit()
    return _vote_out(session, _open_vote(session), me)


@router.post("/cast", response_model=VoteOut)
def cast_vote(
    payload: VoteCastIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> VoteOut:
    maybe_auto_close(session)
    nomination = _load_open_nomination(session, payload.nomination_id)
    vote_id = nomination.vote_id
    ballot = session.exec(
        select(NextUpBallot).where(
            NextUpBallot.vote_id == vote_id,
            NextUpBallot.user_id == me.id,
        )
    ).first()
    if ballot is None:
        session.add(
            NextUpBallot(
                vote_id=vote_id,
                nomination_id=nomination.id or 0,
                user_id=me.id or 0,
            )
        )
        activity.record(session, me, "voted", book=nomination.book)
    else:
        ballot.nomination_id = nomination.id or 0
        session.add(ballot)
    session.commit()
    return _vote_out(session, _open_vote(session), me)


@router.post("/apply", response_model=VoteApplyOut)
def apply_winner(
    payload: VoteApplyIn,
    request: Request,
    background: BackgroundTasks,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> VoteApplyOut:
    nomination = _load_open_nomination(session, payload.nomination_id)
    if nomination.book is None:
        raise HTTPException(status_code=500, detail="Nomination is missing its book")
    vote = _open_vote(session)
    if vote is None:
        raise HTTPException(status_code=404, detail="No open next-up vote")
    book = nomination.book
    pick = apply_club_pick(
        session,
        me,
        ClubPickSetIn(
            ol_work_key=book.ol_work_key,
            title=book.title,
            authors=book.authors,
            cover_id=book.cover_id,
            year=book.year,
            meeting_at=payload.meeting_at,
        ),
        _timezone(),
    )
    vote.ended_at = utcnow()
    vote.winner_nomination_id = nomination.id
    session.add(vote)
    activity.record(session, me, "vote_closed", book=book, pick=pick, votes=len(nomination.ballots))
    session.commit()
    from app.routers.pick import notify_new_pick

    notify_new_pick(background, request.app.state.engine, session, pick, me)
    return VoteApplyOut(
        pick=pick_out(session, pick, me, _timezone()),
        vote=_vote_out(session, None, me),
    )


@router.get("/suggestions", response_model=VoteSuggestionsOut)
def suggestions(
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> VoteSuggestionsOut:
    """Candidates for the next vote: shared TBR first, then subject affinity with past picks."""
    excluded: set[int] = set()
    pick = current_pick(session)
    if pick is not None:
        excluded.add(pick.book_id)
    vote = _open_vote(session)
    if vote is not None:
        for row in _nominations(session, vote):
            excluded.add(row.book_id)
    past_ids = {
        row.book_id
        for row in session.exec(select(ClubPick).where(ClubPick.ended_at.is_not(None))).all()
    }
    excluded |= past_ids

    liked_subjects: dict[str, int] = {}
    if past_ids:
        for book in session.exec(select(Book).where(col(Book.id).in_(list(past_ids)))).all():
            for subject in subjects_from_json(book.subjects):
                key = subject.lower()
                liked_subjects[key] = liked_subjects.get(key, 0) + 1

    wants = session.exec(
        select(ShelfEntry)
        .where(ShelfEntry.status == ShelfStatus.want_to_read)
        .options(selectinload(ShelfEntry.book), selectinload(ShelfEntry.user))
    ).all()
    by_book: dict[int, list[ShelfEntry]] = {}
    for entry in wants:
        if entry.book is None or entry.book_id in excluded:
            continue
        by_book.setdefault(entry.book_id, []).append(entry)

    scored: list[VoteSuggestionOut] = []
    for book_id, entries in by_book.items():
        book = entries[0].book
        if book is None:
            continue
        names = sorted(e.user.username for e in entries if e.user is not None)
        score = 0
        reasons: list[str] = []
        if len(names) >= 2:
            score += 3 * len(names)
            reasons.append(f"On {len(names)} shelves: {', '.join(names)}")
        elif names:
            score += 1
            reasons.append(f"{names[0]} wants to read it")
        matched = [
            subject
            for subject in subjects_from_json(book.subjects)
            if subject.lower() in liked_subjects
        ]
        if matched:
            score += len(matched)
            reasons.append("Like past picks: " + ", ".join(matched[:3]))
        if book.ol_rating and book.ol_rating >= 4.0:
            score += 1
            reasons.append(f"Open Library rating {book.ol_rating:.1f}")
        scored.append(VoteSuggestionOut(book=book_out(book), score=score, reasons=reasons))
    scored.sort(key=lambda row: (-row.score, row.book.title.lower()))
    return VoteSuggestionsOut(items=scored[:SUGGESTION_LIMIT])

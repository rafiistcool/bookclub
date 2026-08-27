from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy.orm import selectinload
from sqlmodel import Session, select

from app.config import get_settings
from app.deps import get_current_user, get_session
from app.models import (
    NOMINATION_LIMIT,
    NextUpBallot,
    NextUpNomination,
    NextUpVote,
    User,
    utcnow,
)
from app.pick_ops import apply_club_pick, current_pick, pick_out
from app.schemas import (
    ClubPickSetIn,
    VoteApplyIn,
    VoteApplyOut,
    VoteCastIn,
    VoteNominateIn,
    VoteNominationOut,
    VoteOut,
)
from app.serialize import book_out
from app.shelf_ops import upsert_book
from app.timezone import resolved_timezone

router = APIRouter(prefix="/api/vote", tags=["vote"])


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


def _vote_out(session: Session, vote: NextUpVote | None, viewer: User) -> VoteOut:
    tz_name = _timezone()
    if vote is None:
        return VoteOut(
            vote_id=None,
            nominations=[],
            my_vote_id=None,
            nomination_limit=NOMINATION_LIMIT,
            can_nominate=True,
            timezone=tz_name,
        )
    rows = session.exec(
        select(NextUpNomination)
        .where(NextUpNomination.vote_id == vote.id)
        .options(
            selectinload(NextUpNomination.book),
            selectinload(NextUpNomination.nominated_by),
            selectinload(NextUpNomination.ballots).selectinload(NextUpBallot.user),
        )
        .order_by(NextUpNomination.created_at, NextUpNomination.id)
    ).all()
    nominations: list[VoteNominationOut] = []
    my_vote_id = None
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
    return VoteOut(
        vote_id=vote.id,
        nominations=nominations,
        my_vote_id=my_vote_id,
        nomination_limit=NOMINATION_LIMIT,
        can_nominate=len(nominations) < NOMINATION_LIMIT,
        timezone=tz_name,
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
        .options(selectinload(NextUpNomination.book))
    ).first()
    if row is None:
        raise HTTPException(status_code=404, detail="No nomination with that id")
    return row


@router.get("", response_model=VoteOut)
def get_vote(
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> VoteOut:
    return _vote_out(session, _open_vote(session), me)


@router.post("/nominations", response_model=VoteOut, status_code=201)
def nominate(
    payload: VoteNominateIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> VoteOut:
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
    session.commit()
    return _vote_out(session, _open_vote(session), me)


@router.post("/cast", response_model=VoteOut)
def cast_vote(
    payload: VoteCastIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> VoteOut:
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
    else:
        ballot.nomination_id = nomination.id or 0
        session.add(ballot)
    session.commit()
    return _vote_out(session, _open_vote(session), me)


@router.post("/apply", response_model=VoteApplyOut)
def apply_winner(
    payload: VoteApplyIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> VoteApplyOut:
    nomination = _load_open_nomination(session, payload.nomination_id)
    if nomination.book is None:
        raise HTTPException(status_code=500, detail="Nomination is missing its book")
    book = nomination.book
    nomination_id = nomination.id
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
    vote = _open_vote(session)
    if vote is None:
        raise HTTPException(status_code=404, detail="No open next-up vote")
    vote.ended_at = utcnow()
    vote.winner_nomination_id = nomination_id
    session.add(vote)
    session.commit()
    return VoteApplyOut(
        pick=pick_out(session, pick, me, _timezone()),
        vote=_vote_out(session, None, me),
    )

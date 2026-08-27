from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy.orm import selectinload
from sqlmodel import Session, col, select

from app.config import get_settings
from app.deps import get_current_user, get_session
from app.models import ClubPick, ClubPickPost, User, utcnow
from app.pick_ops import apply_club_pick, current_pick, pick_out
from app.schemas import (
    ClubPickCurrentOut,
    ClubPickHistoryOut,
    ClubPickOut,
    ClubPickSetIn,
    PickPostIn,
    PickPostListOut,
    PickPostOut,
)
from app.timezone import meeting_label, resolved_timezone

router = APIRouter(prefix="/api/pick", tags=["pick"])


def _timezone() -> str:
    return resolved_timezone(get_settings().bookclub_tz)


def _current_pick(session: Session) -> ClubPick | None:
    return current_pick(session)


def _pick_out(session: Session, pick: ClubPick, viewer: User) -> ClubPickOut:
    return pick_out(session, pick, viewer, _timezone())


@router.get("", response_model=ClubPickCurrentOut)
def get_club_pick(
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> ClubPickCurrentOut:
    pick = _current_pick(session)
    if pick is None:
        return ClubPickCurrentOut(pick=None, timezone=_timezone())
    return ClubPickCurrentOut(pick=_pick_out(session, pick, me), timezone=_timezone())


@router.get("/history", response_model=ClubPickHistoryOut)
def club_pick_history(
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> ClubPickHistoryOut:
    rows = session.exec(
        select(ClubPick)
        .where(ClubPick.ended_at.is_not(None))
        .options(selectinload(ClubPick.book), selectinload(ClubPick.set_by))
        .order_by(col(ClubPick.ended_at).desc(), ClubPick.id.desc())
    ).all()
    return ClubPickHistoryOut(
        items=[_pick_out(session, row, me) for row in rows],
        timezone=_timezone(),
    )


@router.put("", response_model=ClubPickOut)
def set_club_pick(
    payload: ClubPickSetIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> ClubPickOut:
    loaded = apply_club_pick(session, me, payload, _timezone())
    return _pick_out(session, loaded, me)


@router.delete("", response_model=ClubPickCurrentOut)
def clear_club_pick(
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> ClubPickCurrentOut:
    current = _current_pick(session)
    if current is None:
        return ClubPickCurrentOut(pick=None, timezone=_timezone())
    current.ended_at = utcnow()
    session.add(current)
    session.commit()
    return ClubPickCurrentOut(pick=None, timezone=_timezone())


def _load_pick(session: Session, pick_id: int) -> ClubPick | None:
    return session.exec(
        select(ClubPick)
        .where(ClubPick.id == pick_id)
        .options(selectinload(ClubPick.book), selectinload(ClubPick.set_by))
    ).first()


def _post_out(post: ClubPickPost, tz_name: str) -> PickPostOut:
    author = post.author.username if post.author is not None else "unknown"
    return PickPostOut(
        id=post.id or 0,
        author=author,
        body=post.body,
        created_at=post.created_at,
        created_label=meeting_label(post.created_at, tz_name) or "",
    )


def _posts_for(session: Session, pick: ClubPick) -> PickPostListOut:
    tz_name = _timezone()
    rows = session.exec(
        select(ClubPickPost)
        .where(ClubPickPost.pick_id == pick.id)
        .options(selectinload(ClubPickPost.author))
        .order_by(ClubPickPost.created_at, ClubPickPost.id)
    ).all()
    return PickPostListOut(
        pick_id=pick.id or 0,
        can_post=pick.ended_at is None,
        items=[_post_out(row, tz_name) for row in rows],
        timezone=tz_name,
    )


@router.get("/posts", response_model=PickPostListOut)
def current_pick_posts(
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> PickPostListOut:
    pick = _current_pick(session)
    if pick is None:
        raise HTTPException(status_code=404, detail="No club pick yet")
    return _posts_for(session, pick)


@router.post("/posts", response_model=PickPostOut, status_code=201)
def add_current_pick_post(
    payload: PickPostIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> PickPostOut:
    pick = _current_pick(session)
    if pick is None:
        raise HTTPException(status_code=404, detail="No club pick yet")
    post = ClubPickPost(pick_id=pick.id or 0, author_id=me.id or 0, body=payload.body)
    session.add(post)
    session.commit()
    loaded = session.exec(
        select(ClubPickPost)
        .where(ClubPickPost.id == post.id)
        .options(selectinload(ClubPickPost.author))
    ).first()
    if loaded is None:
        raise HTTPException(status_code=500, detail="Could not save that note")
    return _post_out(loaded, _timezone())


@router.get("/{pick_id}/posts", response_model=PickPostListOut)
def pick_posts(
    pick_id: int,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> PickPostListOut:
    pick = _load_pick(session, pick_id)
    if pick is None:
        raise HTTPException(status_code=404, detail="No club pick with that id")
    return _posts_for(session, pick)


@router.post("/{pick_id}/posts", response_model=PickPostOut, status_code=201)
def add_pick_post(
    pick_id: int,
    payload: PickPostIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> PickPostOut:
    pick = _load_pick(session, pick_id)
    if pick is None:
        raise HTTPException(status_code=404, detail="No club pick with that id")
    if pick.ended_at is not None:
        raise HTTPException(status_code=400, detail="That pick is closed")
    post = ClubPickPost(pick_id=pick.id or 0, author_id=me.id or 0, body=payload.body)
    session.add(post)
    session.commit()
    loaded = session.exec(
        select(ClubPickPost)
        .where(ClubPickPost.id == post.id)
        .options(selectinload(ClubPickPost.author))
    ).first()
    if loaded is None:
        raise HTTPException(status_code=500, detail="Could not save that note")
    return _post_out(loaded, _timezone())


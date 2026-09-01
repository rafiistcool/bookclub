from datetime import datetime

from fastapi import APIRouter, Depends, Query
from pydantic import BaseModel
from sqlalchemy.orm import selectinload
from sqlmodel import Session, col, select

from app.activity import payload_of
from app.config import get_settings
from app.deps import get_current_user, get_session
from app.models import Event, User
from app.schemas import BookOut
from app.serialize import book_out
from app.timezone import meeting_label, resolved_timezone

router = APIRouter(prefix="/api/activity", tags=["activity"])


class ActivityItem(BaseModel):
    id: int
    kind: str
    actor: str
    mine: bool
    book: BookOut | None = None
    pick_id: int | None = None
    payload: dict
    created_at: datetime
    created_label: str


class ActivityOut(BaseModel):
    items: list[ActivityItem]
    has_more: bool
    timezone: str


@router.get("", response_model=ActivityOut)
def activity_feed(
    before: int | None = Query(default=None, ge=1),
    limit: int = Query(default=30, ge=1, le=100),
    username: str | None = Query(default=None),
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> ActivityOut:
    tz_name = resolved_timezone(get_settings().bookclub_tz)
    query = (
        select(Event)
        .options(selectinload(Event.actor), selectinload(Event.book))
        .order_by(col(Event.id).desc())
        .limit(limit + 1)
    )
    if before is not None:
        query = query.where(Event.id < before)
    if username:
        owner = session.exec(select(User).where(User.username == username.lower())).first()
        if owner is None:
            return ActivityOut(items=[], has_more=False, timezone=tz_name)
        query = query.where(Event.actor_id == owner.id)
    rows = session.exec(query).all()
    has_more = len(rows) > limit
    items = [
        ActivityItem(
            id=row.id or 0,
            kind=row.kind,
            actor=row.actor.username if row.actor is not None else "unknown",
            mine=row.actor_id == me.id,
            book=book_out(row.book) if row.book is not None else None,
            pick_id=row.pick_id,
            payload=payload_of(row),
            created_at=row.created_at,
            created_label=meeting_label(row.created_at, tz_name) or "",
        )
        for row in rows[:limit]
    ]
    return ActivityOut(items=items, has_more=has_more, timezone=tz_name)

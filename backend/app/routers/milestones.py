"""Reading schedule for the current club pick."""

from datetime import datetime

from fastapi import APIRouter, Depends, HTTPException
from pydantic import BaseModel, Field, field_validator
from sqlmodel import Session, select

from app import activity
from app.config import get_settings
from app.deps import get_current_user, get_session
from app.models import ClubPick, ClubPickPost, PickMilestone, User
from app.pick_ops import current_pick
from app.timezone import meeting_label, meeting_local, parse_meeting, resolved_timezone

router = APIRouter(prefix="/api/pick", tags=["milestones"])


class MilestoneIn(BaseModel):
    title: str
    chapter_from: int | None = Field(default=None, ge=0)
    chapter_to: int | None = Field(default=None, ge=0)
    due_at: str | None = None

    @field_validator("title")
    @classmethod
    def title_ok(cls, value: str) -> str:
        value = value.strip()
        if not value:
            raise ValueError("Give the milestone a title")
        if len(value) > 120:
            raise ValueError("Keep the title to 120 characters")
        return value


class MilestoneOut(BaseModel):
    id: int
    pick_id: int
    title: str
    chapter_from: int | None
    chapter_to: int | None
    due_at: datetime | None
    due_local: str | None
    due_label: str | None
    position: int
    note_count: int
    passed: bool


class MilestoneListOut(BaseModel):
    pick_id: int
    items: list[MilestoneOut]
    timezone: str


def _timezone() -> str:
    return resolved_timezone(get_settings().bookclub_tz)


def _out(session: Session, row: PickMilestone) -> MilestoneOut:
    tz_name = _timezone()
    notes = len(
        session.exec(select(ClubPickPost).where(ClubPickPost.milestone_id == row.id)).all()
    )
    from app.models import utcnow

    due = row.due_at
    if due is not None and due.tzinfo is None:
        from datetime import timezone

        due = due.replace(tzinfo=timezone.utc)
    return MilestoneOut(
        id=row.id or 0,
        pick_id=row.pick_id,
        title=row.title,
        chapter_from=row.chapter_from,
        chapter_to=row.chapter_to,
        due_at=due,
        due_local=meeting_local(row.due_at, tz_name),
        due_label=meeting_label(row.due_at, tz_name),
        position=row.position,
        note_count=notes,
        passed=bool(due and due <= utcnow()),
    )


def _rows(session: Session, pick: ClubPick) -> list[PickMilestone]:
    return session.exec(
        select(PickMilestone)
        .where(PickMilestone.pick_id == pick.id)
        .order_by(PickMilestone.position, PickMilestone.id)
    ).all()


def _pick_or_404(session: Session, pick_id: int) -> ClubPick:
    pick = session.get(ClubPick, pick_id)
    if pick is None:
        raise HTTPException(status_code=404, detail="No club pick with that id")
    return pick


@router.get("/{pick_id}/milestones", response_model=MilestoneListOut)
def list_milestones(
    pick_id: int,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> MilestoneListOut:
    pick = _pick_or_404(session, pick_id)
    return MilestoneListOut(
        pick_id=pick.id or 0,
        items=[_out(session, row) for row in _rows(session, pick)],
        timezone=_timezone(),
    )


@router.post("/{pick_id}/milestones", response_model=MilestoneOut, status_code=201)
def add_milestone(
    pick_id: int,
    payload: MilestoneIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> MilestoneOut:
    pick = _pick_or_404(session, pick_id)
    if pick.ended_at is not None:
        raise HTTPException(status_code=400, detail="That pick is closed")
    if (
        payload.chapter_from is not None
        and payload.chapter_to is not None
        and payload.chapter_to < payload.chapter_from
    ):
        raise HTTPException(status_code=400, detail="Chapter range is backwards")
    try:
        due = parse_meeting(payload.due_at, _timezone())
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    existing = _rows(session, pick)
    row = PickMilestone(
        pick_id=pick.id or 0,
        title=payload.title,
        chapter_from=payload.chapter_from,
        chapter_to=payload.chapter_to,
        due_at=due,
        position=len(existing),
        created_by_id=me.id or 0,
    )
    session.add(row)
    session.flush()
    current = current_pick(session)
    activity.record(
        session,
        me,
        "milestone_added",
        book=pick.book if pick.book else None,
        pick=pick,
        title=payload.title,
        due_at=due.isoformat() if due else None,
    )
    session.commit()
    session.refresh(row)
    return _out(session, row)


@router.patch("/{pick_id}/milestones/{milestone_id}", response_model=MilestoneOut)
def edit_milestone(
    pick_id: int,
    milestone_id: int,
    payload: MilestoneIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> MilestoneOut:
    _pick_or_404(session, pick_id)
    row = session.get(PickMilestone, milestone_id)
    if row is None or row.pick_id != pick_id:
        raise HTTPException(status_code=404, detail="No milestone with that id")
    try:
        due = parse_meeting(payload.due_at, _timezone())
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    row.title = payload.title
    row.chapter_from = payload.chapter_from
    row.chapter_to = payload.chapter_to
    row.due_at = due
    session.add(row)
    session.commit()
    session.refresh(row)
    return _out(session, row)


@router.delete("/{pick_id}/milestones/{milestone_id}", status_code=204)
def delete_milestone(
    pick_id: int,
    milestone_id: int,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> None:
    _pick_or_404(session, pick_id)
    row = session.get(PickMilestone, milestone_id)
    if row is None or row.pick_id != pick_id:
        raise HTTPException(status_code=404, detail="No milestone with that id")
    for post in session.exec(
        select(ClubPickPost).where(ClubPickPost.milestone_id == milestone_id)
    ).all():
        post.milestone_id = None
        session.add(post)
    session.delete(row)
    session.flush()
    pick = session.get(ClubPick, pick_id)
    if pick is not None:
        for index, other in enumerate(_rows(session, pick)):
            other.position = index
    session.commit()

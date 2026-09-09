from fastapi import HTTPException
from sqlalchemy.orm import selectinload
from sqlmodel import Session, select

from app import activity
from app.models import ClubPick, ShelfEntry, ShelfStatus, User, utcnow
from app.schemas import ClubPickOut, ClubPickReader, ClubPickSetIn
from app.serialize import avatar_url_for, book_out
from app.shelf_ops import upsert_book
from app.timezone import meeting_label, meeting_local, parse_meeting


def current_pick(session: Session) -> ClubPick | None:
    return session.exec(
        select(ClubPick)
        .where(ClubPick.ended_at.is_(None))
        .options(selectinload(ClubPick.book), selectinload(ClubPick.set_by))
        .order_by(ClubPick.id.desc())
    ).first()


def apply_club_pick(
    session: Session,
    me: User,
    payload: ClubPickSetIn,
    tz_name: str,
) -> ClubPick:
    try:
        meeting = parse_meeting(payload.meeting_at, tz_name)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    book = upsert_book(
        session,
        ol_work_key=payload.ol_work_key,
        title=payload.title,
        authors=payload.authors,
        cover_id=payload.cover_id,
        year=payload.year,
        cover_image_url=payload.cover_url,
    )
    current = current_pick(session)
    if current is not None and current.book_id == book.id:
        current.note = payload.note
        current.set_by_id = me.id or 0
        if "meeting_at" in payload.model_fields_set:
            current.meeting_at = meeting
        session.add(current)
        session.commit()
        loaded = current_pick(session)
        if loaded is None:
            raise HTTPException(status_code=500, detail="Could not save the club pick")
        return loaded

    if current is not None:
        current.ended_at = utcnow()
        session.add(current)

    pick = ClubPick(
        book_id=book.id or 0,
        set_by_id=me.id or 0,
        note=payload.note,
        meeting_at=meeting,
    )
    session.add(pick)
    session.flush()
    activity.record(
        session,
        me,
        "pick_set",
        book=book,
        pick=pick,
        meeting_at=meeting.isoformat() if meeting else None,
    )
    session.commit()
    loaded = current_pick(session)
    if loaded is None:
        raise HTTPException(status_code=500, detail="Could not save the club pick")
    return loaded


def pick_out(session: Session, pick: ClubPick, viewer: User, tz_name: str) -> ClubPickOut:
    if pick.book is None:
        raise HTTPException(status_code=500, detail="Club pick is missing its book")
    setter = pick.set_by.username if pick.set_by is not None else "unknown"
    entries = session.exec(
        select(ShelfEntry)
        .where(ShelfEntry.book_id == pick.book_id)
        .options(selectinload(ShelfEntry.user))
        .order_by(ShelfEntry.status, ShelfEntry.id)
    ).all()
    readers: list[ClubPickReader] = []
    on_shelf = None
    shelf_id = None
    for entry in entries:
        if entry.user is None:
            continue
        readers.append(
            ClubPickReader(
                username=entry.user.username,
                status=entry.status,
                rating=entry.rating,
                take=entry.take,
                dnf_reason=entry.dnf_reason,
                progress=entry.progress,
                avatar_url=avatar_url_for(entry.user),
            )
        )
        if entry.user_id == viewer.id:
            on_shelf = entry.status
            shelf_id = entry.id
    readers.sort(key=lambda row: (row.username != viewer.username, row.username))
    current = pick.ended_at is None
    reading = [
        row.username for row in readers if row.status == ShelfStatus.currently_reading
    ]
    finished = [row.username for row in readers if row.status == ShelfStatus.finished]
    return ClubPickOut(
        id=pick.id or 0,
        book=book_out(pick.book),
        set_by=setter,
        note=pick.note,
        started_at=pick.started_at,
        ended_at=pick.ended_at,
        meeting_at=pick.meeting_at,
        meeting_local=meeting_local(pick.meeting_at, tz_name),
        meeting_label=meeting_label(pick.meeting_at, tz_name),
        on_shelf=on_shelf,
        shelf_id=shelf_id,
        readers=readers if current else [],
        reading=reading if current else [],
        finished=finished if current else [],
    )

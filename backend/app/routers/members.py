from fastapi import APIRouter, Depends
from sqlalchemy.orm import selectinload
from sqlmodel import Session, col, select

from app.deps import get_current_user, get_session
from app.models import ShelfEntry, ShelfStatus, User
from app.schemas import MemberOut, ReadingPreview

router = APIRouter(prefix="/api/members", tags=["members"])


@router.get("", response_model=list[MemberOut])
def list_members(
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> list[MemberOut]:
    others = session.exec(
        select(User).where(col(User.id) != me.id).order_by(User.username)
    ).all()
    out: list[MemberOut] = []
    for member in others:
        reading = session.exec(
            select(ShelfEntry)
            .where(
                ShelfEntry.user_id == member.id,
                ShelfEntry.status == ShelfStatus.currently_reading,
            )
            .options(selectinload(ShelfEntry.book))
            .order_by(ShelfEntry.position, ShelfEntry.id)
        ).all()
        preview: list[ReadingPreview] = []
        for entry in reading[:3]:
            if entry.book is None:
                continue
            preview.append(
                ReadingPreview(title=entry.book.title, cover_id=entry.book.cover_id)
            )
        out.append(
            MemberOut(
                username=member.username,
                currently_reading_count=len(reading),
                currently_reading_preview=preview,
            )
        )
    return out

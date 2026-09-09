from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import Response
from sqlalchemy.orm import selectinload
from sqlmodel import Session, col, select

from app.avatars import AVATAR_MIME
from app.deps import get_current_user, get_session
from app.models import ShelfEntry, ShelfStatus, User
from app.schemas import MemberOut, ReadingPreview
from app.serialize import avatar_url_for, book_cover_url

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
                ReadingPreview(
                    title=entry.book.title,
                    cover_id=entry.book.cover_id,
                    cover_url=book_cover_url(entry.book),
                )
            )
        out.append(
            MemberOut(
                username=member.username,
                currently_reading_count=len(reading),
                currently_reading_preview=preview,
                avatar_url=avatar_url_for(member),
            )
        )
    return out


@router.get("/{username}/avatar")
def member_avatar(
    username: str,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> Response:
    member = session.exec(select(User).where(User.username == username.lower())).first()
    if member is None or not member.avatar:
        raise HTTPException(status_code=404, detail="No profile picture")
    headers = {"Cache-Control": "private, max-age=86400"}
    if member.avatar_updated_at is not None:
        headers["ETag"] = f'"{int(member.avatar_updated_at.timestamp())}"'
    return Response(
        content=bytes(member.avatar),
        media_type=member.avatar_mime or AVATAR_MIME,
        headers=headers,
    )

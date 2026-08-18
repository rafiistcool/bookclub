from fastapi import APIRouter, Depends
from sqlmodel import Session, col, select

from app.deps import get_current_user, get_session
from app.models import Invite, User
from app.schemas import InviteCreated, InviteOut
from app.security import generate_invite_code

router = APIRouter(prefix="/api/invites", tags=["invites"])


@router.post("", response_model=InviteCreated, status_code=201)
def create_invite(
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> InviteCreated:
    code = generate_invite_code()
    while session.exec(select(Invite).where(Invite.code == code)).first() is not None:
        code = generate_invite_code()
    invite = Invite(code=code, created_by_id=user.id)
    session.add(invite)
    session.commit()
    return InviteCreated(code=code)


@router.get("", response_model=list[InviteOut])
def list_invites(
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> list[InviteOut]:
    rows = session.exec(
        select(Invite)
        .where(Invite.created_by_id == user.id)
        .order_by(Invite.created_at.desc())
    ).all()
    used_ids = {row.used_by_id for row in rows if row.used_by_id}
    usernames: dict[int, str] = {}
    if used_ids:
        for member in session.exec(select(User).where(col(User.id).in_(used_ids))).all():
            if member.id is not None:
                usernames[member.id] = member.username
    return [
        InviteOut(
            code=row.code,
            used=row.used_by_id is not None,
            used_by=usernames.get(row.used_by_id) if row.used_by_id else None,
            created_at=row.created_at,
        )
        for row in rows
    ]

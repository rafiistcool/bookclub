from fastapi import APIRouter, Depends, HTTPException, Request
from sqlalchemy import update
from sqlmodel import Session, select

from app.deps import get_current_user, get_session
from app.models import Invite, User, utcnow
from app.schemas import LoginIn, RegisterIn, UserOut
from app.security import (
    clear_failures,
    hash_password,
    normalize_invite_code,
    record_failure,
    too_many_failures,
    verify_password,
)

router = APIRouter(prefix="/api/auth", tags=["auth"])


def _set_session(request: Request, user: User) -> None:
    request.session.clear()
    request.session["user_id"] = user.id
    request.session["username"] = user.username


@router.post("/register", response_model=UserOut, status_code=201)
def register(
    payload: RegisterIn,
    request: Request,
    session: Session = Depends(get_session),
) -> User:
    taken = session.exec(select(User).where(User.username == payload.username)).first()
    if taken is not None:
        raise HTTPException(status_code=409, detail="That username is already taken")

    code = normalize_invite_code(payload.invite_code)
    user = User(username=payload.username, password_hash=hash_password(payload.password))
    session.add(user)
    session.flush()
    consumed = session.execute(
        update(Invite)
        .where(Invite.code == code, Invite.used_by_id.is_(None))
        .values(used_by_id=user.id, used_at=utcnow())
    )
    if consumed.rowcount != 1:
        session.rollback()
        raise HTTPException(status_code=400, detail="That invite code is not valid")
    session.commit()
    session.refresh(user)
    _set_session(request, user)
    return user


@router.post("/login", status_code=204)
def login(
    payload: LoginIn,
    request: Request,
    session: Session = Depends(get_session),
) -> None:
    if too_many_failures(payload.username):
        raise HTTPException(
            status_code=429, detail="Too many attempts. Wait a few minutes."
        )
    user = session.exec(select(User).where(User.username == payload.username)).first()
    if user is None or not verify_password(payload.password, user.password_hash):
        record_failure(payload.username)
        raise HTTPException(status_code=401, detail="Wrong username or password")
    clear_failures(payload.username)
    _set_session(request, user)


@router.post("/logout", status_code=204)
def logout(request: Request) -> None:
    request.session.clear()


@router.get("/me", response_model=UserOut)
def me(user: User = Depends(get_current_user)) -> User:
    return user

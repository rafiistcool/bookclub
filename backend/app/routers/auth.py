from fastapi import APIRouter, Depends, HTTPException, Request
from starlette.datastructures import UploadFile
from sqlalchemy import update
from sqlmodel import Session, select

from app import activity
from app.avatars import (
    AVATAR_MIME,
    MAX_UPLOAD_BYTES,
    TOO_LARGE,
    oversize_content_length,
    process_avatar,
)
from app.deps import get_current_user, get_session
from app.models import Invite, User, utcnow
from app.schemas import (
    LoginIn,
    NotificationPrefsIn,
    NotificationPrefsOut,
    PasswordChangeIn,
    PreferencesIn,
    RegisterIn,
    UserOut,
)
from app.serialize import user_out
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
) -> UserOut:
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
    activity.record(session, user, "member_joined")
    session.commit()
    session.refresh(user)
    _set_session(request, user)
    return user_out(user)


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
def me(user: User = Depends(get_current_user)) -> UserOut:
    return user_out(user)


@router.patch("/me/preferences", response_model=UserOut)
def update_preferences(
    payload: PreferencesIn,
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> UserOut:
    if payload.theme is not None:
        user.theme = payload.theme
    if payload.color_mode is not None:
        user.color_mode = payload.color_mode
    if payload.locale is not None:
        user.locale = payload.locale
    session.add(user)
    session.commit()
    session.refresh(user)
    return user_out(user)


@router.put("/me/avatar", response_model=UserOut)
async def upload_avatar(
    request: Request,
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> UserOut:
    if oversize_content_length(request.headers.get("content-length")):
        raise HTTPException(status_code=413, detail=TOO_LARGE)
    form = await request.form()
    upload = form.get("file")
    if not isinstance(upload, UploadFile):
        raise HTTPException(status_code=400, detail="Use a JPEG, PNG, WebP, or GIF")
    raw = await upload.read(MAX_UPLOAD_BYTES + 1)
    try:
        jpeg = process_avatar(raw)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    user.avatar = jpeg
    user.avatar_mime = AVATAR_MIME
    user.avatar_updated_at = utcnow()
    session.add(user)
    session.commit()
    session.refresh(user)
    return user_out(user)


@router.delete("/me/avatar", response_model=UserOut)
def delete_avatar(
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> UserOut:
    user.avatar = None
    user.avatar_mime = None
    user.avatar_updated_at = None
    session.add(user)
    session.commit()
    session.refresh(user)
    return user_out(user)


@router.patch("/password", status_code=204)
def change_password(
    payload: PasswordChangeIn,
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> None:
    if too_many_failures(user.username):
        raise HTTPException(
            status_code=429, detail="Too many attempts. Wait a few minutes."
        )
    if not verify_password(payload.current_password, user.password_hash):
        record_failure(user.username)
        raise HTTPException(status_code=400, detail="Current password is wrong")
    if payload.current_password == payload.new_password:
        raise HTTPException(status_code=400, detail="Pick a different password")
    clear_failures(user.username)
    user.password_hash = hash_password(payload.new_password)
    session.add(user)
    session.commit()


@router.get("/notifications", response_model=NotificationPrefsOut)
def notification_prefs(user: User = Depends(get_current_user)) -> NotificationPrefsOut:
    return NotificationPrefsOut(
        notify_meeting=user.notify_meeting,
        notify_pick=user.notify_pick,
        notify_note=user.notify_note,
    )


@router.patch("/notifications", response_model=NotificationPrefsOut)
def update_notification_prefs(
    payload: NotificationPrefsIn,
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> NotificationPrefsOut:
    for field in ("notify_meeting", "notify_pick", "notify_note"):
        value = getattr(payload, field)
        if value is not None:
            setattr(user, field, value)
    session.add(user)
    session.commit()
    session.refresh(user)
    return NotificationPrefsOut(
        notify_meeting=user.notify_meeting,
        notify_pick=user.notify_pick,
        notify_note=user.notify_note,
    )

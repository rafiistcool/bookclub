from fastapi import Depends, HTTPException, Request
from sqlalchemy.orm import defer
from sqlmodel import Session

from app.models import User


def get_session(request: Request):
    with Session(request.app.state.engine) as session:
        yield session


def get_current_user(
    request: Request, session: Session = Depends(get_session)
) -> User:
    user_id = request.session.get("user_id")
    if not user_id:
        raise HTTPException(status_code=401, detail="Not signed in")
    user = session.get(User, user_id, options=[defer(User.avatar)])
    if user is None:
        request.session.clear()
        raise HTTPException(status_code=401, detail="Not signed in")
    return user

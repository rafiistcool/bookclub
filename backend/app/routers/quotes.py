from datetime import datetime

from fastapi import APIRouter, Depends, HTTPException, Query
from pydantic import BaseModel, Field, field_validator
from sqlalchemy.orm import selectinload
from sqlmodel import Session, col, select

from app import activity
from app.config import get_settings
from app.deps import get_current_user, get_session
from app.models import Book, Quote, User
from app.schemas import BookOut
from app.serialize import book_out
from app.shelf_ops import upsert_book
from app.timezone import meeting_label, resolved_timezone

router = APIRouter(prefix="/api/quotes", tags=["quotes"])


class QuoteIn(BaseModel):
    ol_work_key: str
    title: str = ""
    authors: str = ""
    cover_id: int | None = None
    cover_url: str | None = None
    year: int | None = None
    body: str
    page: int | None = Field(default=None, ge=0, le=20000)

    @field_validator("ol_work_key")
    @classmethod
    def work_key_ok(cls, value: str) -> str:
        value = value.strip()
        if not value.startswith("/works/"):
            raise ValueError("ol_work_key must be an Open Library work key")
        return value

    @field_validator("body")
    @classmethod
    def body_ok(cls, value: str) -> str:
        value = value.strip()
        if not value:
            raise ValueError("Paste the line you want to keep")
        if len(value) > 1000:
            raise ValueError("Keep it to 1000 characters")
        return value


class QuotePatchIn(BaseModel):
    body: str | None = None
    page: int | None = Field(default=None, ge=0, le=20000)

    @field_validator("body")
    @classmethod
    def body_ok(cls, value: str | None) -> str | None:
        if value is None:
            return None
        value = value.strip()
        if not value:
            raise ValueError("Paste the line you want to keep")
        if len(value) > 1000:
            raise ValueError("Keep it to 1000 characters")
        return value


class QuoteOut(BaseModel):
    id: int
    book: BookOut
    author: str
    mine: bool
    body: str
    page: int | None
    created_at: datetime
    created_label: str


class QuoteListOut(BaseModel):
    items: list[QuoteOut]


def _timezone() -> str:
    return resolved_timezone(get_settings().bookclub_tz)


def _out(row: Quote, viewer: User) -> QuoteOut:
    if row.book is None:
        raise HTTPException(status_code=500, detail="Quote is missing its book")
    return QuoteOut(
        id=row.id or 0,
        book=book_out(row.book),
        author=row.user.username if row.user is not None else "unknown",
        mine=row.user_id == viewer.id,
        body=row.body,
        page=row.page,
        created_at=row.created_at,
        created_label=meeting_label(row.created_at, _timezone()) or "",
    )


def _query():
    return select(Quote).options(selectinload(Quote.book), selectinload(Quote.user))


@router.get("", response_model=QuoteListOut)
def list_quotes(
    work: str | None = Query(default=None),
    username: str | None = Query(default=None),
    limit: int = Query(default=100, ge=1, le=500),
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> QuoteListOut:
    query = _query().order_by(col(Quote.id).desc()).limit(limit)
    if work:
        book = session.exec(select(Book).where(Book.ol_work_key == work)).first()
        if book is None:
            return QuoteListOut(items=[])
        query = query.where(Quote.book_id == book.id)
    if username:
        owner = session.exec(select(User).where(User.username == username.lower())).first()
        if owner is None:
            return QuoteListOut(items=[])
        query = query.where(Quote.user_id == owner.id)
    rows = session.exec(query).all()
    return QuoteListOut(items=[_out(row, me) for row in rows])


@router.post("", response_model=QuoteOut, status_code=201)
def add_quote(
    payload: QuoteIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> QuoteOut:
    book = session.exec(select(Book).where(Book.ol_work_key == payload.ol_work_key)).first()
    if book is None:
        if not payload.title.strip():
            raise HTTPException(status_code=400, detail="title is required for a new book")
        book = upsert_book(
            session,
            ol_work_key=payload.ol_work_key,
            title=payload.title.strip(),
            authors=payload.authors,
            cover_id=payload.cover_id,
            year=payload.year,
            cover_image_url=payload.cover_url,
        )
    row = Quote(user_id=me.id or 0, book_id=book.id or 0, body=payload.body, page=payload.page)
    session.add(row)
    session.flush()
    activity.record(session, me, "quote_added", book=book, page=payload.page, excerpt=payload.body[:80])
    session.commit()
    loaded = session.exec(_query().where(Quote.id == row.id)).first()
    if loaded is None:
        raise HTTPException(status_code=500, detail="Could not save that quote")
    return _out(loaded, me)


@router.patch("/{quote_id}", response_model=QuoteOut)
def edit_quote(
    quote_id: int,
    payload: QuotePatchIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> QuoteOut:
    row = session.exec(_query().where(Quote.id == quote_id)).first()
    if row is None:
        raise HTTPException(status_code=404, detail="No quote with that id")
    if row.user_id != me.id:
        raise HTTPException(status_code=403, detail="You can only edit your own quotes")
    if payload.body is None and "page" not in payload.model_fields_set:
        raise HTTPException(status_code=400, detail="Nothing to update")
    if payload.body is not None:
        row.body = payload.body
    if "page" in payload.model_fields_set:
        row.page = payload.page
    session.add(row)
    session.commit()
    session.refresh(row)
    return _out(row, me)


@router.delete("/{quote_id}", status_code=204)
def delete_quote(
    quote_id: int,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> None:
    row = session.get(Quote, quote_id)
    if row is None:
        raise HTTPException(status_code=404, detail="No quote with that id")
    if row.user_id != me.id:
        raise HTTPException(status_code=403, detail="You can only delete your own quotes")
    session.delete(row)
    session.commit()

from datetime import datetime, timezone
from enum import Enum
from typing import Optional

from sqlalchemy import Column, DateTime, UniqueConstraint
from sqlmodel import Field, Relationship, SQLModel


def utcnow() -> datetime:
    return datetime.now(timezone.utc)


class ShelfStatus(str, Enum):
    want_to_read = "want_to_read"
    currently_reading = "currently_reading"
    finished = "finished"
    did_not_finish = "did_not_finish"


class User(SQLModel, table=True):
    __tablename__ = "users"

    id: Optional[int] = Field(default=None, primary_key=True)
    username: str = Field(unique=True, index=True, max_length=32)
    password_hash: str
    created_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )

    shelf_entries: list["ShelfEntry"] = Relationship(back_populates="user")


class Invite(SQLModel, table=True):
    __tablename__ = "invites"

    id: Optional[int] = Field(default=None, primary_key=True)
    code: str = Field(unique=True, index=True, max_length=32)
    created_by_id: Optional[int] = Field(default=None, foreign_key="users.id")
    used_by_id: Optional[int] = Field(default=None, foreign_key="users.id")
    created_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )
    used_at: Optional[datetime] = Field(
        default=None,
        sa_column=Column(DateTime(timezone=True), nullable=True),
    )


class Book(SQLModel, table=True):
    __tablename__ = "books"

    id: Optional[int] = Field(default=None, primary_key=True)
    ol_work_key: str = Field(unique=True, index=True, max_length=64)
    title: str
    authors: str = ""
    cover_id: Optional[int] = None
    year: Optional[int] = None

    shelf_entries: list["ShelfEntry"] = Relationship(back_populates="book")


class ShelfEntry(SQLModel, table=True):
    __tablename__ = "shelf"
    __table_args__ = (
        UniqueConstraint("user_id", "book_id", name="uq_shelf_user_book"),
    )

    id: Optional[int] = Field(default=None, primary_key=True)
    user_id: int = Field(foreign_key="users.id", index=True)
    book_id: int = Field(foreign_key="books.id")
    status: ShelfStatus = Field(index=True)
    position: int = Field(default=0)
    updated_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )

    user: Optional[User] = Relationship(back_populates="shelf_entries")
    book: Optional[Book] = Relationship(back_populates="shelf_entries")

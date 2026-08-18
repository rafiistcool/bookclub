import re
from datetime import datetime

from pydantic import BaseModel, Field, field_validator

from app.models import ShelfStatus

USERNAME_RE = re.compile(r"^[a-z0-9_]{2,32}$")


class UserOut(BaseModel):
    id: int
    username: str


class RegisterIn(BaseModel):
    username: str
    password: str
    invite_code: str

    @field_validator("username")
    @classmethod
    def username_ok(cls, value: str) -> str:
        value = value.strip().lower()
        if not USERNAME_RE.fullmatch(value):
            raise ValueError(
                "Username must be 2–32 characters: lowercase letters, numbers, underscore"
            )
        return value

    @field_validator("password")
    @classmethod
    def password_ok(cls, value: str) -> str:
        if len(value) < 8:
            raise ValueError("Password must be at least 8 characters")
        return value


class LoginIn(BaseModel):
    username: str
    password: str

    @field_validator("username")
    @classmethod
    def username_ok(cls, value: str) -> str:
        return value.strip().lower()


class InviteOut(BaseModel):
    code: str
    used: bool
    used_by: str | None
    created_at: datetime


class InviteCreated(BaseModel):
    code: str


class BookOut(BaseModel):
    id: int
    ol_work_key: str
    title: str
    authors: str
    cover_id: int | None
    year: int | None
    cover_url: str | None


class SearchHit(BaseModel):
    ol_work_key: str
    title: str
    authors: str
    cover_id: int | None
    year: int | None
    on_shelf: ShelfStatus | None = None
    shelf_id: int | None = None


class SearchPage(BaseModel):
    items: list[SearchHit]
    page: int
    has_more: bool


class ShelfItemOut(BaseModel):
    id: int
    status: ShelfStatus
    position: int
    updated_at: datetime
    book: BookOut


class ShelfAddIn(BaseModel):
    ol_work_key: str
    title: str
    authors: str = ""
    cover_id: int | None = None
    year: int | None = None
    status: ShelfStatus = ShelfStatus.want_to_read

    @field_validator("ol_work_key")
    @classmethod
    def work_key_ok(cls, value: str) -> str:
        value = value.strip()
        if not value.startswith("/works/"):
            raise ValueError("ol_work_key must be an Open Library work key")
        return value

    @field_validator("title")
    @classmethod
    def title_ok(cls, value: str) -> str:
        value = value.strip()
        if not value:
            raise ValueError("title is required")
        return value


class ShelfPatchIn(BaseModel):
    status: ShelfStatus | None = None
    position: int | None = Field(default=None, ge=0)


class ShelfListOut(BaseModel):
    user: UserOut
    items: list[ShelfItemOut]


class ReadingPreview(BaseModel):
    title: str
    cover_id: int | None


class MemberOut(BaseModel):
    username: str
    currently_reading_count: int
    currently_reading_preview: list[ReadingPreview]

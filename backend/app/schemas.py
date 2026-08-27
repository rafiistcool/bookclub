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
    club_pick: bool = False


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
    rating: int | None = None
    take: str = ""
    dnf_reason: str = ""
    progress: int | None = None


class ShelfAddIn(BaseModel):
    ol_work_key: str
    title: str
    authors: str = ""
    cover_id: int | None = None
    year: int | None = None
    status: ShelfStatus = ShelfStatus.want_to_read
    rating: int | None = Field(default=None, ge=1, le=5)
    take: str = ""
    dnf_reason: str = ""
    progress: int | None = Field(default=None, ge=0, le=100)

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

    @field_validator("take")
    @classmethod
    def take_ok(cls, value: str) -> str:
        value = value.strip()
        if len(value) > 140:
            raise ValueError("Keep the take to 140 characters")
        return value

    @field_validator("dnf_reason")
    @classmethod
    def dnf_ok(cls, value: str) -> str:
        value = value.strip()
        if len(value) > 200:
            raise ValueError("Keep the reason to 200 characters")
        return value


class ShelfPatchIn(BaseModel):
    status: ShelfStatus | None = None
    position: int | None = Field(default=None, ge=0)
    rating: int | None = Field(default=None, ge=1, le=5)
    take: str | None = None
    dnf_reason: str | None = None
    progress: int | None = Field(default=None, ge=0, le=100)

    @field_validator("take")
    @classmethod
    def take_ok(cls, value: str | None) -> str | None:
        if value is None:
            return None
        value = value.strip()
        if len(value) > 140:
            raise ValueError("Keep the take to 140 characters")
        return value

    @field_validator("dnf_reason")
    @classmethod
    def dnf_ok(cls, value: str | None) -> str | None:
        if value is None:
            return None
        value = value.strip()
        if len(value) > 200:
            raise ValueError("Keep the reason to 200 characters")
        return value


class ShelfListOut(BaseModel):
    user: UserOut
    items: list[ShelfItemOut]


class GoodreadsSkipOut(BaseModel):
    title: str
    reason: str


class GoodreadsImportOut(BaseModel):
    imported: int
    skipped: int
    skips: list[GoodreadsSkipOut]


class ReadingPreview(BaseModel):
    title: str
    cover_id: int | None


class MemberOut(BaseModel):
    username: str
    currently_reading_count: int
    currently_reading_preview: list[ReadingPreview]


class ClubPickSetIn(BaseModel):
    ol_work_key: str
    title: str
    authors: str = ""
    cover_id: int | None = None
    year: int | None = None
    note: str = ""
    meeting_at: str | None = None

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

    @field_validator("note")
    @classmethod
    def note_ok(cls, value: str) -> str:
        value = value.strip()
        if len(value) > 280:
            raise ValueError("note must be 280 characters or fewer")
        return value


class ClubPickReader(BaseModel):
    username: str
    status: ShelfStatus
    rating: int | None = None
    take: str = ""
    dnf_reason: str = ""
    progress: int | None = None


class ClubPickOut(BaseModel):
    id: int
    book: BookOut
    set_by: str
    note: str
    started_at: datetime
    ended_at: datetime | None = None
    meeting_at: datetime | None = None
    meeting_local: str | None = None
    meeting_label: str | None = None
    on_shelf: ShelfStatus | None = None
    shelf_id: int | None = None
    readers: list[ClubPickReader] = []
    reading: list[str] = []
    finished: list[str] = []


class ClubPickCurrentOut(BaseModel):
    pick: ClubPickOut | None
    timezone: str


class ClubPickHistoryOut(BaseModel):
    items: list[ClubPickOut]
    timezone: str


class OverlapMember(BaseModel):
    username: str
    status: ShelfStatus


class OverlapBookOut(BaseModel):
    book: BookOut
    count: int
    members: list[OverlapMember]


class OverlapOut(BaseModel):
    items: list[OverlapBookOut]
    include_reading: bool


class PickPostIn(BaseModel):
    body: str

    @field_validator("body")
    @classmethod
    def body_ok(cls, value: str) -> str:
        value = value.strip()
        if not value:
            raise ValueError("Write a short take, quote, or meeting note")
        if len(value) > 1000:
            raise ValueError("Keep it to 1000 characters")
        return value


class PickPostOut(BaseModel):
    id: int
    author: str
    body: str
    created_at: datetime
    created_label: str


class PickPostListOut(BaseModel):
    pick_id: int
    can_post: bool
    items: list[PickPostOut]
    timezone: str


class VoteNominateIn(BaseModel):
    ol_work_key: str
    title: str
    authors: str = ""
    cover_id: int | None = None
    year: int | None = None

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


class VoteCastIn(BaseModel):
    nomination_id: int


class VoteApplyIn(BaseModel):
    nomination_id: int
    meeting_at: str | None = None


class VoteNominationOut(BaseModel):
    id: int
    book: BookOut
    nominated_by: str
    votes: int
    voters: list[str]
    mine: bool


class VoteOut(BaseModel):
    vote_id: int | None
    nominations: list[VoteNominationOut]
    my_vote_id: int | None
    nomination_limit: int
    can_nominate: bool
    timezone: str


class VoteApplyOut(BaseModel):
    pick: ClubPickOut
    vote: VoteOut

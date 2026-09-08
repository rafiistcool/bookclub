"""Work keys: Open Library (`OL123W`) and club-added books (`BCdeadbeef01`)."""

import re
import secrets

OL_WORK_ID_RE = re.compile(r"^OL\d+W$")
CLUB_WORK_ID_RE = re.compile(r"^BC[a-f0-9]{10}$")
WORK_ID_RE = re.compile(r"^(?:OL\d+W|BC[a-f0-9]{10})$")


def is_open_library_work_id(work_id: str) -> bool:
    return bool(OL_WORK_ID_RE.fullmatch(work_id))


def is_club_work_id(work_id: str) -> bool:
    return bool(CLUB_WORK_ID_RE.fullmatch(work_id))


def is_work_id(work_id: str) -> bool:
    return bool(WORK_ID_RE.fullmatch(work_id))


def work_key(work_id: str) -> str:
    return f"/works/{work_id}"


def work_id_from_key(ol_work_key: str) -> str:
    return ol_work_key.rsplit("/", 1)[-1]


def is_club_work_key(ol_work_key: str) -> bool:
    return is_club_work_id(work_id_from_key(ol_work_key))


def new_club_work_id() -> str:
    return f"BC{secrets.token_hex(5)}"

"""Work keys stored on `Book.ol_work_key`.

- Open Library: `OL123W`
- Club-typed books: `BCdeadbeef01`
- Google Books with an ISBN: `ISBN9780316769488` (stable, OL cover-by-ISBN)
- Google Books without an ISBN: `GBzyTCAlFPjgYC` (volume id)
"""

import re
import secrets

OL_WORK_ID_RE = re.compile(r"^OL\d+W$")
CLUB_WORK_ID_RE = re.compile(r"^BC[a-f0-9]{10}$")
ISBN_WORK_ID_RE = re.compile(r"^ISBN(?:\d{9}[\dXx]|\d{13})$")
GOOGLE_VOLUME_ID_RE = re.compile(r"^[A-Za-z0-9_-]{1,40}$")
GOOGLE_WORK_ID_RE = re.compile(r"^GB[A-Za-z0-9_-]{1,40}$")
WORK_ID_RE = re.compile(
    r"^(?:OL\d+W|BC[a-f0-9]{10}|ISBN(?:\d{9}[\dXx]|\d{13})|GB[A-Za-z0-9_-]{1,40})$"
)


def is_open_library_work_id(work_id: str) -> bool:
    return bool(OL_WORK_ID_RE.fullmatch(work_id))


def is_club_work_id(work_id: str) -> bool:
    return bool(CLUB_WORK_ID_RE.fullmatch(work_id))


def is_isbn_work_id(work_id: str) -> bool:
    return bool(ISBN_WORK_ID_RE.fullmatch(work_id))


def is_google_work_id(work_id: str) -> bool:
    return bool(GOOGLE_WORK_ID_RE.fullmatch(work_id))


def is_google_catalog_id(work_id: str) -> bool:
    """ISBN- or volume-keyed rows imported from Google Books."""
    return is_isbn_work_id(work_id) or is_google_work_id(work_id)


def is_work_id(work_id: str) -> bool:
    return bool(WORK_ID_RE.fullmatch(work_id))


def work_key(work_id: str) -> str:
    return f"/works/{work_id}"


def work_id_from_key(ol_work_key: str) -> str:
    return ol_work_key.rsplit("/", 1)[-1]


def is_club_work_key(ol_work_key: str) -> bool:
    return is_club_work_id(work_id_from_key(ol_work_key))


def is_google_catalog_key(ol_work_key: str) -> bool:
    return is_google_catalog_id(work_id_from_key(ol_work_key))


def isbn_from_work_id(work_id: str) -> str | None:
    if not is_isbn_work_id(work_id):
        return None
    return work_id[4:]


def isbn_from_work_key(ol_work_key: str) -> str | None:
    return isbn_from_work_id(work_id_from_key(ol_work_key))


def google_volume_id(work_id: str) -> str | None:
    if not is_google_work_id(work_id):
        return None
    return work_id[2:]


def google_catalog_work_id(*, isbn: str | None, volume_id: str) -> str | None:
    """Prefer a normalized ISBN key; otherwise a Google volume id."""
    cleaned = re.sub(r"[\s-]", "", isbn or "").upper()
    if ISBN_WORK_ID_RE.fullmatch(f"ISBN{cleaned}"):
        return f"ISBN{cleaned}"
    volume = (volume_id or "").strip()
    if GOOGLE_VOLUME_ID_RE.fullmatch(volume):
        return f"GB{volume}"
    return None


def new_club_work_id() -> str:
    return f"BC{secrets.token_hex(5)}"

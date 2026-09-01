"""Append-only club activity log.

Every mutation that other members would care about calls `record()`; the
feed endpoint reads the table back newest-first. Payload is a small JSON
object so the feed can render "mara finished Dune ★★★★" without joining
back into shelf rows that may since have changed.
"""

import json
from typing import Any

from sqlmodel import Session

from app.models import Book, ClubPick, Event, User

KINDS = frozenset(
    {
        "member_joined",
        "shelf_added",
        "shelf_moved",
        "shelf_finished",
        "shelf_dnf",
        "shelf_removed",
        "progress",
        "pick_set",
        "pick_cleared",
        "note_posted",
        "reaction",
        "nominated",
        "voted",
        "vote_closed",
        "milestone_added",
        "quote_added",
    }
)


def record(
    session: Session,
    actor: User,
    kind: str,
    *,
    book: Book | None = None,
    pick: ClubPick | None = None,
    **payload: Any,
) -> Event:
    if kind not in KINDS:
        raise ValueError(f"Unknown event kind: {kind}")
    clean = {key: value for key, value in payload.items() if value is not None}
    event = Event(
        actor_id=actor.id or 0,
        kind=kind,
        book_id=book.id if book is not None else None,
        pick_id=pick.id if pick is not None else None,
        payload=json.dumps(clean, ensure_ascii=False),
    )
    session.add(event)
    return event


def payload_of(event: Event) -> dict[str, Any]:
    try:
        data = json.loads(event.payload or "{}")
    except json.JSONDecodeError:
        return {}
    return data if isinstance(data, dict) else {}

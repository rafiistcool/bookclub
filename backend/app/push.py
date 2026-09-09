"""Web Push delivery.

VAPID keys follow the same convention as SECRET_KEY: set VAPID_PRIVATE_KEY
(PEM) explicitly, or leave it empty and a key pair is generated into the data
directory on first use. Sending happens in FastAPI background tasks so a
request never waits on a push service; dead subscriptions (404/410) are
pruned afterwards with a fresh session.
"""

from __future__ import annotations

import base64
import json
import logging
from datetime import timedelta
from functools import lru_cache
from pathlib import Path
from typing import Any, Callable

from cryptography.hazmat.primitives import serialization
from fastapi import BackgroundTasks
from py_vapid import Vapid
from pywebpush import WebPushException, webpush
from sqlalchemy.engine import Engine
from sqlmodel import Session, col, select

from app.config import get_settings
from app.i18n import DEFAULT_LOCALE, normalize_locale
from app.models import ClubPick, PushSubscription, User, utcnow
from app.runtime import data_dir_from_env, read_or_create_file

logger = logging.getLogger("bookclub.push")

PRIVATE_KEY_FILENAME = ".vapid_private.pem"
REMINDER_WINDOW = timedelta(hours=24)
PREF_FOR_KIND = {
    "meeting": "notify_meeting",
    "pick": "notify_pick",
    "note": "notify_note",
    "test": None,
}


def _generate_private_pem() -> str:
    vapid = Vapid()
    vapid.generate_keys()
    return vapid.private_pem().decode("utf-8").strip()


@lru_cache
def private_key_pem() -> str:
    explicit = (get_settings().vapid_private_key or "").strip()
    if explicit:
        return explicit.replace("\\n", "\n")
    pem, created = read_or_create_file(
        data_dir_from_env() / PRIVATE_KEY_FILENAME, _generate_private_pem
    )
    if created:
        logger.info("Generated VAPID key pair for Web Push")
    return pem


def reset_keys() -> None:
    private_key_pem.cache_clear()


def public_key_b64url() -> str:
    """Uncompressed P-256 point, base64url without padding — what PushManager wants."""
    vapid = Vapid.from_pem(private_key_pem().encode("utf-8"))
    raw = vapid.public_key.public_bytes(
        serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint
    )
    return base64.urlsafe_b64encode(raw).decode("ascii").rstrip("=")


def vapid_subject() -> str:
    settings = get_settings()
    if settings.vapid_subject.strip():
        return settings.vapid_subject.strip()
    if settings.bookclub_public_url:
        return settings.bookclub_public_url
    return "mailto:bookclub@localhost"


def send_to_subscription(target: dict[str, Any], payload: dict[str, Any]) -> bool | None:
    """Deliver one notification. False => subscription is dead and should be removed.

    Tests monkeypatch this to capture payloads instead of talking to a push service.
    """
    info = {
        "endpoint": target["endpoint"],
        "keys": {"p256dh": target["p256dh"], "auth": target["auth"]},
    }
    try:
        webpush(
            subscription_info=info,
            data=json.dumps(payload, ensure_ascii=False),
            vapid_private_key=private_key_pem(),
            vapid_claims={"sub": vapid_subject()},
            ttl=60 * 60 * 24,
            timeout=8.0,
        )
        return True
    except WebPushException as exc:
        status = getattr(getattr(exc, "response", None), "status_code", None)
        if status in (404, 410):
            return False
        logger.warning("push failed status=%s err=%s", status, exc.__class__.__name__)
        return None
    except Exception as exc:  # pragma: no cover - network / crypto edge cases
        logger.warning("push failed err=%s", exc.__class__.__name__)
        return None


def collect_targets(
    session: Session, kind: str, *, exclude_user_id: int | None = None
) -> list[dict[str, Any]]:
    pref = PREF_FOR_KIND.get(kind)
    users = session.exec(select(User)).all()
    allowed = {
        user.id
        for user in users
        if user.id != exclude_user_id and (pref is None or getattr(user, pref, True))
    }
    if not allowed:
        return []
    locale_by_id = {
        user.id: normalize_locale(getattr(user, "locale", None)) for user in users
    }
    subs = session.exec(
        select(PushSubscription).where(col(PushSubscription.user_id).in_(list(allowed)))
    ).all()
    return [
        {
            "id": sub.id,
            "endpoint": sub.endpoint,
            "p256dh": sub.p256dh,
            "auth": sub.auth,
            "locale": locale_by_id.get(sub.user_id, DEFAULT_LOCALE),
        }
        for sub in subs
    ]


def deliver(engine: Engine, targets: list[dict[str, Any]], payload: dict[str, Any]) -> None:
    dead: list[int] = []
    for target in targets:
        if send_to_subscription(target, payload) is False:
            dead.append(int(target["id"]))
    if not dead:
        return
    with Session(engine) as session:
        for sub in session.exec(
            select(PushSubscription).where(col(PushSubscription.id).in_(dead))
        ).all():
            session.delete(sub)
        session.commit()


def schedule(
    background: BackgroundTasks | None,
    engine: Engine,
    session: Session,
    *,
    kind: str,
    title: str,
    body: str,
    url: str = "/",
    tag: str | None = None,
    exclude_user_id: int | None = None,
    copy: dict[str, tuple[str, str]] | None = None,
) -> int:
    """Queue a notification to every opted-in member. Returns how many devices were targeted.

    `copy` maps locale → (title, body). Recipients without a match get `title`/`body`.
    """
    targets = collect_targets(session, kind, exclude_user_id=exclude_user_id)
    if not targets:
        return 0
    groups: dict[str, list[dict[str, Any]]] = {}
    for target in targets:
        loc = normalize_locale(str(target.get("locale") or DEFAULT_LOCALE))
        groups.setdefault(loc, []).append(target)
    for loc, group in groups.items():
        localized = (copy or {}).get(loc)
        text_title, text_body = localized if localized else (title, body)
        payload = {
            "title": text_title,
            "body": text_body,
            "url": url,
            "tag": tag or kind,
            "kind": kind,
        }
        if background is not None:
            background.add_task(deliver, engine, group, payload)
        else:
            deliver(engine, group, payload)
    return len(targets)


def maybe_schedule_meeting_reminder(
    background: BackgroundTasks | None,
    engine: Engine,
    session: Session,
    pick: ClubPick | None,
    club_name: str,
) -> bool:
    """Send the 24-hour meeting reminder once. Called opportunistically on reads."""
    if pick is None or pick.meeting_at is None or pick.reminder_sent_at is not None:
        return False
    if pick.book is None:
        return False
    meeting = pick.meeting_at
    if meeting.tzinfo is None:
        from datetime import timezone

        meeting = meeting.replace(tzinfo=timezone.utc)
    now = utcnow()
    if meeting <= now or meeting - now > REMINDER_WINDOW:
        return False
    pick.reminder_sent_at = now
    session.add(pick)
    session.commit()
    title = f"{club_name}: meeting tomorrow"
    body = f"{pick.book.title} — see you at the meeting."
    schedule(
        background,
        engine,
        session,
        kind="meeting",
        title=title,
        body=body,
        url="/",
        tag=f"meeting-{pick.id}",
        copy={
            "en": (title, body),
            "de": (
                f"{club_name}: Treffen morgen",
                f"{pick.book.title} — bis zum Treffen.",
            ),
        },
    )
    return True


def ensure_keys_at_startup(data_dir: Path | None = None) -> None:  # pragma: no cover - thin wrapper
    private_key_pem()

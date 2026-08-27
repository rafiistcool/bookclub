from datetime import datetime, timezone
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError


def resolved_timezone(name: str | None) -> str:
    raw = (name or "UTC").strip() or "UTC"
    try:
        ZoneInfo(raw)
    except ZoneInfoNotFoundError:
        return "UTC"
    return raw


def club_zone(name: str | None) -> ZoneInfo:
    return ZoneInfo(resolved_timezone(name))


def parse_meeting(value: str | None, tz_name: str | None) -> datetime | None:
    if value is None:
        return None
    text = value.strip()
    if not text:
        return None
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError as exc:
        raise ValueError("meeting_at must be a date and time") from exc
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=club_zone(tz_name))
    return parsed.astimezone(timezone.utc)


def _aware(dt: datetime) -> datetime:
    if dt.tzinfo is None:
        return dt.replace(tzinfo=timezone.utc)
    return dt


def meeting_local(dt: datetime | None, tz_name: str | None) -> str | None:
    if dt is None:
        return None
    local = _aware(dt).astimezone(club_zone(tz_name))
    return local.strftime("%Y-%m-%dT%H:%M")


def meeting_label(dt: datetime | None, tz_name: str | None) -> str | None:
    if dt is None:
        return None
    local = _aware(dt).astimezone(club_zone(tz_name))
    return local.strftime("%a %d %b %Y, %H:%M %Z")

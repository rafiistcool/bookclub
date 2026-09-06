"""Minimal RFC 5545 rendering for the club meeting — one VEVENT, no deps."""

from datetime import datetime, timedelta, timezone

MEETING_LENGTH = timedelta(minutes=90)


def _escape(value: str) -> str:
    return (
        value.replace("\\", "\\\\")
        .replace(";", "\\;")
        .replace(",", "\\,")
        .replace("\r\n", "\\n")
        .replace("\n", "\\n")
    )


def _stamp(dt: datetime) -> str:
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt.astimezone(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def _fold(line: str) -> str:
    """Fold at 75 octets per RFC 5545 §3.1 (continuation lines start with a space)."""
    raw = line.encode("utf-8")
    if len(raw) <= 75:
        return line
    chunks: list[str] = []
    while raw:
        cut = 75 if not chunks else 74
        piece = raw[:cut]
        # Never split a multi-byte character: back off while the byte that
        # would start the next line is a UTF-8 continuation byte.
        while len(piece) < len(raw) and (raw[len(piece)] & 0xC0) == 0x80:
            piece = piece[:-1]
        chunks.append(piece.decode("utf-8"))
        raw = raw[len(piece):]
    return "\r\n ".join(chunks)


def build_meeting_ics(
    *,
    pick_id: int,
    club_name: str,
    title: str,
    authors: str,
    note: str,
    starts_at: datetime,
    ol_work_key: str,
    public_url: str = "",
    now: datetime | None = None,
) -> str:
    summary = f"{club_name}: {title}"
    description_parts = [f"{title}" + (f" — {authors}" if authors else "")]
    if note:
        description_parts.append(note)
    description_parts.append(f"https://openlibrary.org{ol_work_key}")
    if public_url:
        description_parts.append(public_url)
    description = "\n".join(description_parts)
    stamp = _stamp(now or datetime.now(timezone.utc))
    lines = [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        f"PRODID:-//{_escape(club_name)}//Bookclub//EN",
        "CALSCALE:GREGORIAN",
        "METHOD:PUBLISH",
        "BEGIN:VEVENT",
        f"UID:pick-{pick_id}@bookclub",
        f"DTSTAMP:{stamp}",
        f"DTSTART:{_stamp(starts_at)}",
        f"DTEND:{_stamp(starts_at + MEETING_LENGTH)}",
        f"SUMMARY:{_escape(summary)}",
        f"DESCRIPTION:{_escape(description)}",
    ]
    if public_url:
        lines.append(f"URL:{public_url}")
    lines += ["END:VEVENT", "END:VCALENDAR"]
    return "\r\n".join(_fold(line) for line in lines) + "\r\n"

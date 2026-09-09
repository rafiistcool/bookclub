"""Per-member UI locale. English is the default; German is the extra toggle."""

from __future__ import annotations

LOCALES: tuple[str, ...] = ("en", "de")
DEFAULT_LOCALE = "en"

# kind → locale → (title, body) templates for Web Push.
PUSH_COPY: dict[str, dict[str, tuple[str, str]]] = {
    "pick": {
        "en": ("{club}: new club pick", "{actor} chose {title}."),
        "de": ("{club}: neues Club-Buch", "{actor} hat {title} gewählt."),
    },
    "note": {
        "en": ("{actor} on {title}", "{excerpt}"),
        "de": ("{actor} zu {title}", "{excerpt}"),
    },
    "meeting": {
        "en": ("{club}: meeting tomorrow", "{title} — see you at the meeting."),
        "de": ("{club}: Treffen morgen", "{title} — bis zum Treffen."),
    },
    "test": {
        "en": (
            "{club}: notifications are on",
            "You will hear about new picks, notes, and meeting reminders.",
        ),
        "de": (
            "{club}: Benachrichtigungen sind an",
            "Du hörst von neuen Club-Büchern, Notizen und Treffen.",
        ),
    },
}


def is_locale(value: str) -> bool:
    return value in LOCALES


def normalize_locale(value: str | None) -> str:
    raw = (value or "").strip().lower()
    return raw if is_locale(raw) else DEFAULT_LOCALE


def push_copy(kind: str, locale: str | None, **fields: object) -> tuple[str, str]:
    """Format push title/body for one locale. Unknown locales fall back to English."""
    by_locale = PUSH_COPY[kind]
    loc = normalize_locale(locale)
    title, body = by_locale.get(loc) or by_locale[DEFAULT_LOCALE]
    return title.format(**fields), body.format(**fields)


def push_copy_map(kind: str, **fields: object) -> dict[str, tuple[str, str]]:
    return {loc: push_copy(kind, loc, **fields) for loc in LOCALES}

"""Per-member UI locale. English is the default; German is the extra toggle."""

from __future__ import annotations

LOCALES: tuple[str, ...] = ("en", "de")
DEFAULT_LOCALE = "en"


def is_locale(value: str) -> bool:
    return value in LOCALES


def normalize_locale(value: str | None) -> str:
    raw = (value or "").strip().lower()
    return raw if is_locale(raw) else DEFAULT_LOCALE

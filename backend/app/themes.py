"""Per-user palette and colour-mode allowlists.

Unrelated to `app.branding`, which deals with the club-wide accent hex read from
the BOOKCLUB_THEME env var. This module is the single source of truth for the
palette ids the frontend ships, so validation cannot drift from the stylesheets.
"""

from __future__ import annotations

THEME_IDS: tuple[str, ...] = ("paper", "slate", "forest", "ink")
DEFAULT_THEME_ID = "paper"

COLOR_MODES: tuple[str, ...] = ("light", "dark", "system")
DEFAULT_COLOR_MODE = "system"


def is_theme_id(value: str) -> bool:
    return value in THEME_IDS


def is_color_mode(value: str) -> bool:
    return value in COLOR_MODES

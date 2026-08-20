"""Club name, theme, public URL, and CORS helpers."""

from __future__ import annotations

import logging
import re
from html import escape
from urllib.parse import urlparse

from app.config import Settings

logger = logging.getLogger("bookclub")

DEFAULT_NAME = "Bookclub"
DEFAULT_THEME = "#b44a2a"
DEFAULT_THEME_DARK = "#8e361c"
MAX_NAME_LEN = 48
THEME_RE = re.compile(r"^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})$")
DEV_ORIGINS = (
    "http://localhost:5173",
    "http://127.0.0.1:5173",
)


def sanitize_name(value: str | None) -> str:
    raw = (value or "").strip()
    cleaned = "".join(ch for ch in raw if ch.isprintable() and ch not in "<>")
    cleaned = " ".join(cleaned.split())
    return cleaned[:MAX_NAME_LEN] or DEFAULT_NAME


def sanitize_theme(value: str | None) -> str:
    raw = (value or "").strip()
    if THEME_RE.fullmatch(raw):
        return raw.lower() if len(raw) == 4 else raw
    return ""


def _expand_hex(color: str) -> str:
    if len(color) == 4:
        return "#" + "".join(ch * 2 for ch in color[1:])
    return color


def darken_hex(color: str, factor: float = 0.78) -> str:
    expanded = _expand_hex(color)
    rgb = [int(expanded[i : i + 2], 16) for i in (1, 3, 5)]
    darkened = [max(0, min(255, int(channel * factor))) for channel in rgb]
    return "#" + "".join(f"{channel:02x}" for channel in darkened)


def resolve_theme(theme: str | None, theme_dark: str | None) -> tuple[str, str]:
    accent = sanitize_theme(theme) or DEFAULT_THEME
    dark = sanitize_theme(theme_dark) or (
        DEFAULT_THEME_DARK if accent.lower() == DEFAULT_THEME else darken_hex(accent)
    )
    return accent, dark


def public_origin(value: str | None) -> str:
    raw = (value or "").strip()
    if not raw:
        return ""
    parsed = urlparse(raw)
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        return ""
    return f"{parsed.scheme}://{parsed.netloc}"


def cors_origins(settings: Settings) -> list[str]:
    origins: list[str] = []
    if settings.debug:
        origins.extend(DEV_ORIGINS)
    origin = public_origin(settings.bookclub_public_url)
    if origin and origin not in origins:
        origins.append(origin)
    return origins


def open_library_ua(settings: Settings) -> str:
    token = re.sub(r"[^A-Za-z0-9._-]+", "", sanitize_name(settings.bookclub_name))
    token = token or "Bookclub"
    contact = public_origin(settings.bookclub_public_url) or "private book club"
    return f"{token}/1.0 ({contact}; catalog browse via Open Library)"


def warn_invalid_theme(settings: Settings) -> None:
    raw = (settings.bookclub_theme or "").strip()
    if raw and not sanitize_theme(raw):
        logger.warning(
            "Invalid BOOKCLUB_THEME %r; using default %s", raw, DEFAULT_THEME
        )
    raw_dark = (settings.bookclub_theme_dark or "").strip()
    if raw_dark and not sanitize_theme(raw_dark):
        logger.warning("Invalid BOOKCLUB_THEME_DARK %r; ignoring", raw_dark)


def public_config(settings: Settings) -> dict[str, str]:
    name = sanitize_name(settings.bookclub_name)
    theme, theme_dark = resolve_theme(settings.bookclub_theme, settings.bookclub_theme_dark)
    return {
        "name": name,
        "theme": theme,
        "theme_dark": theme_dark,
        "public_url": public_origin(settings.bookclub_public_url),
    }


def brand_index_html(html: str, settings: Settings) -> str:
    cfg = public_config(settings)
    name = escape(cfg["name"], quote=True)
    html = html.replace("<title>Bookclub</title>", f"<title>{name}</title>")
    html = html.replace(
        'name="apple-mobile-web-app-title" content="Bookclub"',
        f'name="apple-mobile-web-app-title" content="{name}"',
    )
    html = html.replace(
        'name="theme-color" content="#b44a2a"',
        f'name="theme-color" content="{cfg["theme"]}"',
    )
    return html


def manifest_payload(settings: Settings) -> dict[str, str]:
    cfg = public_config(settings)
    return {
        "name": cfg["name"],
        "short_name": cfg["name"][:32],
        "start_url": "/",
        "display": "standalone",
        "background_color": "#f6f1e8",
        "theme_color": cfg["theme"],
    }

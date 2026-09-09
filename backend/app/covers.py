"""Remote cover URLs (Google Books imageLinks) and Open Library CDN helpers."""

from __future__ import annotations

from urllib.parse import parse_qsl, urlencode, urlparse, urlunparse

COVER_URL_MAX = 512
_IMAGE_KEYS = (
    "extraLarge",
    "large",
    "medium",
    "small",
    "thumbnail",
    "smallThumbnail",
)


def _host(url: str) -> str:
    return urlparse(url).netloc.lower().split(":")[0]


def is_open_library_cover_host(host: str) -> bool:
    return host == "covers.openlibrary.org" or host.endswith(".covers.openlibrary.org")


def is_allowed_cover_host(host: str) -> bool:
    """Hosts we persist and render as a remote override (Google Books)."""
    if host == "books.google.com" or host.endswith(".books.google.com"):
        return True
    if host == "googleusercontent.com" or host.endswith(".googleusercontent.com"):
        return True
    return False


def normalize_cover_image_url(value: object) -> str | None:
    """Accept an http(s) cover URL. Rewrite http→https. Reject junk."""
    text = str(value or "").strip()
    if not text:
        return None
    if text.startswith("http://"):
        text = "https://" + text[7:]
    parsed = urlparse(text)
    if parsed.scheme != "https" or not parsed.netloc:
        return None
    if len(text) > COVER_URL_MAX:
        return None
    if parsed.username or parsed.password:
        return None
    return text


def storable_cover_image_url(value: object) -> str | None:
    """Persist only Google cover URLs. OL CDN is derivable from cover_id."""
    url = normalize_cover_image_url(value)
    if url is None:
        return None
    host = _host(url)
    if is_open_library_cover_host(host) or not is_allowed_cover_host(host):
        return None
    return url


def _prefer_larger_google_thumb(url: str) -> str:
    """zoom=1 is the default thumbnail; zoom=0 is often a larger front cover."""
    parsed = urlparse(url)
    pairs = parse_qsl(parsed.query, keep_blank_values=True)
    if not pairs:
        return url
    changed = False
    out: list[tuple[str, str]] = []
    for key, value in pairs:
        if key == "edge" and value == "curl":
            changed = True
            continue
        if key == "zoom" and value == "1":
            out.append((key, "0"))
            changed = True
            continue
        out.append((key, value))
    if not changed:
        return url
    return urlunparse(parsed._replace(query=urlencode(out)))


def cover_url_from_image_links(image_links: object) -> str | None:
    if not isinstance(image_links, dict):
        return None
    for key in _IMAGE_KEYS:
        url = normalize_cover_image_url(image_links.get(key))
        if url:
            return storable_cover_image_url(_prefer_larger_google_thumb(url))
    return None

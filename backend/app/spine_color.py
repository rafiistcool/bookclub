"""Pick a spine colour for the physical shelf from a book's cover image."""

from __future__ import annotations

import asyncio
import colorsys
from io import BytesIO
from urllib.parse import urljoin, urlparse

import httpx
from PIL import Image, UnidentifiedImageError

from app.branding import open_library_ua
from app.config import get_settings
from app.covers import is_allowed_cover_host, is_open_library_cover_host
from app.models import Book
from app.serialize import book_cover_url

# archive.org, where Open Library keeps its covers, often takes several seconds per hop.
COVER_TIMEOUT = httpx.Timeout(8.0)
MAX_REDIRECTS = 3
FETCH_DEADLINE = 15.0
MAX_COVER_BYTES = 1_500_000
MAX_COVER_PIXELS = 4_000_000
# Open Library answers a missing cover with a 1×1 pixel when asked nicely.
MIN_COVER_EDGE = 8
# Keep the cloth shading on the spine visible at both ends of the scale.
MIN_LIGHTNESS = 0.17
MAX_LIGHTNESS = 0.86


def spine_cover_url(book: Book) -> str | None:
    """The cover to sample: the same one the UI shows, at a smaller size."""
    url = book_cover_url(book)
    if url and "covers.openlibrary.org" in url:
        return url.replace("-L.jpg", "-M.jpg")
    return url


def _fetchable(url: str) -> bool:
    """Only the cover hosts themselves, and the archive.org hosts Open Library redirects to."""
    parsed = urlparse(url)
    if parsed.scheme != "https" or parsed.port is not None or parsed.username or parsed.password:
        return False
    host = (parsed.hostname or "").lower()
    return (
        is_open_library_cover_host(host)
        or is_allowed_cover_host(host)
        or host == "archive.org"
        or host.endswith(".archive.org")
    )


async def _fetch(url: str) -> bytes | None:
    headers = {"User-Agent": open_library_ua(get_settings())}
    async with httpx.AsyncClient(timeout=COVER_TIMEOUT, follow_redirects=False) as client:
        for _ in range(MAX_REDIRECTS + 1):
            # Every hop is checked before it is requested, so a redirect can
            # never point the server at another host.
            if not _fetchable(url):
                return None
            async with client.stream("GET", url, headers=headers) as response:
                if response.is_redirect and response.headers.get("location"):
                    url = urljoin(url, response.headers["location"])
                    continue
                if response.status_code in (404, 410):
                    return None
                if response.status_code != 200:
                    # Throttling, denials and server errors pass: try again on a later visit.
                    raise httpx.HTTPStatusError(
                        f"cover host answered {response.status_code}",
                        request=response.request,
                        response=response,
                    )
                chunks: list[bytes] = []
                size = 0
                async for chunk in response.aiter_bytes():
                    size += len(chunk)
                    if size > MAX_COVER_BYTES:
                        return None
                    chunks.append(chunk)
                return b"".join(chunks)
    return None


async def fetch_cover(url: str) -> bytes | None:
    """Cover bytes, or None when the host says there is no usable cover.

    Network trouble, throttling and upstream errors raise httpx.HTTPError so
    the caller can retry later instead of recording "no cover" for good.
    """
    try:
        return await asyncio.wait_for(_fetch(url), FETCH_DEADLINE)
    except asyncio.TimeoutError as exc:
        raise httpx.TimeoutException("cover fetch took too long") from exc


def dominant_color(data: bytes) -> str | None:
    """The cover's most characteristic colour as "#rrggbb", or None if unreadable."""
    if not data or len(data) > MAX_COVER_BYTES:
        return None
    try:
        image = Image.open(BytesIO(data))
        width, height = image.size
        if min(width, height) < MIN_COVER_EDGE or width * height > MAX_COVER_PIXELS:
            return None
        image = image.convert("RGB")
    except (Image.DecompressionBombError, UnidentifiedImageError, OSError, ValueError, SyntaxError):
        return None

    # Publisher borders and scan edges are not what a reader remembers.
    inset_x, inset_y = int(width * 0.08), int(height * 0.08)
    image = image.crop((inset_x, inset_y, width - inset_x, height - inset_y))
    image.thumbnail((48, 48))
    quantized = image.quantize(colors=6, method=Image.Quantize.MEDIANCUT)
    palette = quantized.getpalette() or []
    total = image.width * image.height

    best: tuple[float, tuple[float, float, float]] | None = None
    for count, index in quantized.getcolors() or []:
        red, green, blue = (channel / 255 for channel in palette[index * 3 : index * 3 + 3])
        _, saturation, _ = colorsys.rgb_to_hsv(red, green, blue)
        # Area decides, but a vivid colour beats an equally large grey one.
        score = (count / total) * (0.35 + saturation)
        if best is None or score > best[0]:
            best = (score, (red, green, blue))
    if best is None:
        return None

    hue, lightness, saturation = colorsys.rgb_to_hls(*best[1])
    lightness = min(MAX_LIGHTNESS, max(MIN_LIGHTNESS, lightness))
    red, green, blue = colorsys.hls_to_rgb(hue, lightness, saturation)
    return "#{:02x}{:02x}{:02x}".format(round(red * 255), round(green * 255), round(blue * 255))

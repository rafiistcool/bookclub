"""Pick a spine colour for the physical shelf from a book's cover image."""

from __future__ import annotations

import colorsys
from io import BytesIO

import httpx
from PIL import Image, UnidentifiedImageError

from app.branding import open_library_ua
from app.config import get_settings
from app.models import Book
from app.serialize import book_cover_url

COVER_TIMEOUT = httpx.Timeout(4.0)
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


async def fetch_cover(url: str) -> bytes | None:
    """Cover bytes, or None when the host says there is no usable cover.

    Network trouble and upstream 5xx raise httpx.HTTPError so the caller can
    retry later instead of recording "no cover" for good.
    """
    if not url.startswith("https://"):
        return None
    headers = {"User-Agent": open_library_ua(get_settings())}
    async with httpx.AsyncClient(
        timeout=COVER_TIMEOUT, follow_redirects=True, max_redirects=3
    ) as client:
        async with client.stream("GET", url, headers=headers) as response:
            if response.status_code >= 500:
                response.raise_for_status()
            if response.status_code != 200 or response.url.scheme != "https":
                return None
            chunks: list[bytes] = []
            size = 0
            async for chunk in response.aiter_bytes():
                size += len(chunk)
                if size > MAX_COVER_BYTES:
                    return None
                chunks.append(chunk)
    return b"".join(chunks)


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
    except (Image.DecompressionBombError, UnidentifiedImageError, OSError, ValueError):
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

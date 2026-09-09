"""Normalize an uploaded profile picture into a square JPEG blob."""

from __future__ import annotations

from io import BytesIO

from PIL import Image, ImageOps, UnidentifiedImageError

MAX_UPLOAD_BYTES = 50 * 1024 * 1024
MAX_SOURCE_PIXELS = 25_000_000  # ~5000×5000, far above anything that ends up at 256px
UPLOAD_LENGTH_SLACK = 4096
OUTPUT_SIZE = 256
JPEG_QUALITY = 85
AVATAR_MIME = "image/jpeg"
TOO_LARGE = "That picture is too large (50 MB max)"
TOO_MANY_PIXELS = "That picture has too many pixels"
NOT_A_PICTURE = "That file is not a picture"
UNSUPPORTED_TYPE = "Use a JPEG, PNG, WebP, or GIF"

_JPEG = b"\xff\xd8\xff"
_PNG = b"\x89PNG\r\n\x1a\n"
_GIF87 = b"GIF87a"
_GIF89 = b"GIF89a"


def _sniff(data: bytes) -> bool:
    if data.startswith(_JPEG) or data.startswith(_PNG):
        return True
    if data.startswith(_GIF87) or data.startswith(_GIF89):
        return True
    return len(data) >= 12 and data[:4] == b"RIFF" and data[8:12] == b"WEBP"


def oversize_content_length(content_length: str | None) -> bool:
    """True when Content-Length is already bigger than the avatar cap plus multipart slack."""
    if not content_length:
        return False
    try:
        return int(content_length) > MAX_UPLOAD_BYTES + UPLOAD_LENGTH_SLACK
    except ValueError:
        return False


def process_avatar(data: bytes) -> bytes:
    if len(data) > MAX_UPLOAD_BYTES:
        raise ValueError(TOO_LARGE)
    if len(data) < 24 or not _sniff(data):
        raise ValueError(UNSUPPORTED_TYPE)
    try:
        image = Image.open(BytesIO(data))
        width, height = image.size
    except Image.DecompressionBombError as exc:
        raise ValueError(TOO_MANY_PIXELS) from exc
    except (UnidentifiedImageError, OSError) as exc:
        raise ValueError(NOT_A_PICTURE) from exc

    if width <= 0 or height <= 0:
        raise ValueError(NOT_A_PICTURE)
    if width * height > MAX_SOURCE_PIXELS:
        raise ValueError(TOO_MANY_PIXELS)

    try:
        image.load()
    except Image.DecompressionBombError as exc:
        raise ValueError(TOO_MANY_PIXELS) from exc
    except (UnidentifiedImageError, OSError, ValueError) as exc:
        raise ValueError(NOT_A_PICTURE) from exc

    image = ImageOps.exif_transpose(image)
    if getattr(image, "n_frames", 1) > 1:
        image.seek(0)
    if image.mode in {"P", "RGBA", "LA", "PA"}:
        rgba = image.convert("RGBA")
        background = Image.new("RGB", rgba.size, (255, 255, 255))
        background.paste(rgba, mask=rgba.split()[-1])
        image = background
    else:
        image = image.convert("RGB")
    width, height = image.size
    side = min(width, height)
    left = (width - side) // 2
    top = (height - side) // 2
    image = image.crop((left, top, left + side, top + side))
    image = image.resize((OUTPUT_SIZE, OUTPUT_SIZE), Image.Resampling.LANCZOS)

    out = BytesIO()
    image.save(out, format="JPEG", quality=JPEG_QUALITY, optimize=True)
    return out.getvalue()

"""Production/self-host startup: persist secrets, refuse demo defaults."""

from __future__ import annotations

import logging
import os
import secrets
from pathlib import Path

from app.security import generate_invite_code, normalize_invite_code

logger = logging.getLogger("bookclub")

_REPO_ROOT = Path(__file__).resolve().parents[2]
_DEFAULT_DATA_DIR = _REPO_ROOT / "data"

PLACEHOLDER_SECRETS = frozenset(
    {
        "",
        "dev-secret-change-me",
        "change-me-to-a-long-random-string",
        "changeme",
        "secret",
    }
)
MIN_SECRET_LENGTH = 32
SECRET_FILENAME = ".secret_key"
INVITE_FILENAME = ".bootstrap_invite"


def debug_from_env() -> bool:
    raw = os.environ.get("DEBUG")
    if raw is None:
        return True
    return raw.strip().lower() in {"1", "true", "yes", "on"}


def is_placeholder_secret(value: str | None) -> bool:
    if value is None:
        return True
    stripped = value.strip()
    if stripped.lower() in PLACEHOLDER_SECRETS:
        return True
    return len(stripped) < MIN_SECRET_LENGTH


def is_placeholder_invite(value: str | None) -> bool:
    if value is None or not value.strip():
        return True
    return normalize_invite_code(value) == "DEVONLY"


def https_mode(value: str | bool | None) -> str:
    if value is None:
        return "auto"
    if isinstance(value, bool):
        return "always" if value else "never"
    normalized = str(value).strip().lower()
    if normalized in {"1", "true", "yes", "on", "always"}:
        return "always"
    if normalized in {"0", "false", "no", "off", "never"}:
        return "never"
    return "auto"


def trusted_proxy_hosts(value: str | None) -> list[str] | str:
    raw = (value or "*").strip()
    if raw in {"*", "all"}:
        return "*"
    hosts = [part.strip() for part in raw.split(",") if part.strip()]
    return hosts or "*"


def data_dir_from_env() -> Path:
    raw = os.environ.get("DATABASE_PATH")
    if raw:
        return Path(raw).expanduser().resolve().parent
    return _DEFAULT_DATA_DIR


def read_or_create_file(path: Path, factory) -> tuple[str, bool]:
    if path.is_file():
        text = path.read_text(encoding="utf-8").strip()
        if text:
            return text, False
    path.parent.mkdir(parents=True, exist_ok=True)
    value = factory()
    path.write_text(f"{value}\n", encoding="utf-8")
    try:
        path.chmod(0o600)
    except OSError:
        pass
    return value, True


def prepare_environment() -> dict[str, bool]:
    """Fill SECRET_KEY / bootstrap invite for production, persist them under the data dir."""
    debug = debug_from_env()
    info = {
        "debug": debug,
        "generated_secret": False,
        "generated_invite": False,
    }
    if debug:
        return info

    data_dir = data_dir_from_env()
    secret = os.environ.get("SECRET_KEY")
    if is_placeholder_secret(secret):
        secret, created = read_or_create_file(
            data_dir / SECRET_FILENAME, lambda: secrets.token_urlsafe(48)
        )
        if is_placeholder_secret(secret):
            raise RuntimeError(
                "SECRET_KEY is missing or too weak. Set a long random SECRET_KEY "
                f"(at least {MIN_SECRET_LENGTH} characters) or make the data "
                "directory writable so one can be generated."
            )
        os.environ["SECRET_KEY"] = secret
        info["generated_secret"] = created

    invite = os.environ.get("BOOKCLUB_BOOTSTRAP_INVITE")
    if is_placeholder_invite(invite):
        invite, created = read_or_create_file(
            data_dir / INVITE_FILENAME, generate_invite_code
        )
        if is_placeholder_invite(invite):
            raise RuntimeError(
                "Set BOOKCLUB_BOOTSTRAP_INVITE to a secret code when DEBUG=0, "
                "or make the data directory writable so one can be generated."
            )
        os.environ["BOOKCLUB_BOOTSTRAP_INVITE"] = invite
        info["generated_invite"] = created
        if created:
            announce_invite(invite)
    return info


def announce_invite(code: str) -> None:
    banner = (
        "\n"
        "============================================================\n"
        "Bookclub first-run invite (share once, then mint more):\n"
        f"  {code}\n"
        "============================================================\n"
    )
    print(banner, flush=True)
    logger.info("Generated bootstrap invite %s", code)

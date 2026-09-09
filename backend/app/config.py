from functools import lru_cache
from pathlib import Path
from urllib.parse import urlparse
import os

from dotenv import dotenv_values
from pydantic import field_validator
from pydantic_settings import BaseSettings, SettingsConfigDict

_REPO_ROOT = Path(__file__).resolve().parents[2]


def dotenv_path() -> Path:
    raw = os.environ.get("BOOKCLUB_ENV_FILE")
    if raw:
        return Path(raw).expanduser()
    return _REPO_ROOT / ".env"


def apply_dotenv() -> None:
    """Copy unset keys from the dotenv file into os.environ.

    Pydantic Settings reads .env for *its* fields, but prepare_environment()
    and debug_from_env() only see the process environment. Compose injects
    vars; bare-metal `cp .env.example .env && uvicorn` does not. Process
    environment always wins (override=False).
    """
    path = dotenv_path()
    if not path.is_file():
        return
    for key, value in dotenv_values(path).items():
        if not key or key in os.environ:
            continue
        os.environ[key] = value if value is not None else ""


class Settings(BaseSettings):
    model_config = SettingsConfigDict(
        env_file_encoding="utf-8",
        extra="ignore",
    )

    secret_key: str = "dev-secret-change-me"
    debug: bool = True
    bookclub_https: str = "auto"
    bookclub_bootstrap_invite: str = "DEV-ONLY"
    database_path: Path = _REPO_ROOT / "data" / "bookclub.db"
    bookclub_tz: str = "UTC"
    bookclub_trusted_proxies: str = "*"
    bookclub_name: str = "Bookclub"
    bookclub_theme: str = "#b44a2a"
    bookclub_theme_dark: str = ""
    bookclub_public_url: str = ""
    # Web Push. Empty private key => generated into <data>/.vapid_private.pem.
    vapid_private_key: str = ""
    vapid_subject: str = ""
    # Optional. When set, Discover search / browse / ISBN / Goodreads use
    # Google Books only (no Open Library fallback). Leave empty for OL-only.
    google_books_api_key: str = ""

    @field_validator("bookclub_public_url")
    @classmethod
    def public_url_ok(cls, value: str) -> str:
        value = (value or "").strip()
        if not value:
            return ""
        parsed = urlparse(value)
        if parsed.scheme not in {"http", "https"} or not parsed.netloc:
            raise ValueError(
                "BOOKCLUB_PUBLIC_URL must be an http(s) URL like https://books.example.com"
            )
        return f"{parsed.scheme}://{parsed.netloc}"


@lru_cache
def get_settings() -> Settings:
    apply_dotenv()
    path = dotenv_path()
    return Settings(_env_file=path if path.is_file() else None)

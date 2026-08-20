from functools import lru_cache
from pathlib import Path
from urllib.parse import urlparse

from pydantic import field_validator
from pydantic_settings import BaseSettings, SettingsConfigDict

_REPO_ROOT = Path(__file__).resolve().parents[2]


class Settings(BaseSettings):
    model_config = SettingsConfigDict(
        env_file=_REPO_ROOT / ".env",
        env_file_encoding="utf-8",
        extra="ignore",
    )

    secret_key: str = "dev-secret-change-me"
    debug: bool = True
    bookclub_https: str = "auto"
    bookclub_bootstrap_invite: str = "DEV-ONLY"
    database_path: Path = _REPO_ROOT / "data" / "bookclub.db"
    bookclub_trusted_proxies: str = "*"
    bookclub_name: str = "Bookclub"
    bookclub_theme: str = "#b44a2a"
    bookclub_theme_dark: str = ""
    bookclub_public_url: str = ""

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
    return Settings()

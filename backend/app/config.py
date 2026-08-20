from functools import lru_cache
from pathlib import Path

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


@lru_cache
def get_settings() -> Settings:
    return Settings()

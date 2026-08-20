"""Copy the SQLite database with the online backup API (safe while the app runs)."""

from __future__ import annotations

import sqlite3
import sys
from datetime import datetime, timezone
from pathlib import Path

from app.config import get_settings
from app.runtime import prepare_environment


def backup_database(source: Path, destination: Path) -> Path:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if not source.is_file():
        raise FileNotFoundError(f"Database not found: {source}")
    src = sqlite3.connect(f"file:{source}?mode=ro", uri=True)
    try:
        dst = sqlite3.connect(destination)
        try:
            src.backup(dst)
        finally:
            dst.close()
    finally:
        src.close()
    return destination


def main(argv: list[str] | None = None) -> None:
    prepare_environment()
    get_settings.cache_clear()
    settings = get_settings()
    args = sys.argv[1:] if argv is None else argv
    if args:
        dest = Path(args[0]).expanduser()
    else:
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        dest = settings.database_path.parent / f"bookclub-{stamp}.db"
    path = backup_database(settings.database_path, dest)
    print(path)


if __name__ == "__main__":
    main()

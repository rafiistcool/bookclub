import sqlite3
import tempfile
from pathlib import Path

from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import FileResponse
from starlette.background import BackgroundTask

from app.config import get_settings
from app.deps import get_current_user
from app.models import User

router = APIRouter(prefix="/api/backup", tags=["backup"])


@router.get("")
def download_backup(me: User = Depends(get_current_user)) -> FileResponse:
    source_path = Path(get_settings().database_path)
    if not source_path.is_file():
        raise HTTPException(status_code=404, detail="No database file to export")
    tmp = tempfile.NamedTemporaryFile(prefix="bookclub-backup-", suffix=".db", delete=False)
    tmp.close()
    dest = Path(tmp.name)
    source = sqlite3.connect(f"file:{source_path}?mode=ro", uri=True)
    dest_conn = sqlite3.connect(dest)
    try:
        source.backup(dest_conn)
    finally:
        dest_conn.close()
        source.close()
    return FileResponse(
        dest,
        media_type="application/vnd.sqlite3",
        filename="bookclub.db",
        background=BackgroundTask(dest.unlink, missing_ok=True),
    )

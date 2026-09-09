"""Catch drift between frontend apiErrors.ts keys and backend English details."""

from __future__ import annotations

import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
API_ERRORS = REPO / "frontend" / "src" / "i18n" / "apiErrors.ts"
APP = REPO / "backend" / "app"


def _mapped_details() -> list[str]:
    text = API_ERRORS.read_text(encoding="utf-8")
    return re.findall(r'"([^"]+)":\s*"errors\.', text)


def test_mapped_english_api_errors_still_exist_in_backend():
    details = _mapped_details()
    assert len(details) >= 10
    sources = "\n".join(path.read_text(encoding="utf-8") for path in APP.rglob("*.py"))
    missing = [detail for detail in details if detail not in sources]
    assert missing == [], f"apiErrors.ts details missing from backend/app: {missing}"

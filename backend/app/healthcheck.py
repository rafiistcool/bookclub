"""Container probe. Run as the app user (`gosu bookclub python -m app.healthcheck`)."""

from __future__ import annotations

import sys
import urllib.error
import urllib.request

HEALTH_URL = "http://127.0.0.1:8000/api/health"


def main() -> int:
    try:
        with urllib.request.urlopen(HEALTH_URL, timeout=4) as resp:
            if resp.status != 200:
                return 1
            if b'"ok"' not in resp.read():
                return 1
    except (urllib.error.URLError, TimeoutError, OSError):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

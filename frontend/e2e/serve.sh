#!/usr/bin/env bash
# Serve the built app + API on $1 with a throwaway database for the e2e smoke run.
set -euo pipefail
PORT="${1:-8010}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKEND="$HERE/../../backend"
DATA="$(mktemp -d)"

if [ ! -f "$BACKEND/app/static/index.html" ]; then
  echo "backend/app/static is missing — run 'npm run build' first" >&2
  exit 1
fi

cd "$BACKEND"
if [ -n "${PYTHON:-}" ]; then PY="$PYTHON"; elif [ -x .venv/bin/python ]; then PY=.venv/bin/python; else PY=python3; fi

export BOOKCLUB_ENV_FILE="$DATA/no-env"
export DATABASE_PATH="$DATA/e2e.db"
export DEBUG=1
export SECRET_KEY="e2e-secret-key-not-for-production-use"
export BOOKCLUB_BOOTSTRAP_INVITE="E2E-INVITE"
export BOOKCLUB_NAME="Smoke Club"
export BOOKCLUB_TZ="UTC"

exec "$PY" -m uvicorn app.main:app --host 127.0.0.1 --port "$PORT" --log-level warning

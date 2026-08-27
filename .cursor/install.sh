#!/usr/bin/env bash
set -euo pipefail

# Idempotent Cloud Agent setup for Bookclub: FastAPI backend + Vue/Vite frontend.
# Runs from the repository root after checkout.

# The default Ubuntu image ships python3.12 but not its venv module.
sudo apt-get update
sudo apt-get install -y python3.12-venv

# Backend: FastAPI app in a virtualenv (Python 3.12+), with dev extras (pytest).
cd backend
python3 -m venv .venv
./.venv/bin/pip install --upgrade pip
./.venv/bin/pip install -e ".[dev]"
cd ..

# Frontend: Vue 3 + Vite (Node 20+).
cd frontend
npm ci
cd ..

# Local dev env file (gitignored). Only created if a developer has not supplied one.
# DEBUG=1 enables CORS for Vite + /api/docs; DEV-ONLY bootstrap invite works on an empty DB.
if [ ! -f .env ]; then
  printf 'BOOKCLUB_NAME=Bookclub\nDEBUG=1\nBOOKCLUB_HTTPS=0\nSECRET_KEY=dev-secret-change-me\nBOOKCLUB_BOOTSTRAP_INVITE=DEV-ONLY\n' > .env
fi

echo "Bookclub install complete."

FROM node:22-alpine AS frontend
WORKDIR /src/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ .
RUN npm run build

FROM python:3.12-slim
# `image.source` ties the GHCR package to this repository (README, visibility,
# link back from the package page). Version/revision labels come from CI.
LABEL org.opencontainers.image.source="https://github.com/rafiistcool/bookclub" \
      org.opencontainers.image.title="Bookclub" \
      org.opencontainers.image.description="Invite-only book club with shared shelves, one club pick, and a next-up vote. One container, one SQLite file."
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends gosu \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 1000 bookclub \
    && useradd --uid 1000 --gid bookclub --create-home bookclub \
    && mkdir -p /data \
    && chown bookclub:bookclub /data

COPY backend/pyproject.toml backend/requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt
COPY backend/app ./app
COPY --from=frontend /src/backend/app/static ./app/static
COPY deploy/docker-entrypoint.sh /docker-entrypoint.sh
RUN chmod 755 /docker-entrypoint.sh

EXPOSE 8000
# Probe as the runtime user. Proxy headers are handled in-app via BOOKCLUB_TRUSTED_PROXIES.
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD gosu bookclub python -m app.healthcheck

ENTRYPOINT ["/docker-entrypoint.sh"]
CMD ["uvicorn", "app.main:app", "--host", "0.0.0.0", "--port", "8000"]

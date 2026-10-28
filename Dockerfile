# syntax=docker/dockerfile:1
#
# The production image: the Rust server (server/) with the built SPA (frontend/) as its
# STATIC_DIR. One process and no database inside; deploy/hosted/compose.yaml runs it next to
# Postgres, and deploy/hosted/README.md is the runbook.
#
#   docker build -t bookclub:local .
#   docker run --rm bookclub:local bookclub-server openapi
#
# The build argument CARGO_JOBS caps the parallel rustc jobs (default: one per CPU).

FROM node:22-trixie-slim AS frontend
WORKDIR /src/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY frontend/ ./
RUN npm run build

FROM rust:1.99-slim-trixie AS server
WORKDIR /src/server
# The sqlx query macros check against the committed server/.sqlx/; no database at build time.
ENV SQLX_OFFLINE=true
ARG CARGO_JOBS
# The dependencies first, in a layer of their own, so a source change does not rebuild them.
# The stubs stand in for the crate's own targets until the real sources arrive.
COPY server/Cargo.toml server/Cargo.lock ./
RUN mkdir src \
    && echo 'fn main() {}' > src/main.rs \
    && touch src/lib.rs \
    && cargo build --release --locked ${CARGO_JOBS:+--jobs "$CARGO_JOBS"} \
    && rm -rf src
COPY server/ ./
# COPY keeps the sources' modification times, which can be older than the stubs' build.
RUN touch src/main.rs src/lib.rs \
    && cargo build --release --locked ${CARGO_JOBS:+--jobs "$CARGO_JOBS"} \
    && install -m 0755 target/release/bookclub-server /usr/local/bin/bookclub-server

FROM debian:trixie-slim
LABEL org.opencontainers.image.title="Bookclub" \
      org.opencontainers.image.description="Bookclub, the hosted book club service: the server and its web app in one image. Needs Postgres." \
      org.opencontainers.image.source="https://git.tillhome.de/rafiistcool/bookclub" \
      org.opencontainers.image.url="https://git.tillhome.de/rafiistcool/bookclub" \
      org.opencontainers.image.documentation="https://git.tillhome.de/rafiistcool/bookclub/src/branch/main/deploy/hosted/README.md"
# ca-certificates: TLS to the SMTP server, Open Library and the DNB. uid 1000 owns /data/media;
# a bind mount there must belong to uid 1000 too, as the image's ownership does not apply to it.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 1000 bookclub \
    && useradd --uid 1000 --gid 1000 --no-create-home --home-dir /nonexistent \
       --shell /usr/sbin/nologin bookclub \
    && install -d -o 1000 -g 1000 -m 0750 /data/media
COPY --from=server /usr/local/bin/bookclub-server /usr/local/bin/bookclub-server
COPY --from=frontend /src/frontend/dist /app/static
ENV STATIC_DIR=/app/static \
    MEDIA_DIR=/data/media \
    BIND_ADDR=0.0.0.0:8080
WORKDIR /app
USER 1000:1000
EXPOSE 8080
# GET /api/health on the BIND_ADDR port, which answers 503 while the database does not. Three
# misses in a row, a minute or so, mark the container unhealthy.
HEALTHCHECK --interval=20s --timeout=5s --start-period=60s --start-interval=2s --retries=3 \
    CMD ["bookclub-server", "healthcheck"]
CMD ["bookclub-server", "serve"]

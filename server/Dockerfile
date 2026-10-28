# Rust server image. Build context is server/:
#   docker build -t bookclub-server server
FROM rust:1.99-slim-trixie AS build
WORKDIR /src
# The sqlx query macros check against the committed .sqlx/ data; no database at build time.
ENV SQLX_OFFLINE=true
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
    && cp target/release/bookclub-server /bookclub-server

FROM debian:trixie-slim
LABEL org.opencontainers.image.source="https://git.tillhome.de/rafiistcool/bookclub" \
      org.opencontainers.image.title="Bookclub server"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --user-group --no-create-home bookclub
COPY --from=build /bookclub-server /usr/local/bin/bookclub-server
USER bookclub
ENV BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
CMD ["bookclub-server"]

#!/usr/bin/env bash
# The image smoke test, CI's `image` job (.forgejo/workflows/ci.yml): build the root Dockerfile
# without pushing, boot it with compose.yaml and ci.env, and ask the app for /api/health, / and
# one of its scripts from a container on the compose network.
#
#   deploy/hosted/smoke-test.sh              build, boot, check, tear down
#   deploy/hosted/smoke-test.sh --no-build   the same with an image that is already tagged
#   deploy/hosted/smoke-test.sh --down       only the teardown (CI, after a cancelled run)
#
# A CI job runs in a container next to the Docker daemon, so neither the job's files nor the
# ports the daemon publishes are where the job is. Everything the stack needs therefore goes
# through the daemon: a helper container makes the data directory, and the requests come from a
# container on the compose network. The same steps work against a local daemon.
#
# The environment overrides ci.env's COMPOSE_PROJECT_NAME, BOOKCLUB_TAG and BOOKCLUB_DATA_DIR.
# CARGO_JOBS, if set, caps the rustc jobs of the build.
set -euo pipefail

readonly CURL_IMAGE=curlimages/curl:8.11.1
readonly HELPER_IMAGE=busybox:1.37.0

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd "$here/../.." && pwd)
env_file=$here/ci.env

build=1
down_only=0
case "${1-}" in
    "") ;;
    --no-build) build=0 ;;
    --down) down_only=1 ;;
    *)
        echo "usage: $0 [--no-build | --down]" >&2
        exit 2
        ;;
esac

log() { printf '==> %s\n' "$*"; }
fail() {
    printf 'smoke test failed: %s\n' "$*" >&2
    exit 1
}

# The environment's value of $1, else ci.env's. Compose gives the environment precedence too.
setting() {
    local value
    value=$(sed -n "s/^$1=//p" "$env_file")
    [[ -n $value ]] || fail "ci.env does not set $1"
    printf '%s' "${!1:-$value}"
}
COMPOSE_PROJECT_NAME=$(setting COMPOSE_PROJECT_NAME)
BOOKCLUB_TAG=$(setting BOOKCLUB_TAG)
BOOKCLUB_DATA_DIR=$(setting BOOKCLUB_DATA_DIR)
export COMPOSE_PROJECT_NAME BOOKCLUB_TAG BOOKCLUB_DATA_DIR
[[ $BOOKCLUB_DATA_DIR == /?* ]] ||
    fail "BOOKCLUB_DATA_DIR must be an absolute path on the Docker daemon's host"

compose() { docker compose -f "$here/compose.yaml" --env-file "$env_file" "$@"; }
image=$(compose config --images | grep '/bookclub-server:') ||
    fail "compose.yaml names no bookclub-server image"
network=${COMPOSE_PROJECT_NAME}_default
data_parent=$(dirname "$BOOKCLUB_DATA_DIR")
data_name=$(basename "$BOOKCLUB_DATA_DIR")

# Runs a command in a helper container that sees the data directory's parent at /parent.
on_daemon_host() { docker run --rm -v "$data_parent:/parent" "$HELPER_IMAGE" "$@"; }
# curl from inside the compose network; the app is http://app:8080 there.
curl_app() { docker run --rm --network "$network" "$CURL_IMAGE" -fsS --max-time 10 "$@"; }

cleanup() {
    local status=$?
    if ((status != 0)); then
        log "the stack at the failure"
        compose ps -a || true
        compose logs --no-color --tail 200 || true
    fi
    log "tearing down"
    compose down -v --remove-orphans || true
    on_daemon_host rm -rf "/parent/$data_name" || true
    if ((build)); then
        docker image rm "$image" >/dev/null || true
    fi
    exit "$status"
}

if ((down_only)); then
    cleanup
fi

log "Docker $(docker version --format '{{.Server.Version}}'), $(docker compose version)"
trap cleanup EXIT

if ((build)); then
    log "building $image"
    docker build --pull ${CARGO_JOBS:+--build-arg "CARGO_JOBS=$CARGO_JOBS"} -t "$image" "$repo"
fi
docker image inspect "$image" >/dev/null 2>&1 || fail "no image $image (build it, or drop --no-build)"

log "creating $BOOKCLUB_DATA_DIR/{media,legal} for uid 1000 on the daemon's host"
on_daemon_host rm -rf "/parent/$data_name"
on_daemon_host install -d -o 1000 -g 1000 -m 0750 \
    "/parent/$data_name/media" "/parent/$data_name/legal"

log "starting $COMPOSE_PROJECT_NAME"
compose up -d --wait --wait-timeout 180
compose ps

log "GET /api/health"
health=$(curl_app http://app:8080/api/health)
[[ $health == '{"status":"ok"}' ]] || fail "/api/health answered $health"

log "GET /"
headers=$(curl_app -D - -o /dev/null http://app:8080/)
grep -qi '^content-type: text/html' <<<"$headers" || fail "/ is not text/html: $headers"
grep -qi '^cache-control: no-store' <<<"$headers" || fail "/ is not no-store: $headers"
shell=$(curl_app http://app:8080/)
[[ $shell =~ (/assets/[^\"]+\.js) ]] || fail "the shell names no /assets/*.js"
asset=${BASH_REMATCH[1]}

log "GET $asset"
headers=$(curl_app -D - -o /dev/null "http://app:8080$asset")
grep -qi '^cache-control: .*immutable' <<<"$headers" || fail "$asset is not immutable: $headers"

log "bookclub-server healthcheck in the app container"
compose exec -T app bookclub-server healthcheck

log "the image smoke test passed"

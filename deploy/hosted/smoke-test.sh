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
# The teardown deletes what they name, so they must look like CI's: a project `bookclub-ci…`, a
# tag `ci…` and a directory `/tmp/bookclub-ci…`. Anything else (a shell that loaded the
# production .env, for one) is refused before Docker is asked anything.
#
# Before it builds, the script removes what earlier runs left behind when their teardown never
# ran (a cancelled CI run): `bookclub-ci…` stacks, `ci…` tags of the image that no container
# uses, and `/tmp/bookclub-ci…` directories, each older than STALE_MINUTES (default 120; the CI
# job times out after 60).
#
# CARGO_JOBS, if set, caps the rustc jobs of the build.
set -euo pipefail

readonly CURL_IMAGE=curlimages/curl:8.11.1
readonly HELPER_IMAGE=busybox:1.37.0
# What the names of a CI run look like; the script deletes nothing else.
readonly PROJECT_PATTERN='^bookclub-ci([-_][a-z0-9_-]*)?$'
readonly TAG_PATTERN='^ci([-_.][A-Za-z0-9_.-]*)?$'
readonly DATA_DIR_PATTERN='^/tmp/bookclub-ci[A-Za-z0-9_.-]*$'
STALE_MINUTES=${STALE_MINUTES:-120}

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
# Without trailing slashes, so that /tmp/bookclub-ci-1/ is /tmp/bookclub-ci-1.
BOOKCLUB_DATA_DIR=${BOOKCLUB_DATA_DIR%"${BOOKCLUB_DATA_DIR##*[!/]}"}
export COMPOSE_PROJECT_NAME BOOKCLUB_TAG BOOKCLUB_DATA_DIR

[[ $COMPOSE_PROJECT_NAME =~ $PROJECT_PATTERN ]] ||
    fail "COMPOSE_PROJECT_NAME=$COMPOSE_PROJECT_NAME is not a CI project (bookclub-ci…)"
[[ $BOOKCLUB_TAG =~ $TAG_PATTERN ]] ||
    fail "BOOKCLUB_TAG=$BOOKCLUB_TAG is not a CI tag (ci…)"
[[ $BOOKCLUB_DATA_DIR =~ $DATA_DIR_PATTERN ]] ||
    fail "BOOKCLUB_DATA_DIR=$BOOKCLUB_DATA_DIR is not a CI directory on the Docker daemon's host (/tmp/bookclub-ci…)"
[[ $STALE_MINUTES =~ ^[0-9]+$ ]] || fail "STALE_MINUTES=$STALE_MINUTES is not a number of minutes"

compose() { docker compose -f "$here/compose.yaml" --env-file "$env_file" "$@"; }
image=$(compose config --images | grep '/bookclub-server:') ||
    fail "compose.yaml names no bookclub-server image"
network=${COMPOSE_PROJECT_NAME}_default
data_parent=/tmp
data_name=${BOOKCLUB_DATA_DIR#/tmp/}
# Set once this run has made the data directory, which the teardown then removes.
made_data_dir=0

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
    if ((made_data_dir || down_only)); then
        on_daemon_host rm -rf "/parent/$data_name" || true
    fi
    if ((build || down_only)); then
        docker image rm "$image" >/dev/null 2>&1 || true
    fi
    exit "$status"
}

# What earlier runs left behind (see the header). Each part only logs what it cannot remove:
# another run may be removing the same leftovers.
sweep_stale_runs() {
    local cutoff id project created ref tagged
    cutoff=$(($(date +%s) - STALE_MINUTES * 60))

    # Stacks whose every container is older than the cutoff.
    local -A old=() young=()
    while IFS=$'\t' read -r id project; do
        [[ $project =~ $PROJECT_PATTERN && $project != "$COMPOSE_PROJECT_NAME" ]] || continue
        created=$(docker inspect -f '{{.Created}}' "$id" 2>/dev/null) || continue
        if (($(date -d "$created" +%s) < cutoff)); then
            old[$project]=1
        else
            young[$project]=1
        fi
    done < <(docker ps -a --filter label=com.docker.compose.project \
        --format '{{.ID}}{{"\t"}}{{.Label "com.docker.compose.project"}}')
    for project in "${!old[@]}"; do
        [[ -z ${young[$project]-} ]] || continue
        log "removing the stack $project, left behind by an earlier run"
        compose -p "$project" down -v --remove-orphans ||
            log "warning: could not remove the stack $project"
    done

    # Tags of the image that no container uses and that were set before the cutoff. A daemon
    # that does not record when it tagged an image (a zero time) keeps them.
    while IFS= read -r ref; do
        [[ ${ref##*:} =~ $TAG_PATTERN && $ref != "$image" ]] || continue
        [[ -z $(docker ps -a -q --filter "ancestor=$ref") ]] || continue
        tagged=$(docker image inspect -f '{{.Metadata.LastTagTime.Unix}}' "$ref" 2>/dev/null) ||
            continue
        ((tagged > 0 && tagged < cutoff)) || continue
        log "removing the image $ref, left behind by an earlier run"
        docker image rm "$ref" >/dev/null || log "warning: could not remove the image $ref"
    done < <(docker image ls --format '{{.Repository}}:{{.Tag}}' "${image%:*}")

    # Data directories, by the time their media/ and legal/ were made.
    on_daemon_host find /parent -mindepth 1 -maxdepth 1 -name 'bookclub-ci*' \
        ! -name "$data_name" -mmin "+$STALE_MINUTES" -print -exec rm -rf {} + ||
        log "warning: could not remove the data directories of earlier runs"
}

if ((down_only)); then
    cleanup
fi

log "Docker $(docker version --format '{{.Server.Version}}'), $(docker compose version)"
trap cleanup EXIT

log "removing what earlier runs left behind (older than $STALE_MINUTES minutes)"
sweep_stale_runs

if ((build)); then
    log "building $image"
    docker build --pull ${CARGO_JOBS:+--build-arg "CARGO_JOBS=$CARGO_JOBS"} -t "$image" "$repo"
fi
docker image inspect "$image" >/dev/null 2>&1 || fail "no image $image (build it, or drop --no-build)"

# mkdir without -p: a directory that is already there belongs to someone else (or to a run that
# was killed; `--down` removes it), and this run removes only what it made.
log "creating $BOOKCLUB_DATA_DIR/{media,legal} for uid 1000 on the daemon's host"
on_daemon_host mkdir -m 0755 "/parent/$data_name" ||
    fail "$BOOKCLUB_DATA_DIR already exists on the daemon's host (remove it with $0 --down)"
made_data_dir=1
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

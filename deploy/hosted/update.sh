#!/usr/bin/env bash
# Automatic updates of the hosted service (README.md, "Updates and rollback";
# bookclub-update.timer, every 3 minutes). The app follows the image tag that compose.yaml names
# for it, git.tillhome.de/rafiistcool/bookclub-server:${BOOKCLUB_TAG:-stable} with BOOKCLUB_TAG
# from .env:
#   1. `docker compose pull app` fetches the tag's current image. While the tag has not moved,
#      that is one request for its manifest, and nothing is downloaded.
#   2. When the app container already runs that image, there is nothing to do. A healthy app
#      pings the heartbeat with 0, an unhealthy or restarting one with 1, so a failed update keeps
#      the heartbeat failing until it is fixed. A starting app is left to the next run.
#   3. Otherwise the tag has moved (a release, or a new BOOKCLUB_TAG in .env), and the update runs:
#      a. the heartbeat's /start;
#      b. backup.sh: the new version applies its migrations when it starts, and migrations only
#         go forward, so the backup from just before is the way back;
#      c. `docker compose up -d --no-deps --wait app`, which replaces the app container (and
#         nothing else) and waits up to UPDATE_WAIT_SECONDS for Docker's healthcheck;
#      d. a check that the app runs the new image and is healthy; the image it ran before is
#         removed once no tag names it;
#      e. the heartbeat with the exit status and the end of the log.
#      A failed backup leaves the app as it was, and the next run tries again.
# It never starts an app that is not running: while the stack is down (a restore) or the app was
# stopped by hand (`docker compose stop app`, the legacy import), it changes nothing. A registry
# that does not answer gets no ping, so the heartbeat alerts only when that outlasts its grace
# time; the run exits 1.
#
#   update.sh    one check, and the update when the tag has moved
#
# Settings: backup.env next to this file (backup.env.example), or the file BACKUP_ENV names. It
# needs the backups set up, because every update starts with backup.sh.
set -euo pipefail
# The update's log holds the backup's output: readable by the owner only.
umask 077

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=SCRIPTDIR/backup-lib.sh
. "$here/backup-lib.sh"

# An image as its registry digest (repository@sha256:…) when it has one, else as its ID.
describe() {
    docker image inspect \
        -f '{{if .RepoDigests}}{{index .RepoDigests 0}}{{else}}{{.Id}}{{end}}' "$1" 2>/dev/null ||
        printf '%s' "$1"
}

# A container's state and health, e.g. `running healthy`, `running starting`, `restarting
# unhealthy`, or `running none` for an image without a healthcheck.
container_status() {
    docker inspect \
        -f '{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "$1"
}

# The update itself, from the image $2 the app runs to the image $3 that the reference $1 names
# now. Its output is the update's log; any failure stops it.
update() {
    local image=$1 old=$2 new=$3 container status
    log "$image is $(describe "$new") now; the app runs $(describe "$old")"
    log "backing up before the update (migrations only go forward)"
    "$here/backup.sh" || die "the backup failed, so the app was not updated"
    log "starting the app on $image"
    docker compose up -d --no-deps --wait --wait-timeout "$UPDATE_WAIT_SECONDS" app ||
        die "the app did not turn healthy on $image (docker compose logs app); the backup" \
            "from before the update is the latest snapshot"
    container=$(app_container)
    [[ -n $container && $(docker inspect -f '{{.Image}}' "$container") == "$new" ]] ||
        die "the app container does not run $image after the update"
    status=$(container_status "$container")
    [[ $status == "running healthy" ]] || die "the app is $status after the update"
    log "updated: the app runs $(describe "$new") and is healthy"

    # The image the app ran before, once no tag names it any more. A pinned or older tag that
    # still names it keeps it, for a rollback.
    local tags
    if tags=$(docker image inspect -f '{{join .RepoTags " "}}' "$old" 2>/dev/null) &&
        [[ -z $tags ]]; then
        if docker image rm "$old" >/dev/null; then
            log "removed the previous image"
        else
            log "warning: could not remove the previous image $old"
        fi
    fi
}

main() {
    (($# == 0)) || {
        echo "usage: $0" >&2
        exit 2
    }
    load_config
    install -d -m 0700 "$BACKUP_DIR"
    # One run at a time. The timer never overlaps itself; this covers a run by hand.
    exec 8>"$BACKUP_DIR/.update.lock"
    if ! flock -n 8; then
        log "another update.sh is running ($BACKUP_DIR/.update.lock)"
        return 0
    fi
    cd "$COMPOSE_DIR"

    local image container status
    image=$(docker compose config --images app | grep -m 1 '/bookclub-server:') ||
        die "compose.yaml names no bookclub-server image for the app"
    container=$(app_container)
    if [[ -z $container ]]; then
        log "there is no app container, so nothing is updated (the stack starts by hand:" \
            "README.md, \"First boot\")"
        return 0
    fi
    status=$(container_status "$container")
    case ${status%% *} in
        running | restarting) ;;
        *)
            log "the app is ${status%% *}, so nothing is updated (a stopped app stays stopped)"
            return 0
            ;;
    esac

    # Compose reports progress even with --quiet; only a failure's output goes to the journal.
    local pulled
    if ! pulled=$(docker compose pull --quiet app 2>&1); then
        printf '%s\n' "$pulled" >&2
        log "error: cannot pull $image; nothing was changed"
        return 1
    fi
    local new old
    new=$(docker image inspect -f '{{.Id}}' "$image")
    old=$(docker inspect -f '{{.Image}}' "$container")

    local report=$BACKUP_DIR/update.log
    if [[ $new == "$old" ]]; then
        case $status in
            "running healthy")
                log "$image is up to date ($(describe "$new")), and the app is healthy"
                heartbeat "$UPDATE_HEARTBEAT_URL" 0
                return 0
                ;;
            "running starting")
                log "$image is up to date, and the app is starting"
                return 0
                ;;
            *)
                # The last update's log stays; the ping carries only this line.
                log "error: the app runs $image ($(describe "$new")), but is $status" |
                    tee "$report.tail"
                heartbeat "$UPDATE_HEARTBEAT_URL" 1 "$report.tail"
                rm -f "$report.tail"
                return 1
                ;;
        esac
    fi

    # The update, logged to the journal and to BACKUP_DIR/update.log.
    heartbeat "$UPDATE_HEARTBEAT_URL" start
    local result
    set +e
    (
        set -e
        update "$image" "$old" "$new"
    ) 2>&1 | tee "$report"
    result=${PIPESTATUS[0]}
    set -e
    tail -n 50 "$report" >"$report.tail"
    heartbeat "$UPDATE_HEARTBEAT_URL" "$result" "$report.tail"
    rm -f "$report.tail"
    return "$result"
}

main "$@"

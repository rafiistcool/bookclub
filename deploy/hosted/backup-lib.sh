# shellcheck shell=bash
# Shared by backup.sh, restore-test.sh and update.sh (README.md, "Backups" and "Updates and
# rollback"); sourced, never run. The caller sets `here` to its own directory first.

# The scripts' settings: backup.env next to them, or the file BACKUP_ENV names.
load_config() {
    local file=${BACKUP_ENV:-$here/backup.env}
    [[ -r $file ]] || die "cannot read $file (copy backup.env.example and fill it in)"
    set -a
    # shellcheck source=SCRIPTDIR/backup.env.example disable=SC1091 # followed only with -x
    . "$file"
    set +a
    COMPOSE_DIR=${COMPOSE_DIR:-/opt/bookclub}
    MEDIA_DIR=${MEDIA_DIR:-/srv/bookclub/media}
    BACKUP_DIR=${BACKUP_DIR:-/var/lib/bookclub-backup}
    RESTIC_IMAGE=${RESTIC_IMAGE:-restic/restic:0.17.3}
    RESTIC_SSH_DIR=${RESTIC_SSH_DIR:-/root/.ssh}
    BACKUP_HOST=${BACKUP_HOST:-bookclub}
    CHECK_WEEKDAY=${CHECK_WEEKDAY:-7}
    BACKUP_HEARTBEAT_URL=${BACKUP_HEARTBEAT_URL-}
    RESTORE_TEST_HEARTBEAT_URL=${RESTORE_TEST_HEARTBEAT_URL-}
    RESTORE_POSTGRES_IMAGE=${RESTORE_POSTGRES_IMAGE:-postgres:17}
    UPDATE_HEARTBEAT_URL=${UPDATE_HEARTBEAT_URL-}
    # Checked by update.sh alone: a bad value must not stop the backups.
    UPDATE_WAIT_SECONDS=${UPDATE_WAIT_SECONDS:-300}
    [[ -n ${RESTIC_REPOSITORY-} ]] || die "$file sets no RESTIC_REPOSITORY"
    [[ -r ${RESTIC_PASSWORD_FILE-} ]] || die "cannot read RESTIC_PASSWORD_FILE ${RESTIC_PASSWORD_FILE-}"
    local dir
    for dir in "$COMPOSE_DIR" "$MEDIA_DIR" "$BACKUP_DIR"; do
        [[ $dir == /?* ]] || die "$dir is not an absolute path"
    done
    # What a snapshot holds: the dump with its counts (STAGE_DIR), and the media.
    # shellcheck disable=SC2034 # used by backup.sh and restore-test.sh
    STAGE_DIR=$BACKUP_DIR/db
}

log() { printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"; }
die() {
    log "error: $*" >&2
    exit 1
}

# The ID of the app service's container in the compose project of the current directory, running
# or not; nothing when there is none. The one-off containers of `docker compose run app …` (the
# legacy import, for one) carry the service's name too, and are left out.
app_container() {
    local id
    for id in $(docker compose ps -a -q app); do
        if [[ $(docker inspect -f '{{index .Config.Labels "com.docker.compose.oneoff"}}' "$id") != True ]]; then
            printf '%s\n' "$id"
            return 0
        fi
    done
}

# restic, from RESTIC_IMAGE, with the caller's uid. The container sees BACKUP_DIR (read-write)
# and MEDIA_DIR (read-only) at their own paths, so snapshot paths are host paths, and restores
# go into BACKUP_DIR. Its host name is BACKUP_HOST: snapshots, `forget` and `latest` group by it.
restic_() {
    local -a run=(
        docker run --rm --hostname "$BACKUP_HOST" --user "$(id -u):$(id -g)"
        -e RESTIC_REPOSITORY -e RESTIC_PASSWORD_FILE -e "RESTIC_CACHE_DIR=$BACKUP_DIR/cache"
        -e AWS_ACCESS_KEY_ID -e AWS_SECRET_ACCESS_KEY -e AWS_DEFAULT_REGION
        -v "$BACKUP_DIR:$BACKUP_DIR" -v "$MEDIA_DIR:$MEDIA_DIR:ro"
        -v "$RESTIC_PASSWORD_FILE:$RESTIC_PASSWORD_FILE:ro"
    )
    case $RESTIC_REPOSITORY in
        /*) run+=(-v "$RESTIC_REPOSITORY:$RESTIC_REPOSITORY") ;;
        # ssh reads its key, config and known_hosts there; it needs uid 0 (no passwd entry else).
        sftp:*) run+=(-v "$RESTIC_SSH_DIR:/root/.ssh:ro") ;;
    esac
    # Docker would create a missing BACKUP_DIR owned by root, with the default mode.
    install -d -m 0700 "$BACKUP_DIR"
    "${run[@]}" "$RESTIC_IMAGE" --retry-lock 10m "$@"
}

# The row count of every table in the public schema, then the newest migration, as
# "name<TAB>count" lines. backup.sh runs it in the dump's snapshot, restore-test.sh on the
# restored copy, and the two outputs must be equal.
counts_sql() {
    cat <<'SQL'
SELECT format('SELECT %L, count(*) FROM %I.%I', tablename, schemaname, tablename)
FROM pg_tables WHERE schemaname = 'public' ORDER BY tablename \gexec
SELECT '(migration)', coalesce(max(version), 0) FROM _sqlx_migrations WHERE success;
SQL
}

# psql's options for counts_sql: no psqlrc, quiet, unaligned rows separated by a tab, and an
# exit status of 3 on the first error.
# shellcheck disable=SC2034 # used by backup.sh and restore-test.sh
psql_tsv=(psql -X -q -A -t -F $'\t' -v ON_ERROR_STOP=1 -U bookclub -d bookclub)

# A heartbeat in the style of healthchecks.io: GET <url>/start, or POST <url>/<exit status> with
# a body file (the end of the log). No URL, no ping; a failed ping is only logged. The URL's path
# is the check's only credential, so it reaches curl on stdin (`-K -`), never on its command line
# (the process list), and the log names only the host.
heartbeat() {
    local url=$1 what=$2 body=${3-} host
    [[ -n $url ]] || return 0
    local -a data=()
    [[ -z $body ]] || data=(--data-binary "@$body")
    host=${url#*://}
    host=${host%%/*}
    printf 'url = "%s/%s"\n' "${url%/}" "$what" |
        curl -fsS -m 10 --retry 3 -o /dev/null "${data[@]}" -K - ||
        log "warning: the heartbeat ping $what to $host failed"
}

# The end of the log $1 for a heartbeat's body, into the file $2: its last 50 lines with every
# public id blanked. A line can name a picture (avatars/<xx>/<public id>/a, deleted during a
# backup or missed by a restore test), and the public id is the secret part of its URL: 26
# characters of [a-z0-9] (media.public_id's check in server/migrations/0001_init.sql). The
# pattern also blanks restic's 64-character ids and image digests. The lines Postgres adds to an
# error (DETAIL, CONTEXT, HINT, LINE n, QUERY, STATEMENT, and pg_restore's "Command was") are
# withheld whole: they quote the data itself, a duplicated address or a failing row with its
# password hash (security review, ADM2 merge). The ping then carries no personal data, as
# docs/legal/processors.md says of the monitoring service; the full log stays on the VM.
log_tail() {
    tail -n 50 "$1" | sed -E \
        -e 's/^(DETAIL|CONTEXT|HINT|LINE [0-9]+|QUERY|STATEMENT|Command was):.*/\1: <withheld>/' \
        -e 's/[a-z0-9]{26,}/<id>/g' >"$2"
}

# Runs the function "$3" with its arguments under the lock both scripts share, waiting up to 3 h
# for it (Sunday's restore test may start while the backup still checks). Its output goes
# to stdout (the journal) and to BACKUP_DIR/<$1>.log. The heartbeat $2 gets /start first, then
# the exit status with the last 50 lines of that log (log_tail). Returns the function's exit
# status.
supervise() {
    local name=$1 url=$2 status
    shift 2
    install -d -m 0700 "$BACKUP_DIR"
    exec 9>"$BACKUP_DIR/.lock"
    if ! flock -n 9; then
        log "waiting for the other backup or restore test to finish ($BACKUP_DIR/.lock)"
        flock -w 10800 9 || die "$BACKUP_DIR/.lock was held for 3 hours"
    fi
    heartbeat "$url" start
    set +e
    (
        set -e
        "$@"
    ) 2>&1 | tee "$BACKUP_DIR/$name.log"
    status=${PIPESTATUS[0]}
    set -e
    log_tail "$BACKUP_DIR/$name.log" "$BACKUP_DIR/$name.log.tail"
    heartbeat "$url" "$status" "$BACKUP_DIR/$name.log.tail"
    rm -f "$BACKUP_DIR/$name.log.tail"
    return "$status"
}

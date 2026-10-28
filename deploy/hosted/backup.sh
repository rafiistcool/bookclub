#!/usr/bin/env bash
# The nightly backup of the hosted service (README.md, "Backups"; bookclub-backup.timer):
#   1. pg_dump -Fc --no-owner inside the postgres container, and the row count of every table in
#      the same database snapshot (counts.tsv), into BACKUP_DIR/db;
#   2. restic backup of that directory and then of MEDIA_DIR. A picture saved while it runs is
#      on disk before its row commits, so the dump never references one restic could not see.
#      A picture deleted after the dump (a replaced avatar, a takedown, a deleted account) may
#      be gone before restic reads it: restic then saves the snapshot without it and exits 3,
#      which counts as a warning when every file it could not read was such a deletion;
#   3. restic forget --prune: 7 daily, 4 weekly and 12 monthly snapshots;
#   4. on CHECK_WEEKDAY (or with --check), restic check reading back 5 % of the data;
#   5. a heartbeat ping at the start, and with the exit status at the end.
#
#   backup.sh                 the backup
#   backup.sh --check         the backup, and the check whatever the day
#   backup.sh restic ARGS…    restic with this setup, e.g. `init`, `snapshots`, or `restore`
#                             into BACKUP_DIR (the only directory it can write)
#
# Settings: backup.env next to this file (backup.env.example), or the file BACKUP_ENV names.
set -euo pipefail
# The dump, its counts and the restored files hold personal data: readable by the owner only.
umask 077

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=SCRIPTDIR/backup-lib.sh
. "$here/backup-lib.sh"

# The file pg_dump's session writes the counts to, inside the postgres container.
readonly COUNTS_IN_CONTAINER=/tmp/bookclub-backup-counts.tsv

# MEDIA_DIR must be what the app mounts, or the backup would hold another directory's pictures.
check_media_mount() {
    local id mounted
    id=$(docker compose ps -a -q app)
    if [[ -z $id ]]; then
        log "warning: no app container, so MEDIA_DIR is not compared with its mount"
        return 0
    fi
    mounted=$(docker inspect -f \
        '{{range .Mounts}}{{if eq .Destination "/data/media"}}{{.Source}}{{end}}{{end}}' "$id")
    [[ ${mounted%/} == "${MEDIA_DIR%/}" ]] ||
        die "MEDIA_DIR is $MEDIA_DIR, but the app container mounts ${mounted:-nothing} at /data/media"
}

# One psql session in the postgres container opens a read-only snapshot, runs pg_dump in it
# (`\!`, its output is psql's stdout), and counts the rows in the same snapshot into
# COUNTS_IN_CONTAINER (`\o`). The dump and its counts therefore always agree, however busy the
# app is. The result replaces STAGE_DIR only when both are complete.
dump_database() {
    local new=$STAGE_DIR.new
    rm -rf "$new"
    install -d -m 0700 "$new"
    {
        printf '%s\n' 'BEGIN ISOLATION LEVEL REPEATABLE READ, READ ONLY;'
        printf '%s\n' "\\o $COUNTS_IN_CONTAINER"
        printf '%s\n' 'SELECT pg_export_snapshot() AS snapshot \gset'
        printf '%s\n' '\setenv BOOKCLUB_SNAPSHOT :snapshot'
        # shellcheck disable=SC2016 # $BOOKCLUB_SNAPSHOT is for the shell that \! starts.
        printf '%s\n' '\! pg_dump --format=custom --no-owner --snapshot="$BOOKCLUB_SNAPSHOT" --username=bookclub --dbname=bookclub'
        printf '%s\n' '\if :SHELL_ERROR'
        printf '%s\n' "DO \$\$ BEGIN RAISE EXCEPTION 'pg_dump failed'; END \$\$;"
        printf '%s\n' '\endif'
        counts_sql
        printf '%s\n' 'COMMIT;'
    } | docker compose exec -T postgres "${psql_tsv[@]}" >"$new/bookclub.dump"
    docker compose exec -T postgres cat "$COUNTS_IN_CONTAINER" >"$new/counts.tsv"
    docker compose exec -T postgres rm -f "$COUNTS_IN_CONTAINER"
    [[ -s $new/bookclub.dump ]] || die "pg_dump wrote nothing"
    grep -q '^(migration)' "$new/counts.tsv" || die "counts.tsv is incomplete"
    # The table of contents must read back.
    docker compose exec -T postgres pg_restore --list <"$new/bookclub.dump" >/dev/null ||
        die "pg_restore cannot read the dump"
    rm -rf "$STAGE_DIR"
    mv "$new" "$STAGE_DIR"
}

# True when every file restic could not read is one below MEDIA_DIR that no longer exists: the
# app deleted it while restic ran. $1 is restic's stderr, whose lines for those files read
# `error: lstat <path>: no such file or directory` (or open, readdirnames …). The snapshot holds
# the rest, and the dump's row for such a file is gone or taken down in the live database, which
# the restore test checks.
only_deleted_media() {
    local line path found=0
    while IFS= read -r line; do
        [[ $line == error:* ]] || continue
        [[ $line =~ ^error:\ [a-z]+\ (/.*):\ no\ such\ file\ or\ directory$ ]] || return 1
        path=${BASH_REMATCH[1]}
        [[ $path == "${MEDIA_DIR%/}/"* && ! -e $path ]] || return 1
        found=1
    done <"$1"
    ((found))
}

backup() {
    local check=$1
    cd "$COMPOSE_DIR"
    [[ -d $MEDIA_DIR ]] || die "MEDIA_DIR $MEDIA_DIR is not a directory"
    check_media_mount

    log "dumping the database"
    dump_database
    log "dump $(du -h "$STAGE_DIR/bookclub.dump" | cut -f1)," \
        "$(grep -vc '^(migration)' "$STAGE_DIR/counts.tsv") tables," \
        "$(awk -F '\t' '$1 != "(migration)" { rows += $2 } END { print rows + 0 }' \
            "$STAGE_DIR/counts.tsv") rows," \
        "migration $(awk -F '\t' '$1 == "(migration)" { print $2 }' "$STAGE_DIR/counts.tsv")"

    log "restic backup of $STAGE_DIR and $MEDIA_DIR to $RESTIC_REPOSITORY"
    # The media store's half-written files (.<variant>.<hex>.tmp) are renamed or gone a moment
    # later; the next backup has the real file. restic's stderr is kept to tell deletions apart
    # from files it could not read (exit 3, the snapshot is saved either way).
    local errors=$BACKUP_DIR/restic-backup.stderr status=0
    restic_ backup --tag bookclub --exclude '.*.tmp' --exclude '.write-probe.*' \
        "$STAGE_DIR" "$MEDIA_DIR" 2>"$errors" || status=$?
    cat "$errors" >&2
    if ((status == 3)) && only_deleted_media "$errors"; then
        log "warning: the app deleted the pictures above while restic read the media;" \
            "the snapshot was saved without them"
    elif ((status == 3)); then
        die "restic could not read the files above (exit 3; the snapshot was saved without them)"
    elif ((status != 0)); then
        die "restic backup exited $status"
    fi
    rm -f "$errors"

    log "restic forget --prune"
    restic_ forget --host "$BACKUP_HOST" --tag bookclub \
        --keep-daily 7 --keep-weekly 4 --keep-monthly 12 --prune

    if [[ $check == 1 || $(date +%u) == "$CHECK_WEEKDAY" ]]; then
        log "restic check, reading back 5 % of the data"
        restic_ check --read-data-subset=5%
    fi
    log "backup done"
}

main() {
    local check=0
    case "${1-}" in
        "") ;;
        --check) check=1 ;;
        restic)
            shift
            load_config
            restic_ "$@"
            return
            ;;
        *)
            echo "usage: $0 [--check] | $0 restic ARGS…" >&2
            exit 2
            ;;
    esac
    load_config
    supervise backup "$BACKUP_HEARTBEAT_URL" backup "$check"
}

main "$@"

#!/usr/bin/env bash
# The weekly restore test (README.md, "Backups"; bookclub-restore-test.timer). It never writes
# to the running stack:
#   1. restic restore of the latest snapshot of BACKUP_HOST into BACKUP_DIR/restore-test,
#      verifying every restored file against the repository;
#   2. a throwaway postgres (RESTORE_POSTGRES_IMAGE, no network) and pg_restore --exit-on-error,
#      whose full messages stay in BACKUP_DIR/pg_restore.err when it fails (they quote rows; the
#      log, and so the ping, gets only its own lines);
#   3. the row count of every table and the newest migration must equal the backup's counts.tsv;
#   4. every stored media file the restored database references (media.storage_key/<variant> of
#      the rows not taken down) must be in the restored MEDIA_DIR and not empty. A missing one
#      that the live database no longer references either was deleted by the app while the
#      backup ran (backup.sh), and is logged but no failure;
#   5. a heartbeat ping at the start, and with the exit status at the end.
# The throwaway container and the restored files are removed in any case.
#
# Settings: backup.env next to this file (backup.env.example), or the file BACKUP_ENV names.
set -euo pipefail
# The dump, its counts and the restored files hold personal data: readable by the owner only.
umask 077

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=SCRIPTDIR/backup-lib.sh
. "$here/backup-lib.sh"

readonly CONTAINER=bookclub-restore-test

# Every media object the database says is stored: <storage_key>/<variant>.
readonly MEDIA_KEYS_SQL="SELECT m.storage_key || '/' || v.variant
FROM media m CROSS JOIN LATERAL unnest(m.variants) AS v(variant)
WHERE m.storage_key IS NOT NULL AND m.removed_at IS NULL
ORDER BY 1;"

# The media keys the running stack's database references now, or a failure when it does not
# answer (the stack is down, as during a restore).
live_media_keys() {
    (cd "$COMPOSE_DIR" && docker compose exec -T postgres "${psql_tsv[@]}" -c "$MEDIA_KEYS_SQL")
}

# pg_restore's own lines from its messages on stdin, the first 10: "pg_restore: error: …" and
# "pg_restore: warning: …", which name the table, index or constraint. A value quoted after a
# colon (`invalid input syntax for type uuid: "…"`) is withheld, and so are the lines Postgres
# adds (DETAIL, CONTEXT, …), which quote the data (security review, ADM2 merge).
pg_messages() {
    { grep -E '^pg_restore: (error|warning):' || true; } |
        head -n 10 | sed -E 's/: "[^"]*("|$)/: <withheld>/g'
}

# The scratch space: the throwaway container and WORK, the restored files and the comparisons.
# A global, because the EXIT trap runs after restore_test has returned.
WORK=
remove_scratch() {
    docker rm -fv "$CONTAINER" >/dev/null 2>&1 || true
    [[ -z $WORK ]] || rm -rf "$WORK"
}

restore_test() {
    WORK=$BACKUP_DIR/restore-test
    local target=$WORK/files
    trap remove_scratch EXIT
    remove_scratch
    install -d -m 0700 "$WORK" "$target"

    log "restoring the latest snapshot of $BACKUP_HOST from $RESTIC_REPOSITORY"
    restic_ restore latest --host "$BACKUP_HOST" --tag bookclub --target "$target" --verify
    local stage=$target$STAGE_DIR media=$target$MEDIA_DIR
    [[ -s $stage/bookclub.dump && -s $stage/counts.tsv ]] ||
        die "the snapshot holds no dump or no counts.tsv under $STAGE_DIR"
    [[ -d $media ]] || die "the snapshot holds no $MEDIA_DIR"

    log "starting a throwaway $RESTORE_POSTGRES_IMAGE"
    docker run -d --name "$CONTAINER" --network none \
        -e POSTGRES_USER=bookclub -e POSTGRES_PASSWORD=restore-test -e POSTGRES_DB=bookclub \
        -v "$stage:/restore:ro" "$RESTORE_POSTGRES_IMAGE" >/dev/null
    # The image's first start runs a temporary server on the socket only; TCP on loopback
    # answers once the real one runs.
    local waited=0
    until docker exec "$CONTAINER" pg_isready -q -h 127.0.0.1 -U bookclub -d bookclub; do
        ((waited++ < 120)) || die "the throwaway postgres did not start within 120 s"
        sleep 1
    done

    log "pg_restore --exit-on-error"
    # Postgres's messages quote the rows they fail on (a duplicated address, a failing row with
    # its password hash), and the end of this log goes into the heartbeat's ping. So its full
    # messages go to a file on the VM, and the log gets pg_restore's own lines (pg_messages).
    local messages=$BACKUP_DIR/pg_restore.err
    if ! docker exec "$CONTAINER" pg_restore --exit-on-error --no-owner \
        -U bookclub -d bookclub /restore/bookclub.dump 2>"$messages"; then
        pg_messages <"$messages"
        die "pg_restore failed; its full messages are in $messages on the VM"
    fi
    pg_messages <"$messages"
    rm -f "$messages"

    log "comparing the row counts and the migration with the backup's counts.tsv"
    counts_sql | docker exec -i "$CONTAINER" "${psql_tsv[@]}" >"$WORK/counts.tsv"
    diff -u "$stage/counts.tsv" "$WORK/counts.tsv" ||
        die "the restored database does not match the backup's counts.tsv"

    log "checking the media files the database references"
    docker exec "$CONTAINER" "${psql_tsv[@]}" -c "$MEDIA_KEYS_SQL" >"$WORK/media-keys"
    local key checked=0
    : >"$WORK/missing"
    while IFS= read -r key; do
        checked=$((checked + 1))
        [[ -s $media/$key ]] || printf '%s\n' "$key" >>"$WORK/missing"
    done <"$WORK/media-keys"

    # The app deletes a picture's files right after the commit that deletes or takes down its
    # row. A deletion between the dump and restic's read of the media therefore leaves the dump
    # referencing files the snapshot does not hold. That is no loss when the live database no
    # longer references them either; a file it still references is.
    local deleted=0 lost=0
    if [[ -s $WORK/missing ]]; then
        if ! live_media_keys >"$WORK/live-keys"; then
            log "warning: the live database does not answer, so every missing file counts"
            cp "$WORK/media-keys" "$WORK/live-keys"
        fi
        while IFS= read -r key; do
            if grep -qxF -e "$key" "$WORK/live-keys"; then
                lost=$((lost + 1))
                ((lost > 10)) || log "missing or empty: $key"
            else
                deleted=$((deleted + 1))
                ((deleted > 10)) || log "deleted after the backup, no longer referenced: $key"
            fi
        done <"$WORK/missing"
    fi
    ((lost == 0)) || die "$lost of $checked media files are missing or empty"
    local files="$checked media files"
    ((deleted == 0)) || files="$((checked - deleted)) media files ($deleted deleted after the backup)"

    log "restore test passed: $(grep -vc '^(migration)' "$WORK/counts.tsv") tables," \
        "$(awk -F '\t' '$1 != "(migration)" { rows += $2 } END { print rows + 0 }' \
            "$WORK/counts.tsv") rows," \
        "migration $(awk -F '\t' '$1 == "(migration)" { print $2 }' "$WORK/counts.tsv")," \
        "$files"
}

main() {
    (($# == 0)) || {
        echo "usage: $0" >&2
        exit 2
    }
    load_config
    supervise restore-test "$RESTORE_TEST_HEARTBEAT_URL" restore_test
}

main "$@"

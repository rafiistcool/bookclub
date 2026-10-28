#!/usr/bin/env bash
# Checks an import-legacy run with counts the importer did not make (#52, #38).
#
# Runs legacy-counts.sql (sqlite3) on a private copy of the legacy database and
# target-counts.sql (psql) on the target, prints the counts side by side and per user, checks
# the rows the report has no line for (club memberships, invite redemptions), and compares
# id|username|password_hash of every account.
#
# Usage:
#   compare.sh [--report REPORT] LEGACY_DB [-- PSQL_COMMAND...]
#
#   LEGACY_DB      the bookclub.db that import-legacy read. It is copied with its -wal and
#                  -journal files (next to the file a symlink points to, where SQLite looks for
#                  them) into a private temporary directory; the file is never opened.
#   --report FILE  the report of the real import-legacy run (real.txt). Every legacy count must
#                  equal its source column and every target count its target column.
#   PSQL_COMMAND   a psql that connects to the target and reads SQL on stdin. Default:
#                  psql "$DATABASE_URL". On the app VM, from the compose directory:
#                  -- docker compose exec -T postgres psql -U bookclub -d bookclub
#
# Run it after the real import and before anyone signs in: a login re-hashes a legacy password
# hash, and that account then differs. Password hashes are never printed, only ids and names.
# Without --report, the counts are only printed: merges and skips make legacy and target differ.
#
# Exit status: 0 when the accounts and the rows without a report line match and, with
# --report, every count matches the report; 1 on a difference; 2 on a usage error or a failed
# query.
#
# Needs bash, awk and sqlite3 (Debian: apt-get install sqlite3); SQLITE3 names another binary.

set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
sqlite=${SQLITE3:-sqlite3}

usage() {
	awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "${BASH_SOURCE[0]}"
}

fail() {
	printf 'compare.sh: %s\n' "$1" >&2
	exit 2
}

report=""
legacy=""
while [[ $# -gt 0 ]]; do
	case $1 in
	--report)
		[[ $# -ge 2 ]] || fail "--report needs a file"
		report=$2
		shift 2
		;;
	--report=*)
		report=${1#--report=}
		shift
		;;
	-h | --help)
		usage
		exit 0
		;;
	--)
		shift
		break
		;;
	-*) fail "unknown option $1 (see --help)" ;;
	*)
		[[ -z $legacy ]] || fail "name one legacy database (see --help)"
		legacy=$1
		shift
		;;
	esac
done

[[ -n $legacy ]] || fail "name the legacy database (see --help)"
[[ -f $legacy ]] || fail "$legacy is not a file"
# SQLite looks for the -wal and -journal of a symlinked database next to the file the link
# points to, and so does the importer: copy them from there.
legacy=$(readlink -f -- "$legacy")
if [[ -n $report ]]; then
	[[ -f $report ]] || fail "$report is not a file"
	if head -n 1 -- "$report" | grep -q '^DRY RUN'; then
		fail "$report is the report of a dry run; compare after the real run"
	fi
	grep -q '^users  *[0-9][0-9]* → ' -- "$report" || fail "$report is not an import-legacy report"
fi
if [[ $# -eq 0 ]]; then
	[[ -n ${DATABASE_URL:-} ]] || fail "set DATABASE_URL or pass the psql command after --"
	set -- psql "$DATABASE_URL"
fi
command -v "$sqlite" >/dev/null || fail "$sqlite not found (Debian: apt-get install sqlite3)"

work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
chmod 700 "$work"

# Even read-only, SQLite needs a writable directory for a database in WAL mode, and the
# committed rows a checkpoint has not moved yet are in the -wal file. A hot -journal makes
# sqlite3 refuse the copy instead of reading a half-written file.
cp -- "$legacy" "$work/legacy.db"
for sidecar in -wal -journal; do
	if [[ -e $legacy$sidecar ]]; then
		cp -- "$legacy$sidecar" "$work/legacy.db$sidecar"
	fi
done

"$sqlite" -readonly "$work/legacy.db" <"$here/legacy-counts.sql" >"$work/legacy.txt" ||
	fail "legacy-counts.sql failed on $legacy"
"$@" <"$here/target-counts.sql" >"$work/target.txt" ||
	fail "target-counts.sql failed (see the psql message above)"
grep -q '^count|users|' "$work/legacy.txt" || fail "legacy-counts.sql printed no counts"
grep -q '^count|users|' "$work/target.txt" || fail "target-counts.sql printed no counts"

status=0

# The counts side by side, then per user. With a report, awk exits 1 on any mismatch.
rc=0
awk -F '|' '
	function cell(key, values) { return (key in values) ? values[key] : "-" }
	function add(key) { if (!(key in seen)) { seen[key] = 1; order[++rows] = key } }

	file == "legacy" && $1 == "count" { legacy[$2] = $3; add($2); next }
	file == "target" && $1 == "count" { target[$2] = $3; add($2); next }
	file == "legacy" && $1 == "user" { lu[$2, $4] = $5; name[$2] = $3; user($2); what($4); next }
	file == "target" && $1 == "user" { tu[$2, $4] = $5; name[$2] = $3; user($2); what($4); next }
	file == "legacy" && $1 == "same" { lsame[$2] = $3; same($2); next }
	file == "target" && $1 == "same" { tsame[$2] = $3; same($2); next }
	file == "report" {
		# Entity lines: "<name>   <source|-> → <target|DROPPED> [detail]". A report saved through
		# a terminal (docker compose run without -T) ends its lines with CR.
		sub(/\r$/, "")
		if ($0 ~ /^[a-z_]+ +[-0-9]+ → [A-Z0-9]+/) {
			split($0, f, / +/)
			source[f[1]] = f[2]
			dest[f[1]] = f[4]
			reported = 1
		}
		next
	}
	function user(id) { if (!(id in users)) { users[id] = 1; ids[++nids] = id } }
	function what(w) { if (!(w in whats)) { whats[w] = 1; cols[++ncols] = w } }
	function same(key) { if (!(key in sames)) { sames[key] = 1; sameorder[++nsames] = key } }

	END {
		bad = 0
		# "→" is one column but three bytes: the header pads two bytes less.
		printf "%-38s %8s %8s  %-16s %s\n", "entity", "legacy", "target", (reported ? "report" : ""), (reported ? "check" : "")
		for (i = 1; i <= rows; i++) {
			key = order[i]
			shown = ""
			check = ""
			if (key ~ /\+/) {
				# Several report lines that land in one target table.
				n = split(key, parts, "+")
				sum = 0
				missing = ""
				shown = "→ "
				for (j = 1; j <= n; j++) {
					if (!(parts[j] in dest)) missing = parts[j]
					sum += dest[parts[j]]
					shown = shown (j > 1 ? "+" : "") dest[parts[j]]
				}
				if (reported) {
					if (missing != "") check = "MISMATCH: no report line " missing
					else if (sum != target[key]) check = "MISMATCH"
					else check = "ok"
				}
			} else if (reported) {
				if (!(key in source)) {
					check = "MISMATCH: no report line"
				} else {
					shown = source[key] " → " dest[key]
					check = "ok"
					if ((key in legacy) && legacy[key] != source[key]) check = "MISMATCH"
					if ((key in target) && target[key] != dest[key]) check = "MISMATCH"
				}
			}
			if (check != "" && check != "ok") bad = 1
			printf "%-38s %8s %8s  %-18s %s\n", key, cell(key, legacy), cell(key, target), (reported ? shown : ""), check
		}

		# Rows import-legacy creates without a report line: legacy and target always agree.
		printf "\n%-38s %8s %8s  %s\n", "without a report line", "legacy", "target", "check"
		for (i = 1; i <= nsames; i++) {
			key = sameorder[i]
			l = cell(key, lsame)
			t = cell(key, tsame)
			check = (l != "-" && l == t) ? "ok" : "MISMATCH"
			if (check != "ok") bad = 1
			printf "%-38s %8s %8s  %s\n", key, l, t, check
		}

		printf "\nper user (legacy->target where they differ)\n"
		printf "%-24s", "user (id)"
		for (c = 1; c <= ncols; c++) printf " %-10s", cols[c]
		printf "\n"
		for (u = 1; u <= nids; u++) {
			id = ids[u]
			printf "%-24s", name[id] " (" id ")"
			for (c = 1; c <= ncols; c++) {
				l = ((id, cols[c]) in lu) ? lu[id, cols[c]] : "-"
				t = ((id, cols[c]) in tu) ? tu[id, cols[c]] : "-"
				printf " %-10s", (l == t ? l : l "->" t)
			}
			printf "\n"
		}
		exit bad
	}
' file=legacy "$work/legacy.txt" file=target "$work/target.txt" file=report "${report:-/dev/null}" || rc=$?
case $rc in
0) ;;
1) status=1 ;;
*) fail "awk failed" ;;
esac

# The accounts: import-legacy copies id, username and hash byte for byte.
grep '^hash|' "$work/legacy.txt" | cut -d '|' -f 2- >"$work/legacy-accounts" || true
grep '^hash|' "$work/target.txt" | cut -d '|' -f 2- >"$work/target-accounts" || true
accounts=$(wc -l <"$work/legacy-accounts")
if cmp -s "$work/legacy-accounts" "$work/target-accounts"; then
	printf '\naccounts: all %d match (id|username|password_hash)\n' "$accounts"
else
	status=1
	printf '\naccounts: DIFFERENT (id|username; < legacy only, > target only)\n'
	diff "$work/legacy-accounts" "$work/target-accounts" | grep '^[<>]' | cut -d '|' -f 1-2 || true
	printf 'A login re-hashes a legacy password hash: compare before anyone signs in.\n'
fi

if [[ -n $report ]]; then
	if [[ $status -eq 0 ]]; then
		printf 'result: every count matches %s; the members, redemptions and accounts match\n' "$report"
	else
		printf 'result: DIFFERENCES (see MISMATCH and accounts above)\n'
	fi
fi
exit "$status"

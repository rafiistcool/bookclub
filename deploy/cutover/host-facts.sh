#!/usr/bin/env bash
# Reads the facts the private move needs from the host that runs the Python app (#52,
# docs/phase3/work-plan.md §5.4, docs/phase3/private-move.md step 1). Read-only: it changes
# nothing on the host, and it prints no secret, no password hash and no row of the app's data.
#
# Usage, as root on the host:
#   host-facts.sh OLD_COMPOSE_DIR HOSTNAME
#
#   OLD_COMPOSE_DIR  the directory of the Python app's compose project (service `bookclub`,
#                    its .env, data/bookclub.db)
#   HOSTNAME         the name the members open, e.g. books.example.com (no scheme, no path)
#
# It prints:
#   - the OS, the machine (the image is amd64 only), Docker Engine, compose and systemd;
#   - `docker compose ls` and every container with its image, auto-updaters flagged;
#   - the old container: image, state, restart policy, published ports, networks and aliases,
#     mounts, compose project, and the size of bookclub.db and its -wal and -journal;
#   - BOOKCLUB_NAME, BOOKCLUB_TZ, BOOKCLUB_PORT and BOOKCLUB_PUBLIC_URL, by exact key, from
#     the old .env and from the old container's environment (never another key);
#   - whether root's Docker config names git.tillhome.de (presence only);
#   - the old container's established peers on its port, sampled for PEER_SECONDS (default 10)
#     seconds: run it while someone uses the app. Behind a proxy that is the proxy's address,
#     which TRUSTED_PROXIES takes;
#   - who listens on 80 and 443 on this host;
#   - `curl -sI https://HOSTNAME/`: the status line, Strict-Transport-Security and
#     Permissions-Policy;
#   - free disk under /srv and Docker's root, the disks, and mounted NAS shares (for backups);
#   - whether sqlite3 is installed, whether /opt/bookclub and /srv/bookclub exist, and whether
#     the old data directory holds a path of the new layout (M6 deletes it whole);
#   - the checks that stop the move, and the notes for the runbook.
#
# Exit status: 0 when no check stops the move, 1 when one does, 2 on a usage error.
#
# Needs bash, docker with the compose plugin, curl, nsenter and ss (util-linux, iproute2),
# lsblk, findmnt and df. A missing tool is reported, not fatal.

set -uo pipefail

REGISTRY=git.tillhome.de
OLD_SERVICE=${OLD_SERVICE:-bookclub}
PEER_SECONDS=${PEER_SECONDS:-10}
MIN_DOCKER=25.0.0
MIN_COMPOSE=2.24.0
ENV_KEYS=(BOOKCLUB_NAME BOOKCLUB_TZ BOOKCLUB_PORT BOOKCLUB_PUBLIC_URL)

usage() {
	awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "${BASH_SOURCE[0]}"
}

if [[ ${1:-} == -h || ${1:-} == --help ]]; then
	usage
	exit 0
fi
if [[ $# -ne 2 ]]; then
	echo "usage: $0 OLD_COMPOSE_DIR HOSTNAME   (--help for more)" >&2
	exit 2
fi
old=$(cd -- "$1" 2>/dev/null && pwd -P) || {
	echo "host-facts.sh: $1 is not a directory" >&2
	exit 2
}
host=${2#https://}
host=${host#http://}
host=${host%%/*}
if [[ ! $host =~ ^[A-Za-z0-9.-]+(:[0-9]+)?$ ]]; then
	echo "host-facts.sh: $2 is not a host name" >&2
	exit 2
fi
if [[ ! $PEER_SECONDS =~ ^[0-9]+$ ]]; then
	echo "host-facts.sh: PEER_SECONDS must be a whole number" >&2
	exit 2
fi

stops=()
notes=()

section() {
	printf '\n== %s ==\n' "$1"
}

have() {
	command -v "$1" >/dev/null 2>&1
}

# True when version $1 is at least $2 (both dotted, a leading v and a suffix ignored).
at_least() {
	local have=${1#v} want=$2
	have=${have%%[!0-9.]*}
	[[ -n $have ]] || return 1
	[[ $(printf '%s\n%s\n' "$want" "$have" | sort -V | head -n 1) == "$want" ]]
}

# The value of KEY in an env file, by exact key (an `export ` prefix and quotes allowed).
env_value() {
	local file=$1 key=$2 line
	line=$(grep -m 1 -E "^(export[[:space:]]+)?${key}=" "$file" 2>/dev/null) || return 1
	line=${line#export }
	line=${line#"${key}="}
	line=${line%\"}
	line=${line#\"}
	line=${line%\'}
	line=${line#\'}
	printf '%s' "$line"
}

section "System"
if [[ -r /etc/os-release ]]; then
	# shellcheck disable=SC1091
	(. /etc/os-release && echo "os: ${PRETTY_NAME:-unknown}")
else
	echo "os: unknown (no /etc/os-release)"
fi
arch=$(uname -m)
echo "machine: $arch"
[[ $arch == x86_64 ]] || stops+=("the machine is $arch: the image is amd64 only")
if have systemctl; then
	echo "systemd: $(systemctl --version 2>/dev/null | head -n 1)"
else
	echo "systemd: not found"
	stops+=("no systemd: the backup, restore-test and update timers need it")
fi
root=false
[[ $(id -u) -eq 0 ]] && root=true
echo "user: $(id -un) (uid $(id -u))"
$root || notes+=("not run as root: the peers and root's Docker config were not read")

section "Docker"
if ! have docker; then
	echo "docker: not found"
	stops+=("Docker is not installed")
else
	docker_version=$(docker version --format '{{.Server.Version}}' 2>/dev/null) || docker_version=""
	compose_version=$(docker compose version --short 2>/dev/null) || compose_version=""
	echo "engine: ${docker_version:-unknown}"
	echo "compose: ${compose_version:-not found}"
	echo "root dir: $(docker info --format '{{.DockerRootDir}}' 2>/dev/null || echo unknown)"
	at_least "$docker_version" "$MIN_DOCKER" ||
		stops+=("Docker Engine ${docker_version:-unknown} is older than $MIN_DOCKER")
	at_least "$compose_version" "$MIN_COMPOSE" ||
		stops+=("compose ${compose_version:-missing} is older than $MIN_COMPOSE")

	section "Compose projects (docker compose ls)"
	docker compose ls -a 2>&1
	# grep without -q reads to the end: with pipefail, an early exit could fail the writer.
	if docker compose ls -a --format json 2>/dev/null | grep '"Name":"bookclub"' >/dev/null; then
		notes+=("a compose project named bookclub exists: the new .env sets COMPOSE_PROJECT_NAME=bookclub-server")
	fi

	section "Containers"
	docker ps -a --format '{{.Names}}\t{{.Image}}\t{{.Status}}' 2>&1 |
		while IFS=$'\t' read -r name image status; do
			flag=""
			if [[ "$name $image" =~ [Ww]atchtower|[Oo]uroboros|[Ss]hepherd|what.?s.?up.?docker|wud ]]; then
				flag="   <- an auto-updater"
			fi
			printf '%s  %s  (%s)%s\n' "$name" "$image" "$status" "$flag"
		done
	if docker ps -a --format '{{.Names}} {{.Image}}' 2>/dev/null |
		grep -iE 'watchtower|ouroboros|shepherd|whats-up-docker|wud' >/dev/null; then
		notes+=("an auto-updater runs: exclude the new stack from it (or stop it) before installing (§5.4)")
	fi
fi

section "The old app ($old)"
container=""
if have docker; then
	container=$(docker ps -aq \
		--filter "label=com.docker.compose.project.working_dir=$old" \
		--filter "label=com.docker.compose.service=$OLD_SERVICE" 2>/dev/null | head -n 1)
	if [[ -z $container ]]; then
		container=$(cd "$old" && docker compose ps -aq "$OLD_SERVICE" 2>/dev/null | head -n 1)
	fi
fi
pid=0
container_port=""
data=""
if [[ -z $container ]]; then
	echo "container: none found for service $OLD_SERVICE in $old"
	notes+=("no old container found: check OLD_COMPOSE_DIR, or set OLD_SERVICE")
else
	docker inspect -f 'container: {{.Name}}
image: {{.Config.Image}}
image id: {{.Image}}
state: {{.State.Status}}{{if .State.Health}} ({{.State.Health.Status}}){{end}}
restart policy: {{.HostConfig.RestartPolicy.Name}}
compose project: {{index .Config.Labels "com.docker.compose.project"}}
compose files: {{index .Config.Labels "com.docker.compose.project.config_files"}}
watchtower label: {{index .Config.Labels "com.centurylinklabs.watchtower.enable"}}
network mode: {{.HostConfig.NetworkMode}}' "$container"
	echo "published ports:"
	docker inspect -f '{{range $port, $bindings := .NetworkSettings.Ports}}{{range $bindings}}  {{.HostIp}}:{{.HostPort}} -> {{$port}}
{{end}}{{if not $bindings}}  (not published) {{$port}}
{{end}}{{end}}' "$container" | sed '/^$/d'
	echo "networks:"
	docker inspect -f '{{range $name, $net := .NetworkSettings.Networks}}{{$name}} aliases={{join $net.Aliases ","}}
{{end}}' "$container" |
		while read -r network aliases; do
			[[ -n $network ]] || continue
			members=$(docker network inspect -f '{{range .Containers}}{{.Name}} {{end}}' "$network" 2>/dev/null)
			echo "  $network $aliases containers: ${members:-?}"
		done
	echo "mounts:"
	docker inspect -f '{{range .Mounts}}  {{.Type}} {{.Source}} -> {{.Destination}}{{if not .RW}} (read-only){{end}}
{{end}}' "$container" | sed '/^$/d'
	data=$(docker inspect -f '{{range .Mounts}}{{if eq .Destination "/data"}}{{.Source}}{{end}}{{end}}' "$container")
	if [[ -n $data && -d $data ]]; then
		echo "data files (bytes):"
		for file in "$data"/bookclub.db "$data"/bookclub.db-wal "$data"/bookclub.db-journal; do
			[[ -e $file ]] && printf '  %s %s\n' "${file##*/}" "$(stat -c %s "$file")"
		done
		[[ -e $data/bookclub.db-journal ]] &&
			notes+=("bookclub.db-journal exists: the app did not stop cleanly at some point (cutover.md 3.1)")
	fi
	pid=$(docker inspect -f '{{.State.Pid}}' "$container")
	container_port=$(docker inspect -f '{{range $port, $b := .NetworkSettings.Ports}}{{$port}} {{end}}' "$container" |
		tr ' ' '\n' | grep -m 1 -E '^[0-9]+/tcp$')
	container_port=${container_port%/tcp}
	if [[ -z $container_port ]]; then
		container_port=$(docker inspect -f '{{range $port, $v := .Config.ExposedPorts}}{{$port}} {{end}}' "$container" |
			tr ' ' '\n' | grep -m 1 -E '^[0-9]+/tcp$')
		container_port=${container_port%/tcp}
	fi
	container_port=${container_port:-8000}
	echo "container port: $container_port"
fi

section "The old settings (by exact key)"
if [[ -f $old/.env ]]; then
	for key in "${ENV_KEYS[@]}"; do
		if value=$(env_value "$old/.env" "$key"); then
			echo ".env $key=$value"
		else
			echo ".env $key (not set)"
		fi
	done
else
	echo ".env: none in $old"
fi
if [[ -n $container ]]; then
	# Only these keys leave the environment; SECRET_KEY and the rest are never printed.
	environment=$(docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "$container")
	for key in "${ENV_KEYS[@]}"; do
		line=$(grep -m 1 -E "^${key}=" <<<"$environment") || line="$key (not set)"
		echo "container $line"
	done
fi

section "Registry"
docker_config=${DOCKER_CONFIG:-/root/.docker}/config.json
if ! $root; then
	echo "$REGISTRY: not checked (not root)"
elif [[ ! -r $docker_config ]]; then
	echo "$REGISTRY: no readable $docker_config"
	notes+=("root is not logged in to $REGISTRY: Rafael runs docker login with a read:package token (§5.4)")
elif grep -qF "\"$REGISTRY\"" "$docker_config"; then
	echo "$REGISTRY: named in $docker_config"
else
	echo "$REGISTRY: not named in $docker_config"
	notes+=("root is not logged in to $REGISTRY: Rafael runs docker login with a read:package token (§5.4)")
fi

section "Peers of the old container on port ${container_port:-?} (${PEER_SECONDS} s)"
if $root && [[ $pid -gt 0 ]] && have nsenter && have ss; then
	peers=""
	for ((second = 0; second < PEER_SECONDS || second == 0; second++)); do
		peers+=$(nsenter -n -t "$pid" ss -Htn state established "( sport = :$container_port )" 2>/dev/null |
			awk '{ peer = $NF; sub(/:[0-9]+$/, "", peer); print peer }')
		peers+=$'\n'
		((PEER_SECONDS > 0)) && sleep 1
	done
	if [[ -n ${peers//$'\n'/} ]]; then
		echo "address, connections seen over the samples:"
		grep -v '^$' <<<"$peers" | sort | uniq -c | awk '{ printf "  %s %s\n", $2, $1 }'
	else
		echo "none: open the app in a browser and run it again"
		notes+=("no peer seen on the old app's port: run it again while someone uses the app")
	fi
else
	echo "not read: the container does not run, or nsenter or ss is missing, or not root"
fi

section "Listening on 80 and 443"
if have ss; then
	listeners=$(ss -Htlnp '( sport = :80 or sport = :443 )' 2>/dev/null |
		awk '{ process = $NF; sub(/^users:\(\("/, "", process); sub(/".*/, "", process); print "  " $4 " " process }')
	echo "${listeners:-  nothing}"
else
	echo "ss not found"
fi

section "https://$host/"
if have curl; then
	headers=$(curl -sS -I -m 15 "https://$host/" 2>&1) || true
	grep -iE '^(HTTP/|strict-transport-security:|permissions-policy:|curl:)' <<<"$headers" | tr -d '\r'
	status=$(head -n 1 <<<"$headers" | awk '/^HTTP\// { print $2 }')
	if [[ -z $status ]]; then
		stops+=("https://$host/ does not answer over HTTPS (the session cookie is Secure)")
	fi
	if grep -qiE '^permissions-policy:.*camera=\(\)' <<<"$headers"; then
		notes+=("the proxy sends camera=(): the scanner needs camera=(self) in its Permissions-Policy (not a stop)")
	fi
else
	echo "curl not found"
	stops+=("curl is missing: HTTPS could not be checked (and the heartbeats need it)")
fi

section "Disks"
for path in /srv "$(docker info --format '{{.DockerRootDir}}' 2>/dev/null || echo /var/lib/docker)"; do
	target=$path
	while [[ ! -e $target ]]; do target=$(dirname "$target"); done
	df -h --output=target,size,avail "$target" 2>/dev/null | tail -n 1 | awk -v path="$path" '{ print "  " path ": " $3 " free of " $2 " (on " $1 ")" }'
done
if have lsblk; then
	echo "disks:"
	lsblk -o NAME,SIZE,TYPE,MOUNTPOINT 2>/dev/null | sed 's/^/  /'
fi
if have findmnt; then
	shares=$(findmnt -rn -t nfs,nfs4,cifs,smb3 -o TARGET,SOURCE,FSTYPE 2>/dev/null)
	if [[ -n $shares ]]; then
		echo "network shares:"
		while IFS= read -r share; do echo "  $share"; done <<<"$shares"
	else
		echo "network shares: none"
	fi
fi

section "Tools and paths"
if have sqlite3; then
	echo "sqlite3: $(sqlite3 --version | awk '{ print $1 }')"
else
	echo "sqlite3: not installed"
	notes+=("sqlite3 is missing: install it for compare.sh, or use the sqlite3 stand-in of docs/phase3/private-move.md 7.4")
fi
for path in /opt/bookclub /srv/bookclub; do
	if [[ -e $path ]]; then
		real=$(cd "$path" 2>/dev/null && pwd -P)
		echo "$path: exists"
		if [[ $real == "$old" || $old == "$real"/* || (-n $data && ($data == "$real" || $data == "$real"/*)) ]]; then
			notes+=("$path holds the old app: the new stack goes to /opt/bookclub-server and /srv/bookclub-server (§5.4)")
		else
			notes+=("$path exists: check what it holds before installing there")
		fi
	else
		echo "$path: absent"
	fi
done
# M6 deletes the old data directory whole, so the new stack, its data and its backups must lie
# outside it: an old BOOKCLUB_DATA of /srv holds /srv/bookclub (private-move.md 2.3).
if [[ -n $data ]]; then
	real_data=$(realpath -m -- "$data" 2>/dev/null) || real_data=$data
	for path in /opt/bookclub /srv/bookclub /opt/bookclub-server /srv/bookclub-server /srv/bookclub-backup; do
		real=$(realpath -m -- "$path" 2>/dev/null) || real=$path
		if [[ $real == "${real_data%/}"/* ]]; then
			notes+=("the old data directory $data holds $path: the new stack, its data and the backups go outside it, since M6 deletes it whole (private-move.md 2.3)")
		fi
	done
fi

section "Checks"
for note in "${notes[@]}"; do
	echo "note: $note"
done
if [[ ${#stops[@]} -eq 0 ]]; then
	echo "no check stops the move"
	exit 0
fi
for stop in "${stops[@]}"; do
	echo "STOP: $stop"
done
exit 1

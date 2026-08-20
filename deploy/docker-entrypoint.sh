#!/bin/sh
set -eu

# Start as root so bind-mounted ./data can be given to the runtime user,
# then drop privileges. PUID/PGID match common NAS / home-server layouts.
if [ "$(id -u)" = "0" ]; then
  PUID="${PUID:-1000}"
  PGID="${PGID:-1000}"
  groupmod -o -g "$PGID" bookclub
  usermod -o -u "$PUID" bookclub
  mkdir -p /data
  chown -R bookclub:bookclub /data
  exec gosu bookclub "$0" "$@"
fi

exec "$@"

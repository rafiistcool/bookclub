#!/usr/bin/env bash
# Checks the app's sign-up, login and reset limits from outside, through the reverse proxy
# (#55, #51; thresholds in docs/phase1/conventions.md §10.1).
#
# Usage: deploy/hosted/verify-limits.sh <base-url>
#   e.g. deploy/hosted/verify-limits.sh https://bookclub.example.de
#   VERIFY_LIMITS_CURL_FLAGS="--insecure" for a local test with `tls internal`.
#
# It creates nothing: the sign-ups carry an invite code that does not exist, the logins use
# made-up names, and the reset asks for an address no account holds (example.invalid). It does
# use up this client's budgets: sign-up and reset for an hour, login for 10 minutes. Expect 429s
# from this address afterwards, and wait an hour before running it again.
#
# Every request also sends a different forged X-Forwarded-For. If the proxy passed it on and the
# app counted it, every request would land in a fresh bucket and no 429 would come.
#
# Checks, in this order:
#   1. sign-up: 5 attempts answer 400, the 6th 429
#   2. login with an unknown name: 10 answer 401, the 11th 429 (the per-name limit)
#   3. login per address: attempts 12 to 30, each with a fresh name, answer 401, the 31st 429
#   4. reset: 5 requests answer 204, the 6th 429
# Each 429 must carry Retry-After and the app's detail. Search is limited per account inside
# the app, not per address, so it is not checked here.
#
# Needs bash (3.2 or later, so macOS's /bin/bash will do) and curl.
#
# Exit status: 0 when every check passes, 1 when one fails, 2 on a usage error.

set -euo pipefail

TOO_MANY='Too many attempts. Wait a few minutes.'

usage() {
    echo "usage: $0 <base-url>   (e.g. https://bookclub.example.de)" >&2
    exit 2
}

[[ $# -eq 1 ]] || usage
base=${1%/}
[[ $base =~ ^https?://[^/]+$ ]] || usage
command -v curl >/dev/null || { echo "curl is required" >&2; exit 2; }

read -r -a extra_flags <<<"${VERIFY_LIMITS_CURL_FLAGS:-}"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# A tag for this run's made-up names: lowercase hex, so usernames stay within [a-z0-9_].
tag=$(od -An -N4 -tx1 /dev/urandom | tr -d ' \n')
forged=0
failures=0

# request METHOD PATH [JSON]: sets $status, $retry_after and $body.
request() {
    local method=$1 path=$2 json=${3:-}
    forged=$((forged + 1))
    local args=(--silent --show-error --max-time 30 --request "$method"
        --header "X-Forwarded-For: 192.0.2.$((forged % 250 + 1))"
        --output "$work/body" --dump-header "$work/headers" --write-out '%{http_code}')
    if [[ -n $json ]]; then
        args+=(--header 'Content-Type: application/json' --data "$json")
    fi
    # ${a[@]+"${a[@]}"}: bash before 4.4 (macOS /bin/bash is 3.2) calls an empty array unbound
    # under `set -u`, which would turn every request into a failure.
    status=$(curl "${args[@]}" ${extra_flags[@]+"${extra_flags[@]}"} "$base$path") || status=000
    retry_after=$(tr -d '\r' <"$work/headers" 2>/dev/null |
        awk 'tolower($1) == "retry-after:" { print $2 }' | tail -n 1)
    body=$(cat "$work/body" 2>/dev/null || true)
}

pass() { printf 'ok    %s\n' "$1"; }
fail() {
    printf 'FAIL  %s\n' "$1"
    failures=$((failures + 1))
}

# expect_limit NAME: the last request must be the app's 429.
expect_limit() {
    local name=$1
    if [[ $status != 429 ]]; then
        fail "$name: expected 429, got $status. No limit holds: the app may count a forged X-Forwarded-For (check TRUSTED_PROXIES), or the proxy passes the client's header on."
    elif [[ ! $retry_after =~ ^[1-9][0-9]*$ ]]; then
        fail "$name: 429 without a valid Retry-After (${retry_after:-none})"
    elif [[ $body != *"$TOO_MANY"* ]]; then
        fail "$name: 429 that is not the app's (body: ${body:0:120})"
    else
        pass "$name: 429 with Retry-After $retry_after"
    fi
}

# expect_status NAME WANT N: the last request (attempt N) must have answered WANT.
expect_status() {
    local name=$1 want=$2 n=$3
    if [[ $status == "$want" ]]; then
        return 0
    fi
    local hint=''
    if [[ $status == 429 ]]; then
        hint=' This address is already limited: an earlier run within the hour, another client behind the same address, or every client shares one bucket (TRUSTED_PROXIES).'
    fi
    fail "$name: attempt $n answered $status, expected $want.$hint"
    return 1
}

echo "Checking the limits of $base (run tag $tag)"

request GET /api/health
if [[ $status == 200 && $body == *'"ok"'* ]]; then
    pass "health: 200"
else
    fail "health: GET /api/health answered $status (body: ${body:0:120}); is this the app?"
    exit 1
fi

# 1. Sign-up: 5 per address an hour. An unknown invite code answers 400 and rolls back.
check='sign-up per address'
ok=1
for n in 1 2 3 4 5; do
    request POST /api/auth/register "{\"username\":\"vl_${tag}_s$n\",\"email\":\"vl-$tag-$n@example.invalid\",\"password\":\"verify-limits-$tag\",\"invite_code\":\"NOPE$tag\"}"
    expect_status "$check" 400 "$n" || { ok=0; break; }
done
if ((ok)); then
    request POST /api/auth/register "{\"username\":\"vl_${tag}_s6\",\"email\":\"vl-$tag-6@example.invalid\",\"password\":\"verify-limits-$tag\",\"invite_code\":\"NOPE$tag\"}"
    expect_limit "$check: 5 x 400, then"
fi

# 2. Login with one unknown name: 10 failures per name in 10 minutes.
check='login per unknown name'
ok=1
for n in $(seq 1 10); do
    request POST /api/auth/login "{\"username\":\"vl_${tag}_x\",\"password\":\"wrong-$tag\"}"
    expect_status "$check" 401 "$n" || { ok=0; break; }
done
if ((ok)); then
    request POST /api/auth/login "{\"username\":\"vl_${tag}_x\",\"password\":\"wrong-$tag\"}"
    expect_limit "$check: 10 x 401, then"
fi

# 3. Login per address: 30 attempts in 10 minutes, whatever the name. Check 2 made 11.
check='login per address'
if ((ok)); then
    for n in $(seq 12 30); do
        request POST /api/auth/login "{\"username\":\"vl_${tag}_l$n\",\"password\":\"wrong-$tag\"}"
        expect_status "$check" 401 "$n" || { ok=0; break; }
    done
    if ((ok)); then
        request POST /api/auth/login "{\"username\":\"vl_${tag}_l31\",\"password\":\"wrong-$tag\"}"
        expect_limit "$check: 30 attempts, then"
    fi
else
    fail "$check: skipped, check 2 failed"
fi

# 4. Reset: 5 requests per address an hour; 204 whether or not an account holds the address.
check='reset per address'
ok=1
for n in 1 2 3 4 5; do
    request POST /api/auth/password-reset "{\"email\":\"vl-$tag@example.invalid\"}"
    expect_status "$check" 204 "$n" || { ok=0; break; }
done
if ((ok)); then
    request POST /api/auth/password-reset "{\"email\":\"vl-$tag@example.invalid\"}"
    expect_limit "$check: 5 x 204, then"
fi

if ((failures)); then
    echo "$failures check(s) failed"
    exit 1
fi
echo "All limits hold. This address stays limited for sign-up and reset for an hour."

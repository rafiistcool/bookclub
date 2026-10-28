#!/usr/bin/env bash
# The floating tags of a release (README.md, "Releases"). release.yml's publish job runs it
# after it has pushed the exact version:
#   X.Y     when this is the highest release of its minor line;
#   stable  when this is the highest release of all.
# "Highest" counts the version tags vX.Y.Z on the remote at this moment, pre-releases left out.
# So a backport (v0.10.5 after v0.11.0) moves 0.10 but not stable, and of two release runs that
# overlap (v0.11.0, then a hotfix v0.11.1 minutes later), only v0.11.1 moves the tags, whichever
# run ends last. A pre-release moves nothing.
#
#   release-tags.sh IMAGE TAG DIGEST   point the floating tags at IMAGE@DIGEST
#   release-tags.sh --plan TAG         only print the floating tags TAG gets
#
# It reads the tags with `git ls-remote` from REMOTE (default origin), and retags in the
# registry with `docker buildx imagetools create`, which needs a login. The copy is a plain
# image manifest like the one pushed (--prefer-index=false), not an index around it.
set -euo pipefail

remote=${REMOTE:-origin}

usage() {
    echo "usage: $0 IMAGE TAG DIGEST | $0 --plan TAG" >&2
    exit 2
}

# Every release tag on the remote (vX.Y.Z, no pre-release), lowest first.
releases() {
    git ls-remote --tags --refs "$remote" 'v*' |
        sed -n 's#^[0-9a-f]*[[:space:]]*refs/tags/##p' |
        { grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' || true; } |
        sort -V
}

# The floating tags $1 gets, one per line.
plan() {
    local tag=$1 all line
    if [[ ! $tag =~ ^v([0-9]+)\.([0-9]+)\.[0-9]+$ ]]; then
        return 0
    fi
    local minor="${BASH_REMATCH[1]}.${BASH_REMATCH[2]}"
    all=$(releases)
    grep -qxF "$tag" <<<"$all" || {
        echo "$tag is not a release tag on $remote" >&2
        return 1
    }
    line=$(grep -E "^v${minor//./\\.}\.[0-9]+$" <<<"$all" | tail -n 1)
    if [[ $line == "$tag" ]]; then
        echo "$minor"
    fi
    if [[ $(tail -n 1 <<<"$all") == "$tag" ]]; then
        echo stable
    fi
}

case "${1-}" in
    --plan)
        (($# == 2)) || usage
        plan "$2"
        ;;
    -* | "") usage ;;
    *)
        (($# == 3)) || usage
        image=$1 tag=$2 digest=$3
        [[ $digest =~ ^sha256:[0-9a-f]{64}$ ]] || {
            echo "$digest is not a sha256 digest" >&2
            exit 2
        }
        floating=$(plan "$tag")
        if [[ -z $floating ]]; then
            echo "$tag moves no floating tag: a pre-release, or a newer release exists"
            exit 0
        fi
        while IFS= read -r name; do
            echo "pointing $image:$name at $tag ($digest)"
            docker buildx imagetools create --prefer-index=false --tag "$image:$name" \
                "$image@$digest"
        done <<<"$floating"
        ;;
esac

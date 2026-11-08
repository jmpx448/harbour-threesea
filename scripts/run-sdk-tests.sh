#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Every scripts/test-*.sh in the Sailfish SDK image (scripts/sdk-build.sh builds it): a new test
# script is run without being added anywhere. scripts/test-callengine.sh needs the emulator to run,
# so here it is only built; scripts/test-ffi-asan.sh runs on the host (nightly), from
# scripts/check.sh --full and its own CI job. Each one's output is shown if it fails.
# Exit 1 if any failed, 2 if the image is missing.
# Usage: scripts/run-sdk-tests.sh [test-name ...]   (e.g. models pages; default: all)
# SDK_TEST_JOBS: how many run at the same time (default 1: one after the other).
set -u
cd "$(dirname "$0")/.."

# One test, for the run below. Its result goes out in one piece, as others may be writing.
if [ "${1:-}" = --one ]; then
    name=$2
    case $name in
        callengine) flag=--build-only ;;
        *) flag= ;;
    esac
    log=$(mktemp)
    trap 'rm -f "$log"' EXIT
    # shellcheck disable=SC2086 # flag is empty or one word
    if "scripts/test-$name.sh" $flag >"$log" 2>&1; then
        echo "ok   $name${flag:+ (built only)}"
        exit 0
    fi
    printf 'FAIL %s\n%s\n' "$name" "$(tail -n 60 "$log")"
    exit 1
fi

IMAGE=harbour-threesea-sdk:${SFOS_VERSION:-5.1.0.11}
if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
    echo "no $IMAGE: scripts/sdk-build.sh builds it" >&2
    exit 2
fi
if [ $# -gt 0 ]; then
    names=$*
else
    names=$(ls scripts/test-*.sh | grep -v '^scripts/test-ffi-asan.sh$' | sed 's|scripts/test-||; s|\.sh$||')
fi
# shellcheck disable=SC2086 # one name per word
printf '%s\n' $names | xargs -n 1 -P "${SDK_TEST_JOBS:-1}" scripts/run-sdk-tests.sh --one || exit 1

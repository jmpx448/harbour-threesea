#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# The version to release, checked: the same in rpm/harbour-threesea.spec, Cargo.toml (workspace)
# and harbour-threesea.pro, with a section `## [<version>]` in CHANGELOG.md. Prints the version.
#   --notes FILE   also write that CHANGELOG section (the release notes) to FILE
#   --new-tag      also fail if the tag v<version> exists already (a release is made once)
set -eu
cd "$(dirname "$0")/.."
notes=
new_tag=false
while [ $# -gt 0 ]; do
    case $1 in
        --notes) notes=$2; shift 2 ;;
        --new-tag) new_tag=true; shift ;;
        *) echo "usage: $0 [--notes FILE] [--new-tag]" >&2; exit 2 ;;
    esac
done
fail() { echo "release version: $1" >&2; exit 1; }

spec=$(sed -n 's/^Version:[[:space:]]*//p' rpm/harbour-threesea.spec)
cargo=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
pro=$(sed -n 's/^VERSION = //p' harbour-threesea.pro)
[ -n "$spec" ] || fail "no Version in rpm/harbour-threesea.spec"
[ "$spec" = "$cargo" ] || fail "Cargo.toml has $cargo, the spec $spec"
[ "$spec" = "$pro" ] || fail "harbour-threesea.pro has $pro, the spec $spec"
echo "$spec" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || fail "$spec is not MAJOR.MINOR.PATCH"
grep -q "^## \[$spec\]" CHANGELOG.md || fail "CHANGELOG.md has no section ## [$spec]"
if $new_tag && git rev-parse -q --verify "refs/tags/v$spec" >/dev/null; then
    fail "tag v$spec exists already: raise the version for a new release"
fi
if [ -n "$notes" ]; then
    awk -v v="## [$spec]" 'index($0, v) == 1 { on = 1; next } /^## \[/ { on = 0 } on' CHANGELOG.md > "$notes"
    [ -s "$notes" ] || fail "the CHANGELOG section for $spec is empty"
fi
echo "$spec"

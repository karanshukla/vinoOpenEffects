#!/usr/bin/env bash
# Tag the next release. Versions are consecutive (v1, v2, ...), not semver: each
# release is main at that point (#46). The inherited v0.1.x tags count by their
# first number, so the first release is v1. CHANGELOG.md must already have the
# section for the new tag.
#
#   scripts/release.sh --dry-run   # show the next tag and the PRs merged since the last one
#   scripts/release.sh             # tag origin/main's head and push the tag
set -euo pipefail
cd "$(dirname "$0")/.."

die() { echo "release: $*" >&2; exit 1; }

dry_run=0
case "${1:-}" in
    --dry-run) dry_run=1 ;;
    "") ;;
    *) die "usage: scripts/release.sh [--dry-run]" ;;
esac

git fetch -q --tags origin main
head="$(git rev-parse origin/main)"
if [ "$dry_run" = 0 ]; then
    [ "$(git rev-parse HEAD)" = "$head" ] || die "check out origin/main's head first (git switch main && git pull)"
    [ -z "$(git status --porcelain)" ] || die "working tree has changes"
fi

last="$(git tag -l 'v*' | sed -nE 's/^v([0-9]+).*/\1/p' | sort -n | tail -1)"
next="v$(( ${last:-0} + 1 ))"
prev="$(git tag -l "v${last:-0}*" | sort -V | tail -1)"

if tags="$(git tag --points-at "$head" -l 'v*')" && [ -n "$tags" ]; then
    die "origin/main is already tagged: $(echo "$tags" | tr '\n' ' ')"
fi

echo "Next release: $next ($(git rev-parse --short "$head"))"
if [ -n "$prev" ]; then
    echo "Merged since $prev:"
    git log --merges --format='%s%x09%b' "$prev..$head" \
        | sed -nE 's/^Merge pull request (#[0-9]+) from [^\t]*\t(.*)/  \1 \2/p'
fi

if ! git show "$head:CHANGELOG.md" 2>/dev/null | grep -q "^## \[$next\]"; then
    die "CHANGELOG.md on main has no '## [$next]' section; add it with: git cliff --unreleased --tag $next --prepend CHANGELOG.md"
fi

[ "$dry_run" = 1 ] && exit 0
read -r -p "Tag and push $next? [y/N] " answer
[ "$answer" = y ] || die "not tagged"
git tag -a "$next" -m "$next" "$head"
git push origin "$next"
echo "Pushed $next."

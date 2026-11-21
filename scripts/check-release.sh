#!/usr/bin/env bash
# Refuses a release that would go backwards, or that doesn't match what it says it is.
#
#     scripts/check-release.sh v0.14.0 [commit]      (the commit defaults to HEAD)
#
# It runs first in the release workflow (before any build, so a bad release costs nothing), and
# should be run by hand before starting one. A release has to:
#   1. use a new tag, in the form vMAJOR.MINOR.PATCH, newer than every release so far;
#   2. have the same version in Cargo.toml as the tag, so the app says what it is;
#   3. contain every earlier release (so it can never lose a feature the last one had);
#   4. contain everything on main (so main's work isn't left out).
# Other branches with work that isn't in the release only get a warning; branches listed in
# scripts/release-ignore-branches.txt (finished or superseded) are left out of that.
#
# It needs the full history (the workflow checks out with fetch-depth: 0).
set -euo pipefail
cd "$(dirname "$0")/.."

tag="${1:-}"
commit="${2:-HEAD}"
failed=0
bad() { echo "ERROR: $*" >&2; failed=1; }
warn() { echo "warning: $*" >&2; [ -z "${GITHUB_ACTIONS:-}" ] || echo "::warning::$*"; }

if ! [[ "$tag" =~ ^v([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
  echo "usage: $0 vMAJOR.MINOR.PATCH [commit]   (for example v0.14.0)" >&2
  exit 2
fi
version="${tag#v}"
commit=$(git rev-parse --verify --quiet "$commit^{commit}") || { echo "no such commit" >&2; exit 2; }

if [ "$(git rev-parse --is-shallow-repository)" = true ]; then
  echo "ERROR: this clone is shallow, so history can't be checked. Run: git fetch --unshallow --tags" >&2
  exit 2
fi

# 1. A new tag, newer than all the others.
if git rev-parse --quiet --verify "refs/tags/$tag" >/dev/null; then
  bad "the tag $tag already exists, so it can't be released again. Pick the next version."
fi
releases=$(git tag --list 'v[0-9]*.[0-9]*.[0-9]*' | sort -V)
latest=$(printf '%s\n' "$releases" | tail -n 1)
if [ -n "$latest" ] && [ "$(printf '%s\n%s\n' "$latest" "$tag" | sort -V | tail -n 1)" != "$tag" ]; then
  bad "$tag is not newer than the latest release, $latest."
fi

# 2. The version in Cargo.toml says the same.
cargo_version=$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\(.*\)".*/\1/p;}' Cargo.toml | head -n 1)
if [ "$cargo_version" != "$version" ]; then
  bad "Cargo.toml says version \"$cargo_version\" but the tag is $tag. Set version = \"$version\" in the workspace Cargo.toml (and update Cargo.lock)."
fi

# 3. Every earlier release is inside this one.
for old in $releases; do
  if ! git merge-base --is-ancestor "$old" "$commit"; then
    bad "release $old is not part of this commit, so $tag would lose what $old had. Merge the branch it was built from first ($(git log -1 --format=%h "$old"))."
  fi
done

# 4. Everything on main is inside this one.
if git rev-parse --quiet --verify refs/remotes/origin/main >/dev/null; then
  if ! git merge-base --is-ancestor origin/main "$commit"; then
    missing=$(git rev-list --count "$commit..origin/main")
    bad "main has $missing commit(s) this release doesn't. Merge main into the branch first."
  fi
else
  warn "no origin/main to compare with."
fi

# Other branches with work that isn't in this release (a warning only).
ignored=$(grep -v '^[[:space:]]*\(#\|$\)' scripts/release-ignore-branches.txt 2>/dev/null || true)
for ref in $(git for-each-ref --format='%(refname:short)' refs/remotes/origin); do
  name="${ref#origin/}"
  case "$name" in HEAD|main) continue ;; esac
  printf '%s\n' "$ignored" | grep -qx "$name" && continue
  if ! git merge-base --is-ancestor "$ref" "$commit"; then
    n=$(git rev-list --count "$commit..$ref")
    warn "branch $name has $n commit(s) that aren't in this release ($(git log -1 --format='%h %s' "$ref" | cut -c1-70))."
  fi
done

if [ "$failed" -ne 0 ]; then
  echo "Release $tag refused." >&2
  exit 1
fi
echo "Release $tag is OK: ${commit:0:8} contains every release up to ${latest:-none} and main."

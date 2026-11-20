#!/bin/sh
# Tests tools/release-channel.sh against a disposable git repo, built fresh
# each run: a `main` branch, a tag on it (stable), a second branch off `main`
# with its own tag never merged back (latest), and a tag that does not exist
# (refused).
set -eu

cd "$(dirname "$0")/.."
script="$(pwd)/tools/release-channel.sh"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cd "$work"
git init -q -b main
git config user.name test
git config user.email test@example.com

git commit -q --allow-empty -m "main 1"
git tag v0.18.0
git commit -q --allow-empty -m "main 2"
git tag v0.18.1

git checkout -q -b feat/next
git commit -q --allow-empty -m "unreleased feature"
git tag v0.19.0

fail=0

check() {
	tag="$1"
	want="$2"
	got=$("$script" "$tag")
	if [ "$got" != "$want" ]; then
		echo "FAIL: $tag -> '$got', wanted '$want'" >&2
		fail=1
	else
		echo "ok: $tag -> $got"
	fi
}

check v0.18.0 stable
check v0.18.1 stable
check v0.19.0 latest

if "$script" v9.9.9 >/dev/null 2>&1; then
	echo "FAIL: a nonexistent tag was accepted" >&2
	fail=1
else
	echo "ok: v9.9.9 (nonexistent) refused"
fi

if "$script" >/dev/null 2>&1; then
	echo "FAIL: no argument was accepted" >&2
	fail=1
else
	echo "ok: no argument refused"
fi

exit "$fail"

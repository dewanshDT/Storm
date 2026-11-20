#!/bin/sh
# Tests tools/release-channel.sh against a disposable git repo, built fresh
# each run: a `main` branch with an ordinary tag (stable), a feature branch
# with its own tag never merged back (latest), then a decision-72-style
# promotion (merge the feature branch into main) — proving the promoted
# tag's channel does NOT flip, which is exactly what first-parent ancestry
# is for (plain `merge-base --is-ancestor` would flip it, since the feature
# commit becomes reachable from main via the merge's second parent).
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

git checkout -q -b feat/next
git commit -q --allow-empty -m "unreleased feature"
git tag v0.19.0

git checkout -q main
git merge -q --no-ff -m "promote feat/next into main" feat/next
git tag v0.20.0

git commit -q --allow-empty -m "main 2, after the promotion"
git tag v0.20.1

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
# The regression this exists for: v0.19.0 is reachable from main's tip once
# feat/next is merged, but only via the merge commit's second parent — it
# must stay latest, not flip to stable on a rerun after the promotion.
check v0.19.0 latest
# The merge (promotion) commit itself is on main's first-parent chain.
check v0.20.0 stable
# An ordinary commit made directly on main after the promotion.
check v0.20.1 stable

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

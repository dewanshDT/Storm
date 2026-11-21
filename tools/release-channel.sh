#!/bin/sh
# Decision 163c: the channel of a tag is decided ONCE, by ancestry, and only
# here — every workflow and check calls this instead of re-deriving the
# answer. `stable` if the tag's commit is on origin/main's FIRST-PARENT
# chain, else `latest`.
#
# First-parent, not plain reachability: decision 72's promotion merges
# `staging` into `main`, and after that merge, plain `merge-base
# --is-ancestor` would call every earlier `latest` tag `stable` too — they
# are reachable from main's tip via the merge commit's *second* parent. A
# stable tag only ever sits on main's own line (a merge commit's first
# parent, or an ordinary commit made directly on main); a promoted commit's
# tag stays whatever channel it already was. Re-deriving this with plain
# ancestry after a promotion would call a pre-promotion latest tag stable on
# a rerun, and the apt workflow_dispatch fallback could mark it GitHub Latest.
set -eu

tag="${1:-}"
if [ -z "$tag" ]; then
	echo "usage: release-channel.sh <tag>" >&2
	exit 1
fi

if ! git rev-parse -q --verify "${tag}^{commit}" >/dev/null 2>&1; then
	echo "release-channel: '$tag' is not a commit this checkout knows" >&2
	exit 1
fi

# origin/main is the real ref everywhere this runs for real (CI fetches it
# with fetch-depth 0 first). A local main covers a from-scratch test repo
# that has no remote at all.
if git rev-parse -q --verify origin/main >/dev/null 2>&1; then
	main_ref=origin/main
elif git rev-parse -q --verify main >/dev/null 2>&1; then
	main_ref=main
else
	echo "release-channel: no origin/main or main ref to compare '$tag' against" >&2
	exit 1
fi

commit=$(git rev-parse "${tag}^{commit}")
if git rev-list --first-parent "$main_ref" | grep -qx "$commit"; then
	echo stable
else
	echo latest
fi

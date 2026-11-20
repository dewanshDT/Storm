#!/bin/sh
# Decision 163c: the channel of a tag is decided ONCE, by ancestry, and only
# here — every workflow and check calls this instead of re-deriving the
# answer. `stable` if the tag's commit is reachable from origin/main (it was
# promoted there, decision 72), else `latest`.
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

if git merge-base --is-ancestor "$tag" "$main_ref"; then
	echo stable
else
	echo latest
fi

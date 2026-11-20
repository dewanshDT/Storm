#!/bin/sh
# Formula/storm-runtime.rb (stable) and Formula/storm-runtime-latest.rb
# (latest, decision 163) each build the Runtime Host from a release tag.
# This repository is the tap, so `brew install` reads the formula on `main`.
# A tag is bumped with apps/www/src/data/release.ts in the release-prep PR
# (decision 72); a formula left on an older tag silently installs the
# previous release on every Mac.
#
# release.ts's `latest` field (decision 163) doesn't exist yet -- another
# agent adds it once the first latest release is cut -- so until then the
# latest formula's check is skipped, not failed: a missing field is not a
# stale tag. It is expected as a sibling of `tag`, the same flat-string
# shape (the release-prep PR that adds it should match this, or update the
# `latest_tag` line below to whatever shape it actually lands in):
#   export const release = { tag: "v0.18.0", latest: "v0.19.0", ... }
set -eu

cd "$(dirname "$0")/.."

# check_tag LABEL SITE_TAG FORMULA_FILE
check_tag() {
	label=$1 site_tag=$2 formula=$3
	formula_tag=$(sed -n 's/.*url "[^"]*", tag: "\(v[0-9][^"]*\)".*/\1/p' "$formula" | head -1)
	if [ -z "$formula_tag" ]; then
		echo "formula-check: could not read a tag from $formula" >&2
		return 1
	fi
	if [ "$formula_tag" != "$site_tag" ]; then
		echo "formula-check: $formula builds $formula_tag, release.ts's $label tag is $site_tag" >&2
		echo "  Bump the formula's \`tag:\` in the same release-prep PR as release.ts." >&2
		return 1
	fi
	echo "formula-check: ok, $label ($formula_tag)"
}

site_tag=$(sed -n 's/.*tag: "\(v[0-9][^"]*\)".*/\1/p' apps/www/src/data/release.ts | head -1)
if [ -z "$site_tag" ]; then
	echo "formula-check: could not read release.ts's stable tag" >&2
	exit 1
fi

rc=0
check_tag stable "$site_tag" Formula/storm-runtime.rb || rc=1

# Anchored on "latest:" at the start of the (trimmed) line, so it can never
# match the unrelated `latestUrl` getter just above `tag` in release.ts.
latest_tag=$(sed -n 's/^[[:space:]]*latest:[[:space:]]*"\(v[0-9][^"]*\)".*/\1/p' apps/www/src/data/release.ts | head -1)
if [ -z "$latest_tag" ]; then
	echo "formula-check: release.ts has no \`latest\` field yet; skipping Formula/storm-runtime-latest.rb's check"
else
	check_tag latest "$latest_tag" Formula/storm-runtime-latest.rb || rc=1
fi

exit "$rc"

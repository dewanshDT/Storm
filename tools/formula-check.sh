#!/bin/sh
# Formula/storm-runtime.rb builds the Runtime Host from a release tag
# (decision 83, AM41). This repository is the tap, so `brew install` reads the
# formula on `main`. Its tag is bumped with apps/www/src/data/release.ts in the
# release-prep PR (decision 72); a formula left on an older tag silently
# installs the previous release on every Mac.
set -eu

cd "$(dirname "$0")/.."

site_tag=$(sed -n 's/.*tag: "\(v[0-9][^"]*\)".*/\1/p' apps/www/src/data/release.ts | head -1)
formula_tag=$(sed -n 's/.*url "[^"]*", tag: "\(v[0-9][^"]*\)".*/\1/p' Formula/storm-runtime.rb | head -1)

if [ -z "$site_tag" ] || [ -z "$formula_tag" ]; then
	echo "formula-check: could not read a tag (release.ts: '${site_tag}', formula: '${formula_tag}')" >&2
	exit 1
fi

if [ "$formula_tag" != "$site_tag" ]; then
	echo "formula-check: Formula/storm-runtime.rb builds $formula_tag, release.ts names $site_tag" >&2
	echo "  Bump the formula's \`tag:\` in the same release-prep PR as release.ts." >&2
	exit 1
fi

echo "formula-check: ok ($formula_tag)"

#!/bin/sh
# Two checks the Astro build cannot make, because the site can be perfectly
# valid HTML and still be lying.
#
# 1. The download links point at the newest tag.
# 2. No page still describes the shared token, which stopped existing at the
#    A10 cutover (M19).
#
# The second one is why this file exists. `apps/www` kept telling people to
# copy a token out of `storm.env` for thirteen days after the token was deleted,
# CI was green the whole time, and nothing in a build check could ever have
# caught it — a wrong sentence compiles.
set -eu

cd "$(dirname "$0")/.."
status=0

# --- 1. release.ts vs the newest tag ----------------------------------------
site_tag=$(sed -n 's/.*tag: "\(v[0-9][^"]*\)".*/\1/p' src/data/release.ts | head -1)
if [ -z "$site_tag" ]; then
	echo "check-claims: could not read \`tag\` from src/data/release.ts" >&2
	exit 1
fi

# Needs tags in the checkout — CI uses fetch-depth: 0 for exactly this.
newest_tag=$(git tag --list 'v*' --sort=-version:refname | head -1)
if [ -z "$newest_tag" ]; then
	echo "check-claims: no v* tags in this checkout; skipping the version check"
elif [ "$site_tag" != "$newest_tag" ]; then
	echo "check-claims: src/data/release.ts says $site_tag, newest tag is $newest_tag" >&2
	echo "  Every download link on /clients points at $site_tag. Bump \`tag\`." >&2
	status=1
fi

# --- 2. claims that outlived the thing they describe -------------------------
# Phrasings, not the bare word: the pages say "There is no token in that file"
# on purpose, and a grep for `token` would forbid saying so.
stale='generated token|token from|shared token|shared bearer|token in `?storm\.env|no accounts product'
if hits=$(grep -rniE "$stale" src ../../docs/www 2>/dev/null); then
	echo "check-claims: a page still describes the shared token or the pre-M19 identity model" >&2
	echo "$hits" | sed 's/^/  /' >&2
	echo "  There is no shared token. See deploy/storm.env.example and /install#first-login." >&2
	status=1
fi

[ "$status" -eq 0 ] && echo "check-claims: ok ($site_tag, no stale credential copy)"
exit "$status"

#!/bin/sh
# Two checks the Astro build cannot make, because the site can be perfectly
# valid HTML and still be lying.
#
# 1. The download links point at the newest tag.
# 2. No page still describes the shared token, which stopped existing at the
#    A10 cutover (M19).
# 3. No page describes more than one account (decision 82).
# 4. The release release.ts names has a changelog entry.
# 5. src/data/facts.json (the MCP tools) matches the server's source.
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
	# Older is the failure this exists for: links to a stale release. Newer is
	# the release being cut. The site has to name the tag *before* the tag
	# exists, or the tagged commit ships a site pointing at the last release;
	# an equality check made every release PR red (decision 72).
	older=$(printf '%s\n%s\n' "$site_tag" "$newest_tag" | sort -V | head -1)
	if [ "$older" = "$site_tag" ]; then
		echo "check-claims: src/data/release.ts says $site_tag, newest tag is $newest_tag" >&2
		echo "  Every download link on /clients points at $site_tag. Bump \`tag\`." >&2
		status=1
	else
		echo "check-claims: release.ts names $site_tag, not tagged yet (newest is $newest_tag); a release in progress"
	fi
fi

# --- 2. claims that outlived the thing they describe -------------------------
# Phrasings, not the bare word: the pages say "There is no token in that file"
# on purpose, and a grep for `token` would forbid saying so.
stale='generated token|token from|shared token|shared bearer|token in `?storm\.env|no accounts product'
if hits=$(grep -rniE "$stale" src ../../docs/site 2>/dev/null); then
	echo "check-claims: a page still describes the shared token or the pre-M19 identity model" >&2
	echo "$hits" | sed 's/^/  /' >&2
	echo "  There is no shared token. See deploy/storm.env.example and /install#first-login." >&2
	status=1
fi

# --- 3. accounts that stopped existing --------------------------------------
# Storm became single-user in decision 82 and `storm-server user` went with it.
# The site went on telling people to run `user add` for three releases. The
# root README is in scope: it is the first page most people read.
accounts='storm-server user |user add|user list|is (always )?an owner|add more (accounts|from the app)|more accounts|several devices is the default'
if hits=$(grep -rniE "$accounts" src ../../docs/site ../../README.md 2>/dev/null); then
	echo "check-claims: a page still describes more than one account" >&2
	echo "$hits" | sed 's/^/  /' >&2
	echo "  Storm is single-user (decision 82): one account, \`storm-server passwd\` to reset it." >&2
	status=1
fi

# --- 4. a changelog entry for the release the site names -------------------
# /changelog is built from docs/site/releases/, one file per release, written
# during release prep beside the release.ts bump. A tag without one ships a
# changelog that stops at the previous release.
if [ ! -f "../../docs/site/releases/$site_tag.md" ]; then
	echo "check-claims: no docs/site/releases/$site_tag.md; write the changelog entry with the bump" >&2
	status=1
fi

# --- 5. the facts the site quotes ------------------------------------------
# The MCP tool list on / and in the docs comes from src/data/facts.json, which
# tools/www-facts generates from apps/server/src/mcp.rs. A typed list went
# stale at twelve while the server grew to seventeen. The CLI sections of the
# same file are checked by the server and runtime CI jobs, which build them.
if ! python3 ../../tools/www-facts/generate.py --check; then
	status=1
fi

[ "$status" -eq 0 ] && echo "check-claims: ok ($site_tag, no stale credential or account copy, facts current)"
exit "$status"

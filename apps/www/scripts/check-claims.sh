#!/bin/sh
# Two checks the Astro build cannot make, because the site can be perfectly
# valid HTML and still be lying.
#
# 1. The download links point at the newest tag of each channel (decision 163).
# 2. No page still describes the shared token, which stopped existing at the
#    A10 cutover (M19).
# 3. No page describes more than one account (decision 82).
# 4. The release(s) release.ts names each have a changelog entry.
# 5. src/data/facts.json (the MCP tools) matches the server's source.
#
# The second one is why this file exists. `apps/www` kept telling people to
# copy a token out of `storm.env` for thirteen days after the token was deleted,
# CI was green the whole time, and nothing in a build check could ever have
# caught it — a wrong sentence compiles.
set -eu

cd "$(dirname "$0")/.."
status=0

# --- 1. release.ts vs the newest tag, per channel (decision 163) -----------
# `tag` must never fall behind the newest STABLE tag; `latest`, when set,
# must never fall behind the newest tag overall. A tag's channel is decided
# once, by tools/release-channel.sh (163c): first-parent ancestry from
# origin/main, not plain reachability — a decision-72 promotion merge would
# otherwise call every earlier latest tag stable too.
site_tag=$(sed -n 's/.*tag: "\(v[0-9][^"]*\)".*/\1/p' src/data/release.ts | head -1)
if [ -z "$site_tag" ]; then
	echo "check-claims: could not read \`tag\` from src/data/release.ts" >&2
	exit 1
fi
# Anchored at the start of the trimmed line, so it never matches the
# unrelated `latestUrl` getter next to it.
latest_tag=$(sed -n 's/^[[:space:]]*latest:[[:space:]]*"\(v[0-9][^"]*\)".*/\1/p' src/data/release.ts | head -1)

# Needs tags *and* origin/main in the checkout — CI uses fetch-depth: 0 for
# exactly this.
all_tags=$(git tag --list 'v*' --sort=-version:refname)
if [ -z "$all_tags" ]; then
	echo "check-claims: no v* tags in this checkout; skipping the version checks"
elif ! git rev-parse -q --verify origin/main >/dev/null 2>&1; then
	echo "check-claims: no origin/main in this checkout; skipping the channel checks" >&2
else
	repo_root=$(git rev-parse --show-toplevel)
	newest_tag=$(printf '%s\n' "$all_tags" | head -1)
	newest_stable_tag=""
	for t in $all_tags; do
		if [ "$(sh "$repo_root/tools/release-channel.sh" "$t" 2>/dev/null)" = "stable" ]; then
			newest_stable_tag=$t
			break
		fi
	done

	# check_claim LABEL CLAIMED NEWEST
	check_claim() {
		label=$1 claimed=$2 newest=$3
		[ -z "$newest" ] && return 0
		if [ "$claimed" = "$newest" ]; then
			echo "check-claims: ok, $label ($claimed)"
			return 0
		fi
		# Older is the failure this exists for: a stale download link. Newer is
		# the release being cut — the site has to name a tag before the tag
		# exists, or the tagged commit ships a site pointing at the last release
		# (decision 72); an equality check made every release PR red.
		older=$(printf '%s\n%s\n' "$claimed" "$newest" | sort -V | head -1)
		if [ "$older" = "$claimed" ]; then
			echo "check-claims: release.ts's \`$label\` is $claimed, newest is $newest" >&2
			echo "  Every download link for it points at $claimed. Bump \`$label\`." >&2
			return 1
		fi
		echo "check-claims: release.ts's \`$label\` names $claimed, not tagged yet (newest is $newest); a release in progress"
		return 0
	}

	if [ -n "$newest_stable_tag" ]; then
		check_claim tag "$site_tag" "$newest_stable_tag" || status=1
	else
		echo "check-claims: no stable tag reachable from origin/main yet; skipping \`tag\`'s check"
	fi
	if [ -n "$latest_tag" ]; then
		check_claim latest "$latest_tag" "$newest_tag" || status=1
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

# --- 4. a changelog entry for every release the site names -----------------
# /changelog is built from docs/site/releases/, one file per release, written
# during release prep beside the release.ts bump. A tag without one ships a
# changelog that stops at the previous release.
if [ ! -f "../../docs/site/releases/$site_tag.md" ]; then
	echo "check-claims: no docs/site/releases/$site_tag.md; write the changelog entry with the bump" >&2
	status=1
fi
if [ -n "$latest_tag" ] && [ ! -f "../../docs/site/releases/$latest_tag.md" ]; then
	echo "check-claims: no docs/site/releases/$latest_tag.md; write the changelog entry with the \`latest\` bump" >&2
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

[ "$status" -eq 0 ] && echo "check-claims: ok ($site_tag${latest_tag:+, latest $latest_tag}, no stale credential or account copy, facts current)"
exit "$status"

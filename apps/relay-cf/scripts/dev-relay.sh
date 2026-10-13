#!/usr/bin/env bash
# Runs the Worker relay under `wrangler dev` in the foreground, for the
# conformance suite and the live suites (decision 108c).
#
#   dev-relay.sh PORT MODE ALLOWLIST_FILE STATE_DIR [NAME=VALUE ...]
#
# MODE is allowlist or tofu. ALLOWLIST_FILE is storm-relay's allowlist format
# and is passed as STORM_RELAY_ALLOWLIST's content ("-" for none). Each
# NAME=VALUE becomes a Worker var. STATE_DIR is wiped first: every run starts
# with no nonces, bindings or rooms. SIGTERM stops wrangler and workerd with it.
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
port="$1" mode="$2" allowlist="$3" state="$4"
shift 4

rm -rf "$state"
mkdir -p "$state"
vars="$state/vars.env"
{
	echo "STORM_RELAY_BINDING=$mode"
	echo "STORM_RELAY_PUBLIC_BASE=ws://127.0.0.1:$port"
	if [ "$allowlist" != - ]; then
		printf 'STORM_RELAY_ALLOWLIST="'
		grep -v '^[[:space:]]*#' "$allowlist" | grep . || true
		echo '"'
	fi
	for kv in "$@"; do echo "$kv"; done
} >"$vars"

[ -x "$here/node_modules/.bin/wrangler" ] ||
	(cd "$here" && npm ci --no-audit --no-fund >/dev/null)

cd "$here"
export WRANGLER_SEND_METRICS=false
"$here/node_modules/.bin/wrangler" dev --port "$port" --ip 127.0.0.1 \
	--inspector-port 0 --show-interactive-dev-session=false \
	--env-file "$vars" --persist-to "$state/do" &
child=$!
trap 'kill -TERM "$child" 2>/dev/null; wait "$child" 2>/dev/null; exit 0' TERM INT
wait "$child"

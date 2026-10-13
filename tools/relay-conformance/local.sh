#!/usr/bin/env bash
# Starts storm-relay twice with the fixture contract — allowlist mode and TOFU
# mode — runs the conformance suite against both, and stops them.
#
#   tools/relay-conformance/local.sh RELAY_BIN [FILTER ...]
#
# Ports 18900/18901 (CONFORMANCE_PORT, +1), clear of storm-server's 8484/8485
# and the relay's 8486/8487.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
relay_bin="$1"
shift
port="${CONFORMANCE_PORT:-18900}"
tofu_port=$((port + 1))
log_dir="${CONFORMANCE_LOG_DIR:-$here/../../.dev}"
mkdir -p "$log_dir"

for p in "$port" "$tofu_port"; do
	if (exec 3<>"/dev/tcp/127.0.0.1/$p") 2>/dev/null; then
		echo "Something is already listening on 127.0.0.1:$p; set CONFORMANCE_PORT." >&2
		exit 1
	fi
done

# The knobs, exported exactly as the file spells them.
set -a
# shellcheck source=fixtures/relay.env
. "$here/fixtures/relay.env"
set +a
unset STORM_RELAY_BINDINGS

STORM_RELAY_ALLOWLIST="$here/fixtures/allowlist" \
	"$relay_bin" --bind "127.0.0.1:$port" --public-base "ws://127.0.0.1:$port" \
	>"$log_dir/conformance-relay.log" 2>&1 &
pids=($!)
trap 'kill "${pids[@]}" 2>/dev/null || true' EXIT
env -u STORM_RELAY_ALLOWLIST \
	"$relay_bin" --bind "127.0.0.1:$tofu_port" --public-base "ws://127.0.0.1:$tofu_port" \
	>"$log_dir/conformance-relay-tofu.log" 2>&1 &
pids+=($!)

for p in "$port" "$tofu_port"; do
	for i in $(seq 1 50); do
		(exec 3<>"/dev/tcp/127.0.0.1/$p") 2>/dev/null && break
		if [ "$i" -eq 50 ]; then
			echo "storm-relay did not start on $p; logs in $log_dir" >&2
			exit 1
		fi
		sleep 0.1
	done
done

# Linux answers on all of 127/8; macOS only on 127.0.0.1 unless aliased.
multi_ip=0
[ "$(uname -s)" = Linux ] && multi_ip=1

status=0
RELAY_URL="ws://127.0.0.1:$port" \
	RELAY_TOFU_URL="ws://127.0.0.1:$tofu_port" \
	RELAY_PEER_IP=127.0.0.1 \
	RELAY_MULTI_IP="${RELAY_MULTI_IP:-$multi_ip}" \
	RELAY_LATENCY="${RELAY_LATENCY:-1}" \
	"$here/target/release/relay-conformance" "$@" || status=$?
if [ "$status" -ne 0 ]; then
	echo "relay logs: $log_dir/conformance-relay.log, $log_dir/conformance-relay-tofu.log" >&2
fi
exit "$status"

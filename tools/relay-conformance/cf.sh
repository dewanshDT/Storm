#!/usr/bin/env bash
# Starts the Worker relay (apps/relay-cf) under `wrangler dev` with the fixture
# contract — allowlist and TOFU mode, then open (key-addressed) mode — runs the
# conformance suite against them, and stops them (decisions 108c, 116n).
#
#   tools/relay-conformance/cf.sh [FILTER ...]
#
# Ports 18910-18912 (CONFORMANCE_CF_PORT, +1, +2). No latency check: wrangler
# dev is not the edge, and decision 91's median is storm-relay's promise.
#
# Two passes: the open instance runs only for the open-mode checks, so the
# load-sensitive 16 MiB checks run beside two workerds, as before 116n.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
port="${CONFORMANCE_CF_PORT:-18910}"
tofu_port=$((port + 1))
open_port=$((port + 2))
log_dir="${CONFORMANCE_LOG_DIR:-$root/.dev}"
mkdir -p "$log_dir"

for p in "$port" "$tofu_port" "$open_port"; do
	if (exec 3<>"/dev/tcp/127.0.0.1/$p") 2>/dev/null; then
		echo "Something is already listening on 127.0.0.1:$p; set CONFORMANCE_CF_PORT." >&2
		exit 1
	fi
done

knobs=()
while IFS= read -r line; do
	case "$line" in '' | '#'*) ;; *) knobs+=("$line") ;; esac
done <"$here/fixtures/relay.env"

wait_for() {
	for p in "$@"; do
		for i in $(seq 1 240); do
			curl -s -o /dev/null "http://127.0.0.1:$p/" && break
			if [ "$i" -eq 240 ]; then
				echo "wrangler dev did not start on $p; logs in $log_dir/conformance-cf*.log" >&2
				exit 1
			fi
			sleep 0.25
		done
	done
}

dev="$root/apps/relay-cf/scripts/dev-relay.sh"
"$dev" "$port" allowlist "$here/fixtures/allowlist" "$log_dir/conformance-cf" "${knobs[@]}" \
	>"$log_dir/conformance-cf.log" 2>&1 &
pids=($!)
trap 'kill -TERM "${pids[@]}" 2>/dev/null; wait 2>/dev/null || true' EXIT
"$dev" "$tofu_port" tofu - "$log_dir/conformance-cf-tofu" "${knobs[@]}" \
	>"$log_dir/conformance-cf-tofu.log" 2>&1 &
pids+=($!)
wait_for "$port" "$tofu_port"

multi_ip=0
[ "$(uname -s)" = Linux ] && multi_ip=1

run() {
	RELAY_URL="ws://127.0.0.1:$port" \
		RELAY_PEER_IP=127.0.0.1 \
		RELAY_MULTI_IP="${RELAY_MULTI_IP:-$multi_ip}" \
		RELAY_LATENCY=0 \
		"$here/target/release/relay-conformance" "$@"
}

status=0
RELAY_TOFU_URL="ws://127.0.0.1:$tofu_port" run "$@" || status=$?

kill -TERM "${pids[1]}" 2>/dev/null || true
"$dev" "$open_port" open - "$log_dir/conformance-cf-open" "${knobs[@]}" \
	>"$log_dir/conformance-cf-open.log" 2>&1 &
pids+=($!)
wait_for "$open_port"
echo
RELAY_OPEN_URL="ws://127.0.0.1:$open_port" run --open-only "$@" || status=$?

if [ "$status" -ne 0 ]; then
	echo "relay logs: $log_dir/conformance-cf{,-tofu,-open}.log" >&2
fi
exit "$status"

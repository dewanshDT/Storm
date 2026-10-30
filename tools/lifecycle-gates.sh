#!/bin/sh
# The real-CLI gates of decisions 144 to 149 (G-144-S, G-144-W, G-145-R,
# G-145-W; G-146-A, G-146-M, G-146-P; G-147; G-148; G-149), for Claude Code and
# OpenCode, as the operator runs them on VM 113 before any list is switched
# on. `make gate-lifecycle` runs this. The G-146, G-147 and G-148 gates need
# the stand-in model (it calls the tools that ask) and show as SKIP with
# REAL=1; in G-146-P the test plays the person at the terminal and answers
# the prompt, and in G-147 it presses Shift+Tab (`ESC [ Z`) as the person
# would. G-148 runs a child as the host configures it and shows that no
# prompt ever appears, and that a denial reaches the agent and the host.
# G-149 shows a top-level session's Storm deletes reach Storm with no
# Claude Code prompt first, in manual and in auto.
#
# Procedure, on the VM, as the account the agents run as:
#
#   1. Install the CLIs Storm runs there and sign in to each: `claude`
#      (`/login`) and `opencode auth login`. Note both versions.
#   2. Check out the branch and run, from the repository root:
#
#        make gate-lifecycle
#
#      Each gate runs its CLI in a terminal against a stand-in model server
#      started by the test, with Storm's hooks or plugin as the host writes
#      them when every list is on. Prints `G-… PASS|FAIL` per gate.
#   3. Then with the account's own logins and models:
#
#        make gate-lifecycle REAL=1 OPENCODE_MODEL=<provider/model>
#
#      (Claude Code uses `haiku` unless CLAUDE_MODEL is set.) The checks
#      that need the stand-in, a 429 and an exact reply, show as SKIP.
#   4. By hand, once, with Claude Code: arm a wake while a permission
#      dialog is open (the research saw it start nothing), answer the
#      dialog, and note whether the wake is delivered afterwards.
#   5. Look at how OpenCode shows its wake: a user message with Storm's
#      fixed sentence. The operator accepts that look, or OpenCode's wake
#      stays off.
#   6. Record each gate's result, with the CLI versions and this log, in the
#      decision 144 and 145 notes. Only a gate that passed switches its
#      entry on, in both crates' lists in the same change, spelled the same:
#        - signals: `attention::GATED_SIGNALS` (server) and
#          `harness::GATED_SIGNALS` (runtime), {provider, signal, source};
#        - final message: `round::RESULT_SOURCES` and
#          `harness::RESULT_SOURCES`, {provider, source};
#        - wake: `events::WAKE` and `harness::WAKE`, {provider, mechanism};
#        - a child's permission prompt (G-146-P): `attention::GATED_SIGNALS`
#          and `harness::GATED_SIGNALS`, {provider, "permission_prompt",
#          "provider_hook"};
#        - a child's denials (G-148): `events::DENIALS` and
#          `harness::DENIALS`, the provider.
#      A test in each crate holds the two lists equal.
#
# Variables: CLAUDE_BIN, OPENCODE_BIN (default: on PATH), REAL=1,
# CLAUDE_MODEL, OPENCODE_MODEL, GATES (a test-name filter, e.g. `claude`),
# WORKSPACE_PARENT (real mode: a folder Claude Code already trusts).
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
LOG="${LOG:-${TMPDIR:-/tmp}/lifecycle-gates-$(date +%Y%m%d-%H%M%S).log}"

export STORM_LIFECYCLE_GATE=1
export STORM_CLAUDE_BIN="${CLAUDE_BIN:-$(command -v claude || echo claude)}"
export STORM_OPENCODE_BIN="${OPENCODE_BIN:-$(command -v opencode || echo opencode)}"
if [ "${REAL:-}" = 1 ]; then
	export STORM_GATE_REAL=1
	export STORM_GATE_CLAUDE_MODEL="${CLAUDE_MODEL:-haiku}"
	if [ -z "${OPENCODE_MODEL:-}" ]; then
		echo "REAL=1 needs OPENCODE_MODEL=<provider/model> for OpenCode" >&2
		exit 2
	fi
	export STORM_GATE_OPENCODE_MODEL="$OPENCODE_MODEL"
	[ -n "${WORKSPACE_PARENT:-}" ] && export STORM_GATE_WORKSPACE_PARENT="$WORKSPACE_PARENT"
fi

{
	echo "lifecycle gates, $(date -u +%Y-%m-%dT%H:%M:%SZ), $(git -C "$ROOT" rev-parse --short HEAD)"
	echo "claude:   $STORM_CLAUDE_BIN $("$STORM_CLAUDE_BIN" --version 2>&1 | head -1)"
	echo "opencode: $STORM_OPENCODE_BIN $("$STORM_OPENCODE_BIN" --version 2>&1 | head -1)"
	echo "mode:     $([ "${REAL:-}" = 1 ] && echo "real logins" || echo "stand-in models")"
} | tee "$LOG"

cd "$ROOT/apps/runtime"
status=0
cargo test --quiet --test lifecycle_gate ${GATES:-} -- --nocapture --test-threads 1 \
	>>"$LOG" 2>&1 || status=$?

echo "--- results (the whole run is in $LOG) ---"
grep -E '^G-14[456789]-' "$LOG" | grep -vE ': running$' || true
if [ "$status" -ne 0 ]; then
	echo "a gate failed (exit $status): see $LOG" >&2
fi
exit "$status"

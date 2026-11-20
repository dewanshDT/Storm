#!/usr/bin/env bash
# Local end-to-end smoke test for decision 162 (in-app feedback), miniature
# version of AC-162-1/3/4/5: a Worker under `wrangler dev --local`, a
# throwaway storm-server, a signed-in client call, the sync script pairing
# as its own device, and the off switch — all on this box, all in temp
# dirs and random localhost ports. Never touches prod, :8490, or a
# systemd service.
#
# Usage: deploy/feedback/smoke-local.sh
# Run from the repo root (a 162-feedback worktree with built binaries).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

SERVER_BIN="$ROOT/apps/server/target/debug/storm-server"
if [ ! -x "$SERVER_BIN" ]; then
  echo "building storm-server (debug)..." >&2
  (cd apps/server && cargo build --quiet)
fi

WORK="$(mktemp -d /tmp/storm-feedback-smoke.XXXXXX)"
echo "work dir: $WORK"
VAULT_ROOT="$WORK/vaults"
STATE="$WORK/state"
SYNC_STATE="$WORK/sync-state"
mkdir -p "$VAULT_ROOT" "$STATE" "$SYNC_STATE"

pyport() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }
SERVER_PORT="$(pyport)"
WORKER_PORT="$(pyport)"
SYNC_TOKEN="smoke-sync-token-$$"

SERVER_LOG="$WORK/server.log"
WORKER_LOG="$WORK/worker.log"

PIDS=()
cleanup() {
  for p in "${PIDS[@]:-}"; do
    kill "$p" 2>/dev/null || true
  done
  wait 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

echo "--- starting the feedback Worker locally (wrangler dev, local KV+DO emulation) ---"
[ -x "$ROOT/apps/feedback-cf/node_modules/.bin/wrangler" ] ||
  (cd "$ROOT/apps/feedback-cf" && npm ci --no-audit --no-fund >/dev/null)
WORKER_VARS="$WORK/worker-vars.env"
echo "SYNC_TOKEN=$SYNC_TOKEN" > "$WORKER_VARS"
WRANGLER_SEND_METRICS=false bash -c '
  cd "$1" &&
  exec "./node_modules/.bin/wrangler" dev --port "$2" --ip 127.0.0.1 \
    --inspector-port 0 --show-interactive-dev-session=false \
    --env-file "$3" --persist-to "$4"
' _ "$ROOT/apps/feedback-cf" "$WORKER_PORT" "$WORKER_VARS" "$WORK/worker-do" \
  > "$WORKER_LOG" 2>&1 &
PIDS+=("$!")

WORKER_BASE="http://127.0.0.1:$WORKER_PORT"
for i in $(seq 1 60); do
  curl -sf -o /dev/null "$WORKER_BASE/v1/reports" -H "Authorization: Bearer $SYNC_TOKEN" && break
  if [ "$i" -eq 60 ]; then
    echo "worker did not start within 30s; log follows:" >&2
    cat "$WORKER_LOG" >&2
    exit 1
  fi
  sleep 0.5
done
echo "worker up at $WORKER_BASE"

echo "--- starting a throwaway storm-server ---"
"$SERVER_BIN" serve --vault-root "$VAULT_ROOT" --state "$STATE" \
  --port "$SERVER_PORT" --no-discovery > "$SERVER_LOG" 2>&1 &
PIDS+=("$!")
SERVER_BASE="http://127.0.0.1:$SERVER_PORT"
for i in $(seq 1 60); do
  curl -sf -o /dev/null "$SERVER_BASE/v1/health" && break
  if [ "$i" -eq 60 ]; then
    echo "server did not start within 30s; log follows:" >&2
    cat "$SERVER_LOG" >&2
    exit 1
  fi
  sleep 0.5
done
echo "server up at $SERVER_BASE"

echo "--- claiming the server (bootstrap pairing nonce is single-use) ---"
eval "$(python3 "$ROOT/apps/server/tests/bootstrap.py" "$SERVER_BASE" "$SERVER_LOG")"
export STORM_SESSION STORM_DEVICE
SESSION="$STORM_SESSION"
echo "signed in: $SESSION" | sed 's/Bearer .*/Bearer <redacted>/'

echo "--- creating a vault ---"
VAULT_NAME="Smoke Feedback Vault $$"
VAULT_JSON=$(curl -sf -X POST "$SERVER_BASE/v1/vaults" \
  -H "Authorization: $SESSION" -H 'Content-Type: application/json' \
  -d "{\"name\":\"$VAULT_NAME\"}")
VAULT_ID=$(echo "$VAULT_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
echo "vault id: $VAULT_ID (name: $VAULT_NAME)"

echo "--- pointing feedback.intake_url at the local Worker ---"
curl -sf -X PUT "$SERVER_BASE/v1/config/feedback" \
  -H "Authorization: $SESSION" -H 'Content-Type: application/json' \
  -d "{\"enabled\":true,\"intake_url\":\"$WORKER_BASE\"}" > /dev/null

GOT=$(curl -sf "$SERVER_BASE/v1/feedback" -H "Authorization: $SESSION")
echo "GET /v1/feedback -> $GOT"

echo "--- AC-162-3: preview, then send the exact bytes previewed ---"
# A tiny valid PNG (1x1 transparent pixel), base64.
PNG_B64="iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="

PREVIEW_REQ=$(python3 -c '
import json
print(json.dumps({
    "kind": "bug",
    "include_diagnostics": True,
    "client": {
        "version": "0.19.0-smoke",
        "platform": "linux-x86_64",
        "client_kind": "desktop",
        "screen": "smoke/test",
        "actions": ["opened smoke test"],
        "error": None,
        "connection_log": ["smoke connection log line"],
    },
}))
')
PREVIEW=$(curl -sf -X POST "$SERVER_BASE/v1/feedback/preview" \
  -H "Authorization: $SESSION" -H 'Content-Type: application/json' \
  -d "$PREVIEW_REQ")
PREVIEW_ID=$(echo "$PREVIEW" | python3 -c 'import json,sys; print(json.load(sys.stdin)["preview_id"])')
echo "preview id: $PREVIEW_ID"

SEND_REQ=$(python3 -c "
import json
print(json.dumps({
    'preview_id': '$PREVIEW_ID',
    'kind': 'bug',
    'text': 'Smoke test: the sync button did nothing.',
    'email': 'smoke@example.invalid',
    'screenshot_png_b64': '$PNG_B64',
}))
")
SEND=$(curl -sf -X POST "$SERVER_BASE/v1/feedback" \
  -H "Authorization: $SESSION" -H 'Content-Type: application/json' \
  -d "$SEND_REQ")
REPORT_ID=$(echo "$SEND" | python3 -c 'import json,sys; print(json.load(sys.stdin)["report_id"])')
echo "report queued: $REPORT_ID"

echo "--- waiting for the forwarder to deliver to the Worker ---"
WORKER_REPORT=""
for i in $(seq 1 60); do
  LIST=$(curl -sf "$WORKER_BASE/v1/reports" -H "Authorization: Bearer $SYNC_TOKEN")
  COUNT=$(echo "$LIST" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["reports"]))')
  if [ "$COUNT" -ge 1 ]; then
    WORKER_REPORT="$LIST"
    break
  fi
  if [ "$i" -eq 60 ]; then
    echo "report never reached the Worker" >&2
    exit 1
  fi
  sleep 0.5
done
echo "$WORKER_REPORT" > "$WORK/worker-report.json"
echo "$PREVIEW" > "$WORK/preview.json"
echo "worker has the report."

echo "--- AC-162-3 check: forwarded client/bundle equal the preview's ---"
python3 - "$WORK/worker-report.json" "$WORK/preview.json" <<'PYEOF'
import json, sys
with open(sys.argv[1]) as f:
    listing = json.load(f)
with open(sys.argv[2]) as f:
    preview = json.load(f)
report = listing["reports"][0]
assert report["client"] == preview["client"], "client mismatch: forwarded != previewed"
assert report["bundle"] == preview["bundle"], "bundle mismatch: forwarded != previewed"
print("OK: forwarded client/bundle equal the preview's")
PYEOF

echo "--- AC-162-5 prep: pre-create Storm/Feedback/Feedback.md ---"
FEEDBACK_MD=$'---\ntype: note\n---\n\n# Feedback\n\n## Inbox (new)\n\n_None yet._\n'
curl -sf -X POST "$SERVER_BASE/v1/vaults/$VAULT_ID/notes" \
  -H "Authorization: $SESSION" -H 'Content-Type: application/json' \
  -d "$(python3 -c "import json; print(json.dumps({'path': 'Storm/Feedback/Feedback.md', 'content': '''$FEEDBACK_MD'''}))")" \
  > /dev/null
echo "Feedback.md created."

echo "--- pairing deploy/feedback/feedback-sync.py as its own device ---"
PAIRING=$(curl -sf -X POST "$SERVER_BASE/v1/pairings" \
  -H "Authorization: $SESSION" -H 'Content-Type: application/json' \
  -d '{"purpose":"add_device"}')
NONCE=$(echo "$PAIRING" | python3 -c 'import json,sys; print(json.load(sys.stdin)["n"])')
PAIR_CODE="storm://pair?v=1&n=$NONCE"

printf '%s\n' "correct horse battery staple" | python3 "$ROOT/deploy/feedback/feedback-sync.py" pair \
  --server "$SERVER_BASE" --code "$PAIR_CODE" --name feedback-sync-smoke \
  --password-stdin --state-dir "$SYNC_STATE"

echo "--- running the sync once ---"
STORM_URL="$SERVER_BASE" STORM_VAULT="$VAULT_ID" INTAKE_URL="$WORKER_BASE" \
  SYNC_TOKEN="$SYNC_TOKEN" STATE_DIR="$SYNC_STATE" \
  python3 "$ROOT/deploy/feedback/feedback-sync.py" run

TREE=$(curl -sf "$SERVER_BASE/v1/vaults/$VAULT_ID/tree" -H "Authorization: $SESSION")
FB_NOTE_ID=$(echo "$TREE" | python3 -c '
import json, sys
tree = json.load(sys.stdin)
for n in tree.get("notes", []):
    if n.get("path") == "Storm/Feedback/Inbox/FB-0001.md":
        print(n["id"]); break
')
if [ -z "${FB_NOTE_ID:-}" ]; then
  echo "FB-0001.md was not created" >&2
  exit 1
fi
FB_CONTENT=$(curl -sf "$SERVER_BASE/v1/vaults/$VAULT_ID/notes/$FB_NOTE_ID" -H "Authorization: $SESSION" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["content"])')
echo "$FB_CONTENT" > "$WORK/FB-0001.md"
echo "FB-0001.md:"
grep -E '^(type|ref|intake|status|kind):' "$WORK/FB-0001.md" || true

echo "--- checking the screenshot attachment ---"
curl -sf "$SERVER_BASE/v1/vaults/$VAULT_ID/attachments/Storm/Feedback/Inbox/FB-0001.png" \
  -H "Authorization: $SESSION" -o "$WORK/FB-0001.png"
[ -s "$WORK/FB-0001.png" ] && echo "screenshot attachment present ($(wc -c < "$WORK/FB-0001.png") bytes)"

echo "--- checking Feedback.md got a line under Inbox (new) ---"
FEEDBACK_NOTE_ID=$(echo "$TREE" | python3 -c '
import json, sys
tree = json.load(sys.stdin)
for n in tree.get("notes", []):
    if n.get("path") == "Storm/Feedback/Feedback.md":
        print(n["id"]); break
')
FEEDBACK_CONTENT=$(curl -sf "$SERVER_BASE/v1/vaults/$VAULT_ID/notes/$FEEDBACK_NOTE_ID" -H "Authorization: $SESSION" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["content"])')
echo "$FEEDBACK_CONTENT" | grep -q "FB-0001" && echo "Feedback.md has the FB-0001 line." || { echo "Feedback.md missing the FB-0001 line" >&2; exit 1; }

echo "--- checking the Worker queue is now empty ---"
AFTER=$(curl -sf "$WORKER_BASE/v1/reports" -H "Authorization: Bearer $SYNC_TOKEN")
AFTER_COUNT=$(echo "$AFTER" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["reports"]))')
[ "$AFTER_COUNT" -eq 0 ] && echo "worker queue empty." || { echo "worker queue not empty after sync ($AFTER_COUNT left)" >&2; exit 1; }

echo "--- running sync again: no duplicate ---"
STORM_URL="$SERVER_BASE" STORM_VAULT="$VAULT_ID" INTAKE_URL="$WORKER_BASE" \
  SYNC_TOKEN="$SYNC_TOKEN" STATE_DIR="$SYNC_STATE" \
  python3 "$ROOT/deploy/feedback/feedback-sync.py" run
TREE2=$(curl -sf "$SERVER_BASE/v1/vaults/$VAULT_ID/tree" -H "Authorization: $SESSION")
FB_COUNT=$(echo "$TREE2" | python3 -c '
import json, sys
tree = json.load(sys.stdin)
print(sum(1 for n in tree.get("notes", []) if n.get("path", "").startswith("Storm/Feedback/Inbox/") and n["path"].endswith(".md")))
')
[ "$FB_COUNT" -eq 1 ] && echo "still exactly one report note: no duplicate." || { echo "expected 1 report note, found $FB_COUNT" >&2; exit 1; }

echo "--- AC-162-5: the off switch ---"
curl -sf -X PUT "$SERVER_BASE/v1/config/feedback" \
  -H "Authorization: $SESSION" -H 'Content-Type: application/json' \
  -d '{"enabled":false}' > /dev/null

OFF_STATUS=$(curl -sf "$SERVER_BASE/v1/feedback" -H "Authorization: $SESSION")
echo "GET /v1/feedback -> $OFF_STATUS"

PREVIEW_OFF_CODE=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$SERVER_BASE/v1/feedback/preview" \
  -H "Authorization: $SESSION" -H 'Content-Type: application/json' -d "$PREVIEW_REQ")
[ "$PREVIEW_OFF_CODE" = "403" ] && echo "preview refused with 403 while off." || { echo "expected 403 from preview while off, got $PREVIEW_OFF_CODE" >&2; exit 1; }

SEND_OFF_CODE=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$SERVER_BASE/v1/feedback" \
  -H "Authorization: $SESSION" -H 'Content-Type: application/json' \
  -d "{\"preview_id\":\"$PREVIEW_ID\",\"kind\":\"bug\",\"text\":\"should not send\"}")
[ "$SEND_OFF_CODE" = "403" ] && echo "send refused with 403 while off." || { echo "expected 403 from send while off, got $SEND_OFF_CODE" >&2; exit 1; }

STILL_EMPTY=$(curl -sf "$WORKER_BASE/v1/reports" -H "Authorization: Bearer $SYNC_TOKEN")
STILL_EMPTY_COUNT=$(echo "$STILL_EMPTY" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["reports"]))')
[ "$STILL_EMPTY_COUNT" -eq 0 ] && echo "nothing reached the Worker while off." || { echo "something reached the Worker while off!" >&2; exit 1; }

echo "--- scanning for leaked secrets (vault name, hostname, username, tokens) ---"
HOSTNAME_VAL="$(hostname)"
USERNAME_VAL="$(whoami)"
LEAK=0
for needle in "$VAULT_NAME" "$HOSTNAME_VAL" "$USERNAME_VAL"; do
  if grep -qF -- "$needle" "$WORK/FB-0001.md" "$WORK/worker-report.json"; then
    echo "LEAK: '$needle' found in the note or the Worker's stored JSON" >&2
    LEAK=1
  fi
done
if grep -qE '\b(sta_|stk_|sht_|dvs_|dev_)[A-Za-z0-9_-]+' "$WORK/FB-0001.md" "$WORK/worker-report.json"; then
  echo "LEAK: a token-shaped string found in the note or the Worker's stored JSON" >&2
  LEAK=1
fi
if [ "$LEAK" -eq 0 ]; then
  echo "clean: no vault name, hostname, username or token found."
else
  exit 1
fi

echo
echo "=== smoke test passed ==="

#!/usr/bin/env bats
# Secrets: redacted from the log, read with echo off, and handed to the
# programs that need them on stdin, never in argv.

load helpers

setup() {
  setup_world
}

@test "log_redact hides each kind of secret" {
  out=$(printf '%s\n' \
    'token sk-ant-oat01-AbC_d-123 done' \
    'got storm-enroll:v1:eyJhIjoxfQ.sig ok' \
    'CLAUDE_CODE_OAUTH_TOKEN=sk-ant-x' \
    'export CLAUDE_CODE_OAUTH_TOKEN="whatever123"' | log_redact)
  [[ "$out" != *"AbC_d"* ]]
  [[ "$out" != *"eyJhIjoxfQ"* ]]
  [[ "$out" != *"whatever123"* ]]
  [[ "$out" == *"sk-ant-[redacted] done"* ]]
  [[ "$out" == *"storm-enroll:v1:[redacted] ok"* ]]
  [[ "$out" == *"CLAUDE_CODE_OAUTH_TOKEN=[redacted]"* ]]
}

@test "log() and logged() write only redacted text" {
  platform
  log_init test
  log "pasted storm-enroll:v1:SECRET1"
  logged echo "child said sk-ant-SECRET2"
  [ -s "$LOG_FILE" ]
  ! grep -q SECRET "$LOG_FILE"
  grep -q 'storm-enroll:v1:\[redacted\]' "$LOG_FILE"
  # Only its owner can read it.
  [ "$(file_mode "$LOG_FILE")" = 600 ]
}

@test "write_claude_env: the token reaches the file through stdin only" {
  platform
  mkdir -p "$RT_STATE"
  printf '%s\n' "sk-ant-oat01-TOKENSECRET" | write_claude_env
  [ "$(cat "$RT_STATE/claude.env")" = "CLAUDE_CODE_OAUTH_TOKEN=sk-ant-oat01-TOKENSECRET" ]
  [ "$(file_mode "$RT_STATE/claude.env")" = 600 ]
  grep -q "sudo -u storm-runtime sh -c" "$FAKE_LOG"
  ! grep -q TOKENSECRET "$FAKE_LOG"
}

@test "the providers block, appended only when none is listed" {
  platform
  mkdir -p "$(dirname "$RT_CONFIG")" "$RT_STATE"
  printf 'workspaces = []\n# [[providers]] is commented out\n' >"$RT_CONFIG"
  touch "$RT_STATE/claude.env"
  agents_providers
  grep -q '^id = "claude-code"$' "$RT_CONFIG"
  grep -q "^env_file = \"$RT_STATE/claude.env\"$" "$RT_CONFIG"
  grep -q '^id = "opencode"$' "$RT_CONFIG"
  grep -q '^id = "shell"$' "$RT_CONFIG"
  [ "$AGENTS_CHANGED" = 1 ]
  before=$(cat "$RT_CONFIG")
  AGENTS_CHANGED=0
  agents_providers
  [ "$(cat "$RT_CONFIG")" = "$before" ]
  [ "$AGENTS_CHANGED" = 0 ]
}

@test "no token yet: runtime.toml keeps its defaults" {
  platform
  mkdir -p "$(dirname "$RT_CONFIG")"
  printf 'x = 1\n' >"$RT_CONFIG"
  run agents_providers
  [ "$status" -eq "$STEP_SKIPPED" ]
  [ "$(cat "$RT_CONFIG")" = "x = 1" ]
}

# An interactive session whose "terminal" is a file of typed lines.
typed() {
  printf '%s\n' "$@" >"$BATS_TEST_TMPDIR/tty.in"
  export STORM_TTY="$BATS_TEST_TMPDIR/tty.in" STORM_TTY_OUT="$BATS_TEST_TMPDIR/tty.out"
  OPT_YES=0
  ui_init
}

@test "interactive Claude token: wrong shape re-asked, then saved; never on screen or argv" {
  platform
  mkdir -p "$RT_STATE"
  typed "not-a-token" "sk-ant-oat01-TYPEDSECRET"
  WORK_DIR="$BATS_TEST_TMPDIR"
  agents_claude_token
  [ "$(cat "$RT_STATE/claude.env")" = "CLAUDE_CODE_OAUTH_TOKEN=sk-ant-oat01-TYPEDSECRET" ]
  grep -q "That isn't a Claude Code token" "$STORM_TTY_OUT"
  ! grep -q TYPEDSECRET "$STORM_TTY_OUT"
  ! grep -q TYPEDSECRET "$FAKE_LOG"
  [ "$AGENTS_CHANGED" = 1 ]
}

@test "interactive Claude token: Enter alone skips" {
  platform
  typed ""
  run agents_claude_token
  [ "$status" -eq "$STEP_SKIPPED" ]
  [ ! -e "$RT_STATE/claude.env" ]
}

@test "interactive enrollment paste: shape checked, piped to enroll" {
  platform
  install_fake linux-runtime
  typed "storm-enroll:v1:PASTED-SECRET-9"
  RT_ENROLL_ACTION=enroll
  LOG_FILE="$BATS_TEST_TMPDIR/install.log"
  runtime_enroll_paste
  grep -q 'storm-runtime enroll: storm-enroll:v1:PASTED-SECRET-9' "$FAKE_STDIN"
  ! grep -q PASTED-SECRET "$FAKE_LOG"
  ! grep -q PASTED-SECRET "$STORM_TTY_OUT"
  ! grep -q PASTED-SECRET "$LOG_FILE"
  grep -q 'Agents › Hosts › Enroll a host' "$STORM_TTY_OUT"
}

@test "pasted text is trimmed of bracketed-paste markers and spaces" {
  platform
  typed $'\033[200~  storm-enroll:v1:abc  \033[201~'
  out=$(ui_secret_pipe "x" enroll_string_ok "bad" cat)
  [ "$out" = "storm-enroll:v1:abc" ]
}

@test "the local enrollment string is piped, never captured in argv" {
  platform
  install_fake linux-server
  install_fake linux-runtime
  touch "$FAKE_DIR/account"
  DATA_ROOT="$STORM_TEST_SYSROOT/srv/storm" PORT=8484 RT_ENROLL_ACTION=enroll
  runtime_enroll_local
  grep -q 'storm-runtime enroll: storm-enroll:v1:LOCALSECRET' "$FAKE_STDIN"
  ! grep -q LOCALSECRET "$FAKE_LOG"
  grep -q 'storm-server host-enrollment --state .* --url http://127.0.0.1:8484' "$FAKE_LOG"
}

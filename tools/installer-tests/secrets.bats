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

# An interactive session whose "terminal" is a file of typed lines.
typed() {
  printf '%s\n' "$@" >"$BATS_TEST_TMPDIR/tty.in"
  export STORM_TTY="$BATS_TEST_TMPDIR/tty.in" STORM_TTY_OUT="$BATS_TEST_TMPDIR/tty.out"
  OPT_YES=0
  ui_init
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

@test "interactive first workspace: a bad name is re-asked, then made" {
  platform
  mkdir -p "$RT_WORKSPACES"
  typed "a/b" "notes"
  LOG_FILE="$BATS_TEST_TMPDIR/install.log"
  runtime_workspace
  [ -d "$RT_WORKSPACES/notes" ]
  grep -q "That isn't a folder name" "$STORM_TTY_OUT"
  grep -q "There's none yet" "$STORM_TTY_OUT"
}

@test "interactive first workspace: Enter alone skips; one already there is kept" {
  platform
  mkdir -p "$RT_WORKSPACES"
  typed ""
  run runtime_workspace
  [ "$status" -eq "$STEP_SKIPPED" ]
  [ -z "$(ls -A "$RT_WORKSPACES")" ]
  mkdir "$RT_WORKSPACES/site"
  typed "never-read"
  runtime_workspace
  grep -q "already has workspaces: site" "$STORM_TTY_OUT"
  [ ! -e "$RT_WORKSPACES/never-read" ]
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

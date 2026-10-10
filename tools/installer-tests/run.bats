#!/usr/bin/env bats
# Running plans against the fakes: sudo only in the steps marked sudo,
# secrets only on stdin, re-runs that change nothing, failures that stop.

load helpers

setup() {
  setup_world
  make_assets 0.6.0
}

# The plan_print part of a run's output (it is on stderr, which run merges).
sudo_marked_steps() {
  plan_steps "$1" | awk '$2=="sudo"{print $1}' | sort -un
}

@test "linux everything --yes: installs, pairs by password, enrolls locally" {
  run storm_main --yes --everything --from-dir "$ASSETS" --addr 10.0.0.5 --password-stdin <<<"hunter2-not-in-argv"
  [ "$status" -eq 0 ]
  [ -x "$FAKE_DIR/installed/storm-server" ]
  [ -x "$FAKE_DIR/installed/storm-runtime" ]
  [ -f "$STORM_TEST_SYSROOT/var/lib/storm-runtime/host.json" ]
  grep -q 'apt-get install -y ./storm-server_0.6.0-1_amd64.deb' "$FAKE_LOG"
  grep -q 'storm-server up --data-root .*/srv/storm --port 8484 --host 0.0.0.0' "$FAKE_LOG"
  grep -q 'systemctl enable --now storm-runtime' "$FAKE_LOG"
  grep -q 'storm-runtime check' "$FAKE_LOG"
  # The password and the enrollment string went over stdin, never argv.
  grep -q 'storm-server passwd: hunter2-not-in-argv' "$FAKE_STDIN"
  grep -q 'storm-runtime enroll: storm-enroll:v1:LOCALSECRET-fromhost-0001' "$FAKE_STDIN"
  ! grep -q 'hunter2' "$FAKE_LOG"
  ! grep -q 'LOCALSECRET' "$FAKE_LOG"
  # sudo ran only inside steps the plan marked sudo.
  marked=$(sudo_marked_steps "$output")
  used=$(sudo_steps_used)
  [ -n "$used" ]
  for s in $used; do
    [[ " $(echo $marked) " == *" $s "* ]] || { echo "step $s used sudo but isn't marked"; false; }
  done
  # The log exists and holds no secret.
  log="$HOME/.local/state/storm/install.log"
  [ -s "$log" ]
  ! grep -q 'LOCALSECRET' "$log"
  ! grep -q 'hunter2' "$log"
  [[ "$output" == *"Summary"* ]]
}

@test "linux: every sudo-marked step that runs uses sudo, and no other step does" {
  run storm_main --yes --server --runtime --from-dir "$ASSETS" --addr 10.0.0.5 --password-stdin <<<"pw"
  [ "$status" -eq 0 ]
  marked=$(sudo_marked_steps "$output" | tr '\n' ' ')
  used=$(sudo_steps_used | tr '\n' ' ')
  [ "$marked" = "$used" ]
}

@test "macos everything --yes: user-level server, sudo runtime, app installed" {
  use_macos
  run storm_main --yes --everything --from-dir "$ASSETS" --addr 10.0.0.5 --password-stdin <<<"pw-mac"
  [ "$status" -eq 0 ]
  [ -x "$HOME/Library/Application Support/Storm/bin/storm-server" ]
  [ -x "$STORM_TEST_SYSROOT/Library/StormRuntime/bin/storm-runtime" ]
  [ -f "$STORM_TEST_SYSROOT/Library/StormRuntime/state/host.json" ]
  [ -d "$STORM_TEST_SYSROOT/Applications/Storm.app" ]
  [ -x "$STORM_TEST_SYSROOT/opt/homebrew/bin/claude" ]
  [ -x "$STORM_TEST_SYSROOT/opt/homebrew/bin/opencode" ]
  grep -q '^step=[0-9]* storm-server up --data-root '"$HOME"'/Storm --port 8484 --host 0.0.0.0' "$FAKE_LOG"
  grep -q 'sudo -n ./storm-runtime install --operator' "$FAKE_LOG"
  # The server never goes through sudo on macOS.
  ! grep -q 'sudo .*storm-server' "$FAKE_LOG"
  grep -q 'storm-runtime enroll: storm-enroll:v1:LOCALSECRET' "$FAKE_STDIN"
  ! grep -q 'LOCALSECRET' "$FAKE_LOG"
  marked=$(sudo_marked_steps "$output" | tr '\n' ' ')
  used=$(sudo_steps_used | tr '\n' ' ')
  [ "$marked" = "$used" ]
  [ -s "$HOME/Library/Logs/Storm/install.log" ]
  [[ "$output" == *"macOS may ask whether"* ]]
  [[ "$output" == *"stops at logout"* ]]
}

@test "macos downloads are verified against checksums.txt" {
  use_macos
  # A tampered tarball: same name, different bytes.
  echo tampered >"$ASSETS/storm-runtime-0.6.0-macos-universal.tar.gz"
  run storm_main --yes --runtime --version v0.6.0 --addr 10.0.0.5 --enroll-from-stdin <<<"storm-enroll:v1:abc"
  [ "$status" -eq 1 ]
  grep -q 'checksum mismatch for storm-runtime-0.6.0-macos-universal.tar.gz' "$HOME/Library/Logs/Storm/install.log"
  ! grep -q 'storm-runtime install' "$FAKE_LOG"
}

@test "macos downloads from the release URL when not --from-dir" {
  use_macos
  run storm_main --yes --server --addr 10.0.0.5 --password-stdin <<<"pw"
  [ "$status" -eq 0 ]
  grep -q 'curl .*api.github.com/repos/dewanshDT/Storm/releases/latest' "$FAKE_LOG"
  grep -q 'curl .*releases/download/v0.6.0/checksums.txt' "$FAKE_LOG"
  grep -q 'curl .*releases/download/v0.6.0/storm-server-0.6.0-macos-universal.tar.gz' "$FAKE_LOG"
}

@test "runtime only, enrollment string from stdin: it reaches enroll on stdin only" {
  run storm_main --yes --runtime --from-dir "$ASSETS" --enroll-from-stdin <<<"storm-enroll:v1:PASTEDSECRET-123"
  [ "$status" -eq 0 ]
  grep -q 'storm-runtime enroll: storm-enroll:v1:PASTEDSECRET-123' "$FAKE_STDIN"
  ! grep -q 'PASTEDSECRET' "$FAKE_LOG"
  ! grep -q 'PASTEDSECRET' "$HOME/.local/state/storm/install.log"
  [[ "$output" != *"PASTEDSECRET"* ]]
}

@test "runtime only: a malformed enrollment string is refused and not echoed" {
  run storm_main --yes --runtime --from-dir "$ASSETS" --enroll-from-stdin <<<"sk-ant-oops-wrong-thing"
  [ "$status" -eq 1 ]
  [[ "$output" == *"didn't hold an enrollment string"* ]]
  [[ "$output" != *"sk-ant-oops"* ]]
  ! grep -q 'enroll' "$FAKE_STDIN"
}

@test "re-running on an installed machine changes nothing" {
  run storm_main --yes --everything --from-dir "$ASSETS" --addr 10.0.0.5 --password-stdin <<<"pw"
  [ "$status" -eq 0 ]
  : >"$FAKE_LOG"
  run storm_main --yes --everything --from-dir "$ASSETS" --addr 10.0.0.5
  [ "$status" -eq 0 ]
  ! grep -q 'apt-get' "$FAKE_LOG"
  ! grep -q 'storm-server up' "$FAKE_LOG"
  ! grep -q 'enroll' "$FAKE_LOG"
  ! grep -q 'passwd\|pair' "$FAKE_LOG"
}

@test "a failing step stops a non-interactive run and points at the log" {
  export FAKE_HEALTH_DOWN=1
  run storm_main --yes --server --from-dir "$ASSETS" --addr 10.0.0.5 --password-stdin <<<"pw"
  [ "$status" -eq 1 ]
  [[ "$output" == *"Check the server answers on port 8484"* ]]
  [[ "$output" == *"The log has the details"* ]]
  ! grep -q 'passwd' "$FAKE_LOG"
}

@test "without a password the account step shows the pairing QR and carries on" {
  run storm_main --yes --server --from-dir "$ASSETS" --addr 10.0.0.5
  [ "$status" -eq 0 ]
  grep -q 'storm-server pair --state .*/srv/storm/state --addr 10.0.0.5:8484 --qr' "$FAKE_LOG"
  [[ "$output" == *"[pairing qr]"* ]]
}

@test "local enrollment without an account fails clearly" {
  run storm_main --yes --server --runtime --from-dir "$ASSETS" --addr 10.0.0.5
  [ "$status" -eq 1 ]
  grep -q 'the server has no account yet' "$HOME/.local/state/storm/install.log"
}

@test "upgrade --yes upgrades an older install" {
  install_fake linux-server
  install_fake linux-runtime
  mkdir -p "$STORM_TEST_SYSROOT/var/lib/storm-runtime"
  touch "$STORM_TEST_SYSROOT/var/lib/storm-runtime/host.json" "$FAKE_DIR/account"
  export FAKE_SERVER_VERSION=0.5.0 FAKE_RUNTIME_VERSION=0.5.0
  run storm_main --yes --from-dir "$ASSETS" upgrade
  [ "$status" -eq 0 ]
  grep -q 'apt-get install -y ./storm-server_0.6.0-1_amd64.deb' "$FAKE_LOG"
  grep -q 'systemctl restart storm-server' "$FAKE_LOG"
  grep -q 'apt-get install -y ./storm-runtime_0.6.0-1_amd64.deb' "$FAKE_LOG"
  grep -q 'systemctl try-restart storm-runtime' "$FAKE_LOG"
  ! grep -q 'enroll' "$FAKE_LOG"
}

@test "uninstall --yes removes packages and says what remains" {
  install_fake linux-server
  install_fake linux-runtime
  mkdir -p "$STORM_TEST_SYSROOT/srv/storm/vaults/personal"
  run storm_main --yes uninstall --server --runtime
  [ "$status" -eq 0 ]
  grep -q 'apt-get remove -y storm-server' "$FAKE_LOG"
  grep -q 'apt-get remove -y storm-runtime' "$FAKE_LOG"
  [ -d "$STORM_TEST_SYSROOT/srv/storm/vaults/personal" ]
  [[ "$output" == *"What remains"* ]]
  [[ "$output" == *"Your data: $STORM_TEST_SYSROOT/srv/storm"* ]]
  [[ "$output" == *"workspaces"* ]]
}

@test "uninstall --yes without components refuses" {
  install_fake linux-server
  run storm_main --yes uninstall
  [ "$status" -eq 2 ]
  [[ "$output" == *"say which installed components to remove"* ]]
}

@test "status works without a terminal and changes nothing" {
  install_fake linux-server
  touch "$FAKE_DIR/account"
  run storm_main --from-dir "$ASSETS" status
  [ "$status" -eq 0 ]
  [[ "$output" == *"Storm server"*"installed v0.6.0"*"account: yes"* ]]
  [[ "$output" == *"Runtime Host"*"not installed"* ]]
  [[ "$output" == *"Remote access"*"coming soon"* ]]
  ! grep -q 'install\|up \|enroll' "$FAKE_LOG"
}

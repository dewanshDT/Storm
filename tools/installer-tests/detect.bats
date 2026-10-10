#!/usr/bin/env bats
# Detection of what is installed, against fixtures and fake binaries.

load helpers

setup() {
  setup_world
}

@test "a fresh linux machine has nothing" {
  platform
  detect_all
  [ "$SRV_INSTALLED" = 0 ]
  [ "$RT_INSTALLED" = 0 ]
  [ "$APP_INSTALLED" = 0 ]
  [ "$SRV_DATA" = "$STORM_TEST_SYSROOT/srv/storm" ]
}

@test "linux: server version, port, data root, health and account" {
  install_fake linux-server
  touch "$FAKE_DIR/account"
  mkdir -p "$STORM_TEST_SYSROOT/etc/systemd/system/storm-server.service.d"
  printf '[Service]\nUser=me\nReadWritePaths=/mnt/notes /mnt/notes/state\n' \
    >"$STORM_TEST_SYSROOT/etc/systemd/system/storm-server.service.d/data-root.conf"
  export FAKE_PORT=9090
  platform
  server_detect
  [ "$SRV_INSTALLED" = 1 ]
  [ "$SRV_VERSION" = 0.6.0 ]
  [ "$SRV_PORT" = 9090 ]
  [ "$SRV_DATA" = /mnt/notes ]
  [ "$SRV_RUNNING" = yes ]
  [ "$SRV_ACCOUNT" = yes ]
  grep -q 'sudo -u storm storm-server has-account --state /mnt/notes/state' "$FAKE_LOG"
}

@test "linux: no account, server down" {
  install_fake linux-server
  export FAKE_HEALTH_DOWN=1
  platform
  server_detect
  [ "$SRV_RUNNING" = no ]
  [ "$SRV_ACCOUNT" = no ]
}

@test "linux: runtime installed and enrolled" {
  install_fake linux-runtime
  mkdir -p "$STORM_TEST_SYSROOT/var/lib/storm-runtime"
  touch "$STORM_TEST_SYSROOT/var/lib/storm-runtime/host.json"
  export FAKE_RUNTIME_VERSION=0.5.1
  platform
  runtime_detect
  [ "$RT_INSTALLED" = 1 ]
  [ "$RT_VERSION" = 0.5.1 ]
  [ "$RT_ENROLLED" = yes ]
  [ "$RT_RUNNING" = yes ]
}

@test "linux: runtime installed, not enrolled, not running" {
  install_fake linux-runtime
  export FAKE_RUNTIME_INACTIVE=3
  platform
  runtime_detect
  [ "$RT_ENROLLED" = no ]
  [ "$RT_RUNNING" = no ]
}

@test "without passwordless sudo, enrollment is unknown rather than guessed" {
  install_fake linux-runtime
  platform
  sudo() { return 1; }
  runtime_detect
  [ "$RT_ENROLLED" = unknown ]
}

@test "macOS: server from its LaunchAgent, runtime, app" {
  use_macos
  install_fake macos-server Notes
  install_fake macos-runtime
  install_fake macos-app
  touch "$STORM_TEST_SYSROOT/Library/StormRuntime/state/host.json"
  export FAKE_APP_VERSION=0.6.0
  platform
  detect_all
  [ "$SRV_INSTALLED" = 1 ]
  [ "$SRV_DATA" = "$HOME/Notes" ]
  [ "$SRV_RUNNING" = yes ]
  [ "$RT_INSTALLED" = 1 ]
  [ "$RT_ENROLLED" = yes ]
  [ "$RT_RUNNING" = yes ]
  [ "$APP_INSTALLED" = 1 ]
  [ "$APP_VERSION" = 0.6.0 ]
  # The user's own server: has-account runs without sudo.
  grep -q "storm-server has-account --state $HOME/Notes/state" "$FAKE_LOG"
  ! grep -q 'sudo .*has-account' "$FAKE_LOG"
}

@test "agents: CLIs where the host can run them, and ones it can't" {
  use_macos
  platform
  mkdir -p "$STORM_TEST_SYSROOT/opt/homebrew/bin" "$HOME/.local/bin"
  cp "$FAKE_DIR/tools/opencode" "$STORM_TEST_SYSROOT/opt/homebrew/bin/opencode"
  cp "$FAKE_DIR/tools/claude" "$HOME/.local/bin/claude"
  agents_detect
  [ "$OPENCODE_BIN" = "$STORM_TEST_SYSROOT/opt/homebrew/bin/opencode" ]
  [ -z "$CLAUDE_BIN" ]
  [ "$CLAUDE_IN_HOME" = "$HOME/.local/bin/claude" ]
  [ -n "$BREW_BIN" ]
}

@test "agents: providers already listed in runtime.toml" {
  use_macos
  platform
  mkdir -p "$STORM_TEST_SYSROOT/Library/StormRuntime"
  printf '# [[providers]]\n' >"$RT_CONFIG"
  agents_detect
  [ "$AG_PROVIDERS" = no ]
  printf '  [[providers]]\nid = "shell"\n' >>"$RT_CONFIG"
  agents_detect
  [ "$AG_PROVIDERS" = yes ]
}

@test "a dry run detects from files only and runs nothing" {
  install_fake linux-server
  install_fake linux-runtime
  OPT_DRY_RUN=1
  platform
  detect_all
  [ "$SRV_INSTALLED" = 1 ]
  [ "$RT_INSTALLED" = 1 ]
  [ "$SRV_ACCOUNT" = unknown ]
  [ "$RT_ENROLLED" = unknown ]
  [ ! -s "$FAKE_LOG" ]
}

@test "the state directory's owner runs state commands" {
  platform
  mkdir -p "$BATS_TEST_TMPDIR/state"
  [ "$(srv_owner "$BATS_TEST_TMPDIR/state")" = tester ]
  [ "$(srv_owner_prefix "$BATS_TEST_TMPDIR/state")" = "" ]
  [ "$(srv_owner_sudo "$BATS_TEST_TMPDIR/state")" = 0 ]
  # Not there (or unreadable): Linux assumes storm.
  [ "$(srv_owner /nonexistent/state)" = storm ]
  [ "$(srv_owner_prefix /nonexistent/state)" = "sudo -u storm " ]
  [ "$(srv_owner_sudo /nonexistent/state)" = 1 ]
}

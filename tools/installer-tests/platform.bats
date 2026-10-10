#!/usr/bin/env bats
# OS and architecture detection, and the per-OS paths.

load helpers

setup() {
  setup_world
}

@test "ubuntu x86_64" {
  platform
  [ "$OS" = linux ]
  [ "$ARCH" = x86_64 ]
  [ "$DEBIAN_LIKE" = 1 ]
  [ "$OS_ID" = ubuntu ]
  [ "$OS_LABEL" = "Ubuntu 24.04 LTS" ]
}

@test "debian, and a derivative through ID_LIKE" {
  os_release debian
  platform
  [ "$DEBIAN_LIKE" = 1 ]
  os_release mint
  platform
  [ "$DEBIAN_LIKE" = 1 ]
}

@test "fedora: the server and runtime are unavailable, with a pointer to the binaries" {
  os_release fedora
  platform
  [ "$DEBIAN_LIKE" = 0 ]
  REASON=""
  run server_available
  [ "$status" -eq 1 ]
  server_available || :
  [[ "$REASON" == *"Debian and Ubuntu (this is fedora)"* ]]
  [[ "$REASON" == *"https://github.com/dewanshDT/Storm/releases"* ]]
}

@test "linux arm64: the packages are x86_64 only" {
  use_linux aarch64
  platform
  [ "$ARCH" = arm64 ]
  server_available || :
  [[ "$REASON" == *"built for x86_64 (this machine is arm64)"* ]]
}

@test "macOS on Apple silicon" {
  use_macos arm64
  platform
  [ "$OS" = macos ]
  [ "$ARCH" = arm64 ]
  [ "$OS_LABEL" = "macOS 15.1" ]
  [ "$MACHINE" = Mac ]
}

@test "macOS on Intel" {
  use_macos x86_64
  platform
  [ "$ARCH" = x86_64 ]
}

@test "anything else is unsupported, and says so" {
  export FAKE_UNAME_S=FreeBSD
  platform
  [ "$OS" = other ]
  run storm_main --yes --server
  [ "$status" -eq 1 ]
  [[ "$output" == *"supports macOS and Debian/Ubuntu Linux"* ]]
}

@test "macOS paths" {
  use_macos
  platform
  [ "$RT_ACCOUNT" = _stormruntime ]
  [ "$RT_BIN" = "$STORM_TEST_SYSROOT/Library/StormRuntime/bin/storm-runtime" ]
  [ "$RT_STATE" = "$STORM_TEST_SYSROOT/Library/StormRuntime/state" ]
  [ "$RT_CONFIG" = "$STORM_TEST_SYSROOT/Library/StormRuntime/runtime.toml" ]
  [ "$SRV_INSTALLED_BIN" = "$HOME/Library/Application Support/Storm/bin/storm-server" ]
  [ "$SRV_PLIST" = "$HOME/Library/LaunchAgents/dev.storm.server.plist" ]
  [ "$DEFAULT_DATA" = "$HOME/Storm" ]
  [ "$LOG_FILE" = "$HOME/Library/Logs/Storm/install.log" ]
}

@test "linux paths, and the log under XDG_STATE_HOME" {
  platform
  [ "$RT_ACCOUNT" = storm-runtime ]
  [ "$RT_STATE" = "$STORM_TEST_SYSROOT/var/lib/storm-runtime" ]
  [ "$RT_CONFIG" = "$STORM_TEST_SYSROOT/etc/storm-runtime/runtime.toml" ]
  [ "$DEFAULT_DATA" = "$STORM_TEST_SYSROOT/srv/storm" ]
  [ "$LOG_FILE" = "$HOME/.local/state/storm/install.log" ]
  XDG_STATE_HOME="$BATS_TEST_TMPDIR/xdg" platform
  [ "$LOG_FILE" = "$BATS_TEST_TMPDIR/xdg/storm/install.log" ]
}

@test "without the test flag the sysroot is ignored" {
  STORM_INSTALL_TEST=0 platform
  [ "$RT_STATE" = /var/lib/storm-runtime ]
  [ "$DEFAULT_DATA" = /srv/storm ]
}

@test "the LAN address: ip route on Linux, ipconfig on macOS" {
  platform
  [ "$(lan_addr)" = 192.168.1.10 ]
  use_macos
  platform
  [ "$(lan_addr)" = 192.168.1.20 ]
}

@test "macOS refuses to run as root" {
  use_macos
  id() { if [ "$1" = -u ]; then echo 0; else echo root; fi; }
  run storm_main --yes --server
  [ "$status" -eq 2 ]
  [[ "$output" == *"Run the installer as yourself, not with sudo"* ]]
}

#!/usr/bin/env bats
# MCP only: storm-runtime on PATH for `mcp-connect`, and nothing else -- no
# package, no account, no service, no enrollment.

load helpers

setup() {
  setup_world
  make_assets 0.6.0
}

mcp_bin() { printf '%s' "$STORM_TEST_SYSROOT/usr/local/bin/storm-runtime"; }

typed() {
  printf '%s\n' "$@" >"$BATS_TEST_TMPDIR/tty.in"
  export STORM_TTY="$BATS_TEST_TMPDIR/tty.in" STORM_TTY_OUT="$BATS_TEST_TMPDIR/tty.out"
  OPT_YES=0
  ui_init
}

@test "--mcp is a component of its own, in no preset" {
  parse_args --mcp
  [ "$OPT_COMPONENTS" = mcp ]
  parse_args --everything
  [[ " $OPT_COMPONENTS " != *" mcp "* ]]
}

@test "from a release: unavailable until a release has mcp-connect" {
  run storm_main --yes --dry-run --version v0.6.0 --mcp
  [ "$status" -eq 1 ]
  [[ "$output" == *"MCP connector is unavailable: mcp-connect isn't in a Storm release yet"* ]]
  [[ "$output" != *"Put storm-runtime on PATH"* ]]
}

@test "from a release older than the first with mcp-connect: unavailable, with the version" {
  STORM_MCP_CONNECT_SINCE=0.7.0
  run storm_main --yes --dry-run --version v0.6.0 --mcp
  [ "$status" -eq 1 ]
  [[ "$output" == *"mcp-connect ships from v0.7.0; this release is v0.6.0"* ]]
}

@test "from a release that has it: the Linux binary, no apt" {
  STORM_MCP_CONNECT_SINCE=0.6.0
  run storm_main --dry-run --version v0.6.0 --mcp
  [ "$status" -eq 0 ]
  [[ "$output" == *"curl -fL -o storm-runtime-0.6.0-x86_64-linux https://github.com/dewanshDT/Storm/releases/download/v0.6.0/storm-runtime-0.6.0-x86_64-linux"* ]]
  [[ "$output" != *"apt"* ]]
}

@test "linux plan: download, copy onto PATH, check; no package, service or enrollment" {
  run storm_main --dry-run --from-dir "$ASSETS" --mcp
  [ "$status" -eq 0 ]
  descs=$(plan_descs "$output")
  [[ "$descs" == *"Download and verify storm-runtime-0.6.0-x86_64-linux"* ]]
  [[ "$descs" == *"Put storm-runtime on PATH at $(mcp_bin) (no host, no service)"* ]]
  [[ "$descs" == *"Check storm-runtime has mcp-connect"* ]]
  [[ "$output" == *"sudo cp storm-runtime-0.6.0-x86_64-linux $(mcp_bin)"* ]]
  [[ "$output" == *"Settings › Devices & access › New key"* ]]
  [[ "$output" != *"apt-get"* ]]
  [[ "$output" != *"systemctl"* ]]
  [[ "$output" != *"enroll"* ]]
  [ "$(plan_steps "$output" | wc -l)" -eq 3 ]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
  [ ! -s "$FAKE_LOG" ]
}

@test "linux run: the binary lands on PATH, and nothing else happens" {
  run storm_main --yes --from-dir "$ASSETS" --mcp
  [ "$status" -eq 0 ]
  [ -x "$(mcp_bin)" ]
  [ "$(file_mode "$(mcp_bin)")" = 755 ]
  ! grep -q 'apt-get' "$FAKE_LOG"
  ! grep -q 'systemctl' "$FAKE_LOG"
  ! grep -q 'storm-runtime enroll' "$FAKE_LOG"
  grep -q 'storm-runtime mcp-connect --help' "$FAKE_LOG"
  [ ! -e "$STORM_TEST_SYSROOT/var/lib/storm-runtime" ]
  [ ! -e "$FAKE_DIR/installed/storm-runtime" ]
  [[ "$output" == *"AI apps here can run $(mcp_bin) mcp-connect"* ]]
  marked=$(plan_steps "$output" | awk '$2=="sudo"{print $1}' | tr '\n' ' ')
  used=$(sudo_steps_used | tr '\n' ' ')
  [ "$marked" = "$used" ]
}

@test "a storm-runtime without mcp-connect fails the check and says so" {
  export FAKE_NO_MCP_CONNECT=1
  run storm_main --yes --from-dir "$ASSETS" --mcp
  [ "$status" -ne 0 ]
  grep -q 'this storm-runtime has no mcp-connect' "$HOME/.local/state/storm/install.log"
}

@test "macos plan: the runtime tarball, copied to /usr/local/bin; no LaunchDaemon" {
  use_macos
  run storm_main --dry-run --from-dir "$ASSETS" --mcp
  [ "$status" -eq 0 ]
  [[ "$output" == *"Download and verify storm-runtime-0.6.0-macos-universal.tar.gz"* ]]
  [[ "$output" == *"sudo cp storm-runtime $(mcp_bin)"* ]]
  [[ "$output" != *"storm-runtime install"* ]]
  [[ "$output" != *"_stormruntime"* ]]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
}

@test "macos run: on PATH, no daemon, no account" {
  use_macos
  run storm_main --yes --from-dir "$ASSETS" --mcp
  [ "$status" -eq 0 ]
  [ -x "$(mcp_bin)" ]
  [ ! -e "$STORM_TEST_SYSROOT/Library/StormRuntime" ]
  ! grep -q 'storm-runtime install' "$FAKE_LOG"
  ! grep -q 'launchctl' "$FAKE_LOG"
}

@test "macos with the runtime too: the tarball is fetched once" {
  use_macos
  run storm_main --dry-run --from-dir "$ASSETS" --runtime --mcp --enroll-from-stdin
  [ "$status" -eq 0 ]
  [ "$(printf '%s\n' "$output" | grep -c 'Download and verify storm-runtime-0.6.0-macos-universal.tar.gz')" -eq 1 ]
  [[ "$output" == *"Install the Runtime Host"* ]]
  [[ "$output" == *"Put storm-runtime on PATH"* ]]
}

@test "linux with the Runtime Host's package: already on PATH, nothing to do" {
  install_fake linux-runtime
  run storm_main --dry-run --from-dir "$ASSETS" --mcp
  [ "$status" -eq 0 ]
  [[ "$output" == *"the Runtime Host's storm-runtime is on PATH already"* ]]
  [[ "$output" != *"Put storm-runtime on PATH"* ]]
}

@test "the MCP copy on PATH is not mistaken for the Runtime Host" {
  platform
  mkdir -p "$(dirname "$(mcp_bin)")"
  cp "$FAKE_DIR/tools/storm-runtime" "$(mcp_bin)"
  PATH="$(dirname "$(mcp_bin)"):$PATH"
  runtime_detect
  [ "$RT_INSTALLED" = 0 ]
  mcp_detect
  [ "$MCP_INSTALLED" = 1 ]
  [ "$MCP_VERSION" = 0.6.0 ]
}

@test "a Homebrew link at /usr/local/bin is left alone" {
  use_macos
  mkdir -p "$(dirname "$(mcp_bin)")" "$BATS_TEST_TMPDIR/cellar"
  cp "$FAKE_DIR/tools/storm-runtime" "$BATS_TEST_TMPDIR/cellar/storm-runtime"
  ln -s "$BATS_TEST_TMPDIR/cellar/storm-runtime" "$(mcp_bin)"
  run storm_main --yes --from-dir "$ASSETS" --mcp
  [ "$status" -eq 0 ]
  [[ "$output" == *"is a link (Homebrew's storm-runtime?), so it is left alone"* ]]
  [ -L "$(mcp_bin)" ]
  [ "$(cat "$FAKE_DIR/tools/storm-runtime")" = "$(cat "$BATS_TEST_TMPDIR/cellar/storm-runtime")" ]
}

@test "status shows it; upgrade and uninstall include it" {
  mkdir -p "$(dirname "$(mcp_bin)")"
  cp "$FAKE_DIR/tools/storm-runtime" "$(mcp_bin)"
  run storm_main status
  [ "$status" -eq 0 ]
  [[ "$output" == *"MCP connector"*"installed v0.6.0"* ]]
  FAKE_RUNTIME_VERSION=0.5.0 run storm_main --dry-run --from-dir "$ASSETS" upgrade
  [ "$status" -eq 0 ]
  [[ "$output" == *"Put storm-runtime on PATH"* ]]
  run storm_main --yes --from-dir "$ASSETS" uninstall --mcp
  [ "$status" -eq 0 ]
  [ ! -e "$(mcp_bin)" ]
}

@test "the first screen offers MCP only" {
  typed "4" "3"
  run storm_main --from-dir "$ASSETS"
  [ "$status" -eq 0 ]
  out=$(cat "$STORM_TTY_OUT")
  [[ "$out" == *"MCP only"* ]]
  [[ "$out" == *"Put storm-runtime on PATH"* ]]
  [[ "$out" == *"Nothing was changed."* ]]
}

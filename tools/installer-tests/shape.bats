#!/usr/bin/env bats
# How a new Runtime Host runs (decision 151 E5): as you by default, the
# service one flag (--service) or one menu choice away, and a host already
# installed as the service keeps that shape on every re-run. On a Mac with
# the server, which runs as you, the host is the service, and --user is
# refused.

load helpers

setup() {
  setup_world
  make_assets 0.6.0
}

dry() {
  run storm_main --dry-run --from-dir "$ASSETS" --addr 10.0.0.5 "$@"
}

typed() {
  printf '%s\n' "$@" >"$BATS_TEST_TMPDIR/tty.in"
  export STORM_TTY="$BATS_TEST_TMPDIR/tty.in" STORM_TTY_OUT="$BATS_TEST_TMPDIR/tty.out"
}

@test "ubuntu: the default plan runs the host as you, with no sudo" {
  dry --runtime --agents --enroll-from-stdin
  [ "$status" -eq 0 ]
  [[ "$(plan_descs "$output")" == *"Install the Runtime Host as tester (a systemd --user unit; no root)"* ]]
  [[ "$output" == *"<the enrollment string> | $HOME/.local/bin/storm-runtime enroll"* ]]
  [[ "$output" == *"the default for a new host"*"re-run with --service"* ]]
  [[ "$output" != *"Install the storm-runtime package"* ]]
  [[ "$output" != *"sudo -u storm-runtime"* ]]
  [ -z "$(plan_steps "$output" | grep sudo)" ]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
}

@test "macos: the default plan is a LaunchAgent as you, with no sudo" {
  use_macos
  dry --runtime --enroll-from-stdin
  [ "$status" -eq 0 ]
  [[ "$(plan_descs "$output")" == *"Install the Runtime Host as a LaunchAgent running as tester"* ]]
  [[ "$output" == *"re-run with --service"* ]]
  [[ "$output" != *"LaunchDaemon"* ]]
  [ -z "$(plan_steps "$output" | grep sudo)" ]
}

@test "--yes takes the default: installed as you, enrolled, never sudo or apt" {
  run storm_main --yes --runtime --from-dir "$ASSETS" --enroll-from-stdin <<<"storm-enroll:v1:E5-DEFAULT"
  [ "$status" -eq 0 ]
  [ -f "$HOME/.config/systemd/user/storm-runtime.service" ]
  [ -f "$HOME/.local/state/storm-runtime/host.json" ]
  [ ! -e "$STORM_TEST_SYSROOT/lib/systemd/system/storm-runtime.service" ]
  grep -q 'storm-runtime install --user$' "$FAKE_LOG"
  ! grep -q ' sudo ' "$FAKE_LOG"
  ! grep -q 'apt-get' "$FAKE_LOG"
  [[ "$output" == *"The Runtime Host runs as tester (systemd --user storm-runtime)"* ]]
}

@test "macos --yes takes the default: a LaunchAgent, never sudo" {
  use_macos
  run storm_main --yes --runtime --from-dir "$ASSETS" --enroll-from-stdin <<<"storm-enroll:v1:E5-MAC"
  [ "$status" -eq 0 ]
  grep -q 'storm-runtime install --user$' "$FAKE_LOG"
  ! grep -q 'install --operator' "$FAKE_LOG"
  ! grep -q ' sudo ' "$FAKE_LOG"
  [ -f "$HOME/Library/LaunchAgents/dev.storm.runtime.plist" ]
  [ -f "$HOME/Library/Application Support/StormRuntime/state/host.json" ]
  [ ! -e "$STORM_TEST_SYSROOT/Library/StormRuntime/bin/storm-runtime" ]
}

@test "--service picks the service: the .deb and sudo -u storm-runtime" {
  dry --runtime --enroll-from-stdin --service
  [ "$status" -eq 0 ]
  [[ "$(plan_descs "$output")" == *"Install the storm-runtime package"* ]]
  [[ "$output" == *"| sudo -u storm-runtime storm-runtime enroll"* ]]
  [[ "$output" != *"install --user"* ]]
  [[ "$output" != *"the default for a new host"* ]]
  [ -n "$(plan_steps "$output" | grep sudo)" ]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
}

@test "macos --service picks the LaunchDaemon" {
  use_macos
  dry --runtime --enroll-from-stdin --service
  [ "$status" -eq 0 ]
  [[ "$(plan_descs "$output")" == *"Install the Runtime Host (a LaunchDaemon as _stormruntime"* ]]
  [[ "$output" == *"sudo ./storm-runtime install --operator tester"* ]]
}

@test "--user and --service together are refused" {
  run storm_main --dry-run --runtime --user --service
  [ "$status" -eq 2 ]
  [[ "$output" == *"--user and --service are the two ways to run the Runtime Host; pick one"* ]]
}

@test "an installed service host re-runs unchanged, without --service" {
  run storm_main --yes --runtime --service --from-dir "$ASSETS" --enroll-from-stdin <<<"storm-enroll:v1:svc"
  [ "$status" -eq 0 ]
  [ -f "$STORM_TEST_SYSROOT/lib/systemd/system/storm-runtime.service" ]
  platform
  [ "$RT_USER" = 0 ]
  [ "$RT_SHAPE_WHY" = installed ]
  : >"$FAKE_LOG"
  run storm_main --yes --runtime --from-dir "$ASSETS"
  [ "$status" -eq 0 ]
  [[ "$output" == *"runs it as its own account (storm-runtime); it stays that way"* ]]
  [[ "$output" != *"install --user"* ]]
  ! grep -q 'install --user' "$FAKE_LOG"
  ! grep -q 'apt-get install' "$FAKE_LOG"
  [ ! -e "$HOME/.config/systemd/user/storm-runtime.service" ]
  [ ! -e "$HOME/.local/bin/storm-runtime" ]
}

@test "an installed service host: upgrade and uninstall stay the service's" {
  install_fake linux-runtime
  export FAKE_RUNTIME_VERSION=0.5.0
  dry upgrade
  [ "$status" -eq 0 ]
  [[ "$output" == *"Install the storm-runtime package"* ]]
  [[ "$output" != *"install --user"* ]]
  run storm_main --dry-run --from-dir "$ASSETS" uninstall --runtime
  [ "$status" -eq 0 ]
  [[ "$output" == *"remove the storm-runtime package"* ]]
  [[ "$output" != *"uninstall --user"* ]]
}

@test "macos: an installed LaunchDaemon host re-runs unchanged" {
  use_macos
  install_fake macos-runtime
  platform
  [ "$RT_USER" = 0 ]
  [ "$RT_SHAPE_WHY" = installed ]
  run storm_main --yes --runtime --from-dir "$ASSETS"
  [ "$status" -eq 0 ]
  [[ "$output" == *"runs it as its own account (_stormruntime); it stays that way"* ]]
  ! grep -q 'storm-runtime install' "$FAKE_LOG"
  [ ! -e "$HOME/Library/Application Support/StormRuntime" ]
}

@test "a host installed as you stays as you" {
  use_macos
  install_fake macos-runtime-user
  platform
  [ "$RT_USER" = 1 ]
  [ "$RT_SHAPE_WHY" = installed ]
}

@test "both installed: the service is the one kept" {
  install_fake linux-runtime
  mkdir -p "$HOME/.config/systemd/user"
  : >"$HOME/.config/systemd/user/storm-runtime.service"
  platform
  [ "$RT_USER" = 0 ]
  [ "$RT_SHAPE_WHY" = installed ]
}

@test "as root on Debian there's no you: the service" {
  export FAKE_UID=0 FAKE_USER=root
  platform
  [ "$RT_USER" = 0 ]
  [ "$RT_SHAPE_WHY" = root ]
  OPT_USER=1
  platform
  [ "$RT_USER" = 1 ]
}

@test "--server --runtime: the server's package with sudo, the host as you" {
  os_release debian
  dry --server --runtime
  [ "$status" -eq 0 ]
  [[ "$output" == *"Install the storm-server package"* ]]
  [[ "$output" == *"Install the Runtime Host as tester (a systemd --user unit; no root)"* ]]
  [[ "$output" != *"Install the storm-runtime package"* ]]
  [[ "$output" != *"sudo -u storm-runtime"* ]]
  dry --server --runtime --service
  [[ "$output" == *"Install the storm-runtime package"* ]]
  [[ "$output" == *"sudo -u storm-runtime storm-runtime enroll"* ]]
}

@test "interactive: asks how the host runs, as you first; choosing the service plans it" {
  # Runtime only (3), As its own account (2), then Quit at the plan (3).
  typed "3" "2" "3"
  run storm_main --from-dir "$ASSETS"
  [ "$status" -eq 0 ]
  out=$(cat "$STORM_TTY_OUT")
  [[ "$out" == *"How should the Runtime Host run?"* ]]
  [[ "$out" == *" 1) As you (tester)"* ]]
  [[ "$out" == *" 2) As its own account"* ]]
  [[ "$out" == *"Install the storm-runtime package"* ]]
  [[ "$out" == *"Nothing was changed."* ]]
}

@test "interactive: Enter takes as you" {
  typed "3" "" "3"
  run storm_main --from-dir "$ASSETS"
  [ "$status" -eq 0 ]
  out=$(cat "$STORM_TTY_OUT")
  [[ "$out" == *"How should the Runtime Host run?"* ]]
  [[ "$out" == *"Install the Runtime Host as tester (a systemd --user unit; no root)"* ]]
  [[ "$out" != *"the default for a new host"* ]]
}

@test "interactive: no question where there's only one way, or a host already" {
  os_release arch
  typed "3" "3"
  run storm_main --from-dir "$ASSETS"
  [ "$status" -eq 0 ]
  [[ "$(cat "$STORM_TTY_OUT")" != *"How should the Runtime Host run?"* ]]
  os_release ubuntu
  install_fake linux-runtime
  typed "3" "" "3"
  run storm_main --from-dir "$ASSETS"
  [ "$status" -eq 0 ]
  [[ "$(cat "$STORM_TTY_OUT")" != *"How should the Runtime Host run?"* ]]
}

@test "as root, the plan says why the host is the service" {
  export FAKE_UID=0 FAKE_USER=root
  dry --runtime
  [ "$status" -eq 0 ]
  [[ "$output" == *"Install the storm-runtime package"* ]]
  [[ "$output" == *"the installer runs as root, so the host is the service (storm-runtime)"* ]]
}

@test "macos interactive: the service choice names the LaunchDaemon's account" {
  use_macos
  typed "3" "2" "3"
  run storm_main --from-dir "$ASSETS"
  [ "$status" -eq 0 ]
  out=$(cat "$STORM_TTY_OUT")
  [[ "$out" == *"As its own account  - _stormruntime, a LaunchDaemon"* ]]
  [[ "$out" == *"Install the Runtime Host (a LaunchDaemon as _stormruntime"* ]]
}

@test "macos --server --runtime: the host is the LaunchDaemon, since the server runs as you" {
  use_macos
  for preset in "--server --runtime" "--everything"; do
    # shellcheck disable=SC2086
    dry $preset --enroll-from-stdin
    [ "$status" -eq 0 ]
    [[ "$(plan_descs "$output")" == *"Install the Runtime Host (a LaunchDaemon as _stormruntime"* ]]
    [[ "$output" == *"the server runs as you on this Mac, so the host runs as its own account (_stormruntime)"* ]]
    [[ "$output" != *"install --user"* ]]
    [[ "$output" != *"re-run with --service"* ]]
  done
  platform
  [ "$RT_USER" = 1 ]
  OPT_COMPONENTS="server runtime" platform
  [ "$RT_USER" = 0 ]
  [ "$RT_SHAPE_WHY" = server ]
}

@test "macos --server --runtime --user is refused, and says why" {
  use_macos
  for preset in "--server --runtime" "--everything"; do
    # shellcheck disable=SC2086
    run storm_main --dry-run --from-dir "$ASSETS" $preset --user
    [ "$status" -eq 2 ]
    [[ "$output" == *"--user with --server on a Mac would run agents as the account that holds your notes"* ]]
  done
  dry --runtime --user --enroll-from-stdin
  [ "$status" -eq 0 ]
  dry --server --runtime --service --enroll-from-stdin
  [ "$status" -eq 0 ]
}

@test "linux --server --runtime --user stays allowed: the server is a system service" {
  os_release debian
  dry --server --runtime --user
  [ "$status" -eq 0 ]
  [[ "$output" == *"Install the Runtime Host as tester (a systemd --user unit; no root)"* ]]
}

@test "macos interactive: Everything asks nothing about the host and plans the LaunchDaemon" {
  use_macos
  typed "1" "" "" "" "" "" "" "" "3"
  run storm_main --from-dir "$ASSETS"
  [ "$status" -eq 0 ]
  out=$(cat "$STORM_TTY_OUT")
  [[ "$out" != *"How should the Runtime Host run?"* ]]
  [[ "$out" == *"Install the Runtime Host (a LaunchDaemon as _stormruntime"* ]]
  [[ "$out" == *"the server runs as you on this Mac"* ]]
}

@test "macos: a host installed as you is kept beside a new server, with a warning" {
  use_macos
  install_fake macos-runtime-user
  dry --server --runtime
  [ "$status" -eq 0 ]
  [[ "$output" == *"the server runs as you too, so its agents can read your vaults on disk"* ]]
  [[ "$output" != *"LaunchDaemon as _stormruntime"* ]]
}

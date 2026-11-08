#!/usr/bin/env bats
# The Runtime Host run as you (decision 151 E1): `--user`, or any Linux
# without the .deb. The tarball's `storm-runtime install --user`, enrollment
# as you, a systemd --user unit, and never sudo for the runtime.

load helpers

setup() {
  setup_world
  make_assets 0.6.0
}

dry() {
  run storm_main --dry-run --from-dir "$ASSETS" --addr 10.0.0.5 "$@"
}

user_paths_ok() {
  [ "$RT_USER" = 1 ]
  [ "$RT_ACCOUNT" = tester ]
  [ "$RT_BIN" = "$HOME/.local/bin/storm-runtime" ]
  [ "$RT_STATE" = "$HOME/.local/state/storm-runtime" ]
  [ "$RT_CONFIG" = "$HOME/.config/storm-runtime/runtime.toml" ]
  [ "$RT_UNIT" = "$HOME/.config/systemd/user/storm-runtime.service" ]
  [ "$RT_WORKSPACES" = "$HOME/.local/state/storm-runtime/workspaces" ]
  [ "$WS_SUDO" = 0 ]
}

@test "arch: no .deb, so the runtime runs as you" {
  os_release arch
  platform
  [ "$DEBIAN_LIKE" = 0 ]
  user_paths_ok
}

@test "debian: the service, unless --user or a user unit is installed" {
  os_release debian
  platform
  [ "$RT_USER" = 0 ]
  [ "$RT_ACCOUNT" = storm-runtime ]
  OPT_USER=1
  platform
  user_paths_ok
  OPT_USER=0
  mkdir -p "$HOME/.config/systemd/user"
  : >"$HOME/.config/systemd/user/storm-runtime.service"
  platform
  user_paths_ok
}

@test "XDG directories move the paths; a relative one is ignored" {
  os_release arch
  export XDG_STATE_HOME="$BATS_TEST_TMPDIR/st" XDG_CONFIG_HOME="$BATS_TEST_TMPDIR/cfg"
  platform
  [ "$RT_STATE" = "$BATS_TEST_TMPDIR/st/storm-runtime" ]
  [ "$RT_CONFIG" = "$BATS_TEST_TMPDIR/cfg/storm-runtime/runtime.toml" ]
  [ "$RT_UNIT" = "$BATS_TEST_TMPDIR/cfg/systemd/user/storm-runtime.service" ]
  export XDG_STATE_HOME=st XDG_CONFIG_HOME=cfg
  platform
  [ "$RT_STATE" = "$HOME/.local/state/storm-runtime" ]
  [ "$RT_UNIT" = "$HOME/.config/systemd/user/storm-runtime.service" ]
}

@test "the user path is unavailable as root, without systemd, or without the tarball" {
  os_release arch
  platform
  MY_UID=0
  runtime_available || :
  [[ "$REASON" == *"run the installer as yourself, not as root"* ]]
  platform
  rm "$FAKE_DIR/bin/systemctl"
  runtime_available || :
  [[ "$REASON" == *"needs systemd (systemctl --user)"* ]]
  ln -s "$FAKE_DIR/fake.sh" "$FAKE_DIR/bin/systemctl"
  make_assets 0.6.0 runtime-deb
  OPT_FROM_DIR=$ASSETS
  release_init
  runtime_available || :
  [[ "$REASON" == *"storm-runtime-0.6.0-x86_64-linux.tar.gz isn't in v0.6.0's checksums.txt"* ]]
}

@test "arch runtime: the tarball, install --user, enroll as you, no sudo" {
  os_release arch
  dry --runtime --enroll-from-stdin --workspace scratch
  [ "$status" -eq 0 ]
  descs=$(plan_descs "$output")
  [[ "$descs" == *"Download and verify storm-runtime-0.6.0-x86_64-linux.tar.gz"* ]]
  [[ "$descs" == *"Install the Runtime Host as tester (a systemd --user unit; no root)"* ]]
  [[ "$output" == *"$ ./storm-runtime install --user"* ]]
  [[ "$output" == *"<the enrollment string> | $HOME/.local/bin/storm-runtime enroll"* ]]
  [[ "$output" == *"$ systemctl --user enable --now storm-runtime"* ]]
  [[ "$output" == *"$ $HOME/.local/bin/storm-runtime check"* ]]
  [[ "$output" == *"$ mkdir $HOME/.local/state/storm-runtime/workspaces/scratch"* ]]
  [[ "$output" == *"loginctl enable-linger"* ]]
  [[ "$output" != *"Install the storm-runtime package"* ]]
  [ "$(plan_steps "$output" | awk '$2=="sudo"' | wc -l)" -eq 0 ]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
  [ ! -s "$FAKE_LOG" ]
}

@test "debian --user: the same plan, and the server keeps its sudo -u" {
  os_release debian
  dry --server --runtime --user
  [ "$status" -eq 0 ]
  [[ "$output" == *"Install the storm-server package"* ]]
  [[ "$output" != *"Install the storm-runtime package"* ]]
  [[ "$output" == *"Install the Runtime Host as tester (a systemd --user unit; no root)"* ]]
  [[ "$output" == *"host-enrollment --state $STORM_TEST_SYSROOT/srv/storm/state --url http://127.0.0.1:8484 | $HOME/.local/bin/storm-runtime enroll"* ]]
  [[ "$output" != *"sudo -u storm-runtime"* ]]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
}

@test "arch --yes: installs as you and enrolls from stdin without sudo -u" {
  os_release arch
  run storm_main --yes --runtime --from-dir "$ASSETS" --enroll-from-stdin --workspace scratch <<<"storm-enroll:v1:USERSECRET-1"
  [ "$status" -eq 0 ]
  [ -x "$HOME/.local/bin/storm-runtime" ]
  [ -f "$HOME/.config/systemd/user/storm-runtime.service" ]
  [ -f "$HOME/.local/state/storm-runtime/host.json" ]
  [ -d "$HOME/.local/state/storm-runtime/workspaces/scratch" ]
  [ ! -e "$STORM_TEST_SYSROOT/var/lib/storm-runtime" ]
  grep -q 'storm-runtime install --user$' "$FAKE_LOG"
  grep -q 'storm-runtime enroll$' "$FAKE_LOG"
  grep -q 'systemctl --user enable --now storm-runtime' "$FAKE_LOG"
  grep -q 'storm-runtime check' "$FAKE_LOG"
  grep -q 'storm-runtime enroll: storm-enroll:v1:USERSECRET-1' "$FAKE_STDIN"
  ! grep -q 'USERSECRET' "$FAKE_LOG"
  ! grep -q ' sudo ' "$FAKE_LOG"
  ! grep -q 'apt-get' "$FAKE_LOG"
  [ -z "$(sudo_steps_used)" ]
  [[ "$output" == *"The Runtime Host runs as tester (systemd --user storm-runtime)"* ]]
  [[ "$output" == *"sudo loginctl enable-linger tester"* ]]
}

@test "the local enrollment pipes the server's string into enroll as you" {
  os_release debian
  install_fake linux-server
  touch "$FAKE_DIR/account"
  run storm_main --yes --runtime --user --from-dir "$ASSETS"
  [ "$status" -eq 0 ]
  grep -q 'storm-runtime enroll: storm-enroll:v1:LOCALSECRET-fromhost-0001' "$FAKE_STDIN"
  [ -f "$HOME/.local/state/storm-runtime/host.json" ]
  ! grep -q 'sudo .*-u tester' "$FAKE_LOG"
  ! grep -q 'sudo .*storm-runtime enroll' "$FAKE_LOG"
}

@test "status, then uninstall, as you: the unit and binary go, state stays" {
  os_release arch
  run storm_main --yes --runtime --from-dir "$ASSETS" --enroll-from-stdin <<<"storm-enroll:v1:abc"
  [ "$status" -eq 0 ]
  run storm_main --from-dir "$ASSETS" status
  [ "$status" -eq 0 ]
  [[ "$output" == *"Runtime Host"*"installed v0.6.0"*"enrolled: yes"*"running: yes"* ]]
  : >"$FAKE_LOG"
  run storm_main --yes uninstall --runtime
  [ "$status" -eq 0 ]
  grep -q 'storm-runtime uninstall --user$' "$FAKE_LOG"
  ! grep -q 'sudo\|apt-get' "$FAKE_LOG"
  [ ! -e "$HOME/.config/systemd/user/storm-runtime.service" ]
  [ ! -e "$HOME/.local/bin/storm-runtime" ]
  [ -f "$HOME/.local/state/storm-runtime/host.json" ]
  [[ "$output" == *"The runtime's state and config: $HOME/.local/state/storm-runtime, $HOME/.config/storm-runtime/runtime.toml"* ]]
  [[ "$output" != *"apt source"* ]]
}

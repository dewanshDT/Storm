#!/usr/bin/env bats
# The plan screen: what each preset plans on Linux and macOS, which steps
# are marked sudo, and that --dry-run executes nothing.

load helpers

setup() {
  setup_world
  make_assets 0.6.0
}

dry() {
  run storm_main --dry-run --from-dir "$ASSETS" --addr 10.0.0.5 "$@"
}

@test "linux everything: every step, the sudo ones marked, nothing executed" {
  dry --everything
  [ "$status" -eq 0 ]
  [[ "$output" == *"Dry run: nothing was changed."* ]]
  descs=$(plan_descs "$output")
  [[ "$descs" == *"Install the storm-server package"* ]]
  [[ "$descs" == *"Set up and start the server (systemd"* ]]
  [[ "$descs" == *"Check the server answers on port 8484"* ]]
  [[ "$descs" == *"Set up your account"* ]]
  [[ "$descs" == *"Install the storm-runtime package"* ]]
  [[ "$descs" == *"Enroll this runtime with the server on this machine"* ]]
  [[ "$descs" == *"Start the Runtime Host"* ]]
  [[ "$descs" == *"Check the runtime reaches its server"* ]]
  [[ "$descs" == *"Show the web client's address"* ]]
  [[ "$descs" == *"Show how to connect your phone or tablet"* ]]
  [[ "$output" == *"sudo storm-server up --data-root $STORM_TEST_SYSROOT/srv/storm --port 8484 --host 0.0.0.0"* ]]
  [[ "$output" == *"apt-get install -y ./storm-server_0.6.0-1_amd64.deb"* ]]
  [[ "$output" == *"host-enrollment --state $STORM_TEST_SYSROOT/srv/storm/state --url http://127.0.0.1:8484 | sudo -u storm-runtime storm-runtime enroll"* ]]
  [[ "$output" == *"storm-server qr http://10.0.0.5:8484"* ]]
  [[ "$output" == *"storm-server qr https://github.com/dewanshDT/Storm/releases/download/v0.6.0/storm-0.6.0.apk"* ]]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
  [ ! -s "$FAKE_LOG" ]
}

@test "linux everything from the apt repo plans the repository once" {
  run storm_main --dry-run --version v0.6.0 --addr 10.0.0.5 --everything
  [ "$status" -eq 0 ]
  [ "$(printf '%s\n' "$output" | grep -c "Add Storm's apt repository")" -eq 1 ]
  [[ "$output" == *"sudo apt-get install -y storm-server"* ]]
  [[ "$output" == *"sudo apt-get install -y storm-runtime"* ]]
  [[ "$output" == *"/storm-archive-keyring.gpg | sudo tee $STORM_TEST_SYSROOT/usr/share/keyrings/storm.gpg"* ]]
  [[ "$output" == *"deb [signed-by=/usr/share/keyrings/storm.gpg] https://dewanshdt.github.io/Storm stable main"* ]]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
}

@test "linux server and app: no runtime steps" {
  dry --server --app --mobile
  [ "$status" -eq 0 ]
  [[ "$output" == *"Install the storm-server package"* ]]
  [[ "$output" != *"storm-runtime"* ]]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
  [ ! -s "$FAKE_LOG" ]
}

@test "linux runtime only: paste enrollment, agents need a terminal" {
  dry --runtime --agents --enroll-from-stdin
  [ "$status" -eq 0 ]
  [[ "$output" == *"Install the storm-runtime package"* ]]
  [[ "$output" == *"Enroll this runtime with your Storm server (paste its enrollment string)"* ]]
  [[ "$output" == *"<the enrollment string> | sudo -u storm-runtime storm-runtime enroll"* ]]
  [[ "$output" != *"storm-server"* ]]
  [[ "$output" == *"Agent logins need a terminal"* ]]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
  [ ! -s "$FAKE_LOG" ]
}

@test "linux runtime only without --enroll-from-stdin says how to enroll" {
  dry --runtime
  [ "$status" -eq 0 ]
  [[ "$output" != *"Enroll this runtime"* ]]
  [[ "$output" == *"Pass --enroll-from-stdin"* ]]
}

@test "macos everything: the server is never sudo; the runtime install is" {
  use_macos
  dry --everything
  [ "$status" -eq 0 ]
  descs=$(plan_descs "$output")
  [[ "$descs" == *"Download and verify storm-server-0.6.0-macos-universal.tar.gz"* ]]
  [[ "$descs" == *"Install the server as a LaunchAgent running as"* ]]
  [[ "$descs" == *"Download and verify storm-runtime-0.6.0-macos-universal.tar.gz"* ]]
  [[ "$descs" == *"Install the Runtime Host (a LaunchDaemon as _stormruntime"* ]]
  [[ "$descs" == *"Enroll this runtime with the server on this Mac"* ]]
  [[ "$descs" == *"Install Claude Code with Homebrew"* ]]
  [[ "$descs" == *"Install OpenCode with Homebrew"* ]]
  [[ "$descs" == *"Download and verify Storm-0.6.0-macos-arm64.zip"* ]]
  [[ "$descs" == *"Install Storm.app into $STORM_TEST_SYSROOT/Applications"* ]]
  [[ "$output" == *"./storm-server up --data-root $HOME/Storm --port 8484 --host 0.0.0.0"* ]]
  [[ "$output" == *"sudo ./storm-runtime install --operator $(id -un)"* ]]
  [[ "$output" == *"| (cd / && sudo -u _stormruntime $STORM_TEST_SYSROOT/Library/StormRuntime/bin/storm-runtime enroll)"* ]]
  [[ "$output" == *"brew install --cask claude-code"* ]]
  [[ "$descs" != *"apt"* ]]
  # Every server step runs as the user.
  steps=$(plan_steps "$output")
  while read -r n kind; do
    d=$(plan_descs "$output" | sed -n "${n}p")
    case "$d" in
      *storm-server* | *LaunchAgent* | *"Check the server"* | *"Set up your account"*) [ "$kind" = user ] ;;
    esac
  done <<<"$steps"
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
  [ ! -s "$FAKE_LOG" ]
}

@test "macos server and app" {
  use_macos
  dry --server --app --mobile
  [ "$status" -eq 0 ]
  [[ "$output" == *"Install the server as a LaunchAgent"* ]]
  [[ "$output" == *"Install Storm.app"* ]]
  [[ "$output" == *"0 with sudo"* ]]
  [ -z "$(plan_steps "$output" | grep sudo)" ]
  [ ! -s "$FAKE_LOG" ]
}

@test "macos runtime only" {
  use_macos
  dry --runtime --agents --enroll-from-stdin
  [ "$status" -eq 0 ]
  [[ "$output" == *"Install the Runtime Host"* ]]
  [[ "$output" == *"paste its enrollment string"* ]]
  [[ "$output" != *"LaunchAgent"* ]]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
}

@test "macos on a release without the server tarball: unavailable, with the reason" {
  use_macos
  make_assets 0.5.0 runtime-tar app-zip apk
  run storm_main --dry-run --from-dir "$ASSETS" --addr 10.0.0.5 --everything --enroll-from-stdin
  [ "$status" -eq 0 ]
  [[ "$output" == *"Storm server is unavailable: the macOS server ships from v0.6.0; this release is v0.5.0"* ]]
  [[ "$output" != *"LaunchAgent"* ]]
  [[ "$output" == *"Install the Runtime Host"* ]]
  [[ "$output" == *"paste its enrollment string"* ]]
}

@test "macos on Intel: the app is unavailable" {
  use_macos x86_64
  dry --server --app
  [ "$status" -eq 0 ]
  [[ "$output" == *"Desktop app is unavailable: Storm.app is built for Apple silicon only"* ]]
  [[ "$output" != *"Storm.app into"* ]]
}

@test "an installed server is kept: no install steps, only the health check" {
  install_fake linux-server
  touch "$FAKE_DIR/account"
  run storm_main --yes --dry-run --from-dir "$ASSETS" --addr 10.0.0.5 --server
  [ "$status" -eq 0 ]
  [[ "$output" != *"Install the storm-server package"* ]]
  [[ "$output" != *"storm-server up"* ]]
  [[ "$output" == *"Check the server answers"* ]]
  [ ! -s "$FAKE_LOG" ]
}

@test "upgrade plans the package and a restart" {
  install_fake linux-server
  install_fake linux-runtime
  run storm_main --dry-run --from-dir "$ASSETS" --addr 10.0.0.5 upgrade
  [ "$status" -eq 0 ]
  [[ "$output" == *"Install the storm-server package"* ]]
  [[ "$output" == *"Restart the server on the new files"* ]]
  [[ "$output" == *"Install the storm-runtime package"* ]]
  [[ "$output" != *"storm-server up"* ]]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
}

@test "uninstall plans removal and keeps the data" {
  install_fake linux-server
  install_fake linux-runtime
  run storm_main --dry-run --from-dir "$ASSETS" uninstall --everything
  [ "$status" -eq 0 ]
  [[ "$output" == *"remove the storm-server package (data in $STORM_TEST_SYSROOT/srv/storm stays)"* ]]
  [[ "$output" == *"remove the storm-runtime package (workspaces stay)"* ]]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
  [ ! -s "$FAKE_LOG" ]
}

@test "macos uninstall: the server's own uninstall, as the user" {
  use_macos
  install_fake macos-server
  install_fake macos-runtime
  install_fake macos-app
  run storm_main --dry-run --from-dir "$ASSETS" uninstall --server --runtime --app
  [ "$status" -eq 0 ]
  [[ "$output" == *"storm-server' uninstall"* ]]
  [[ "$output" == *"sudo $STORM_TEST_SYSROOT/Library/StormRuntime/bin/storm-runtime uninstall"* ]]
  [[ "$output" == *"Remove Storm.app"* ]]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
}

@test "--data and --port reach every command" {
  dry --server --data /data/storm --port 9000 --addr 10.0.0.5
  [ "$status" -eq 0 ]
  [[ "$output" == *"--data-root /data/storm --port 9000"* ]]
  [[ "$output" == *"curl -fsS http://127.0.0.1:9000/v1/health"* ]]
  [[ "$output" == *"--state /data/storm/state --addr 10.0.0.5:9000 --qr"* ]]
}

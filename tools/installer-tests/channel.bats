#!/usr/bin/env bats
# Decision 163: stable (main, v0.18.x) vs latest (staging, v0.19.x) channels.
# apt's suite, the release GitHub resolves with no --version, and an
# `upgrade` with no --channel staying on whichever one is installed.

load helpers

setup() {
  setup_world
}

@test "--channel defaults to stable: the apt line is unchanged" {
  make_assets 0.6.0 server-deb
  run storm_main --yes --server --version v0.6.0 --addr 10.0.0.5 --password-stdin <<<"pw"
  [ "$status" -eq 0 ]
  [ "$(cat "$STORM_TEST_SYSROOT/etc/apt/sources.list.d/storm.list")" = \
    "deb [signed-by=/usr/share/keyrings/storm.gpg] https://dewanshdt.github.io/Storm stable main" ]
}

@test "--channel latest writes the apt line with suite latest" {
  make_assets 0.19.0 server-deb
  run storm_main --yes --server --channel latest --version v0.19.0 --addr 10.0.0.5 --password-stdin <<<"pw"
  [ "$status" -eq 0 ]
  [ "$(cat "$STORM_TEST_SYSROOT/etc/apt/sources.list.d/storm.list")" = \
    "deb [signed-by=/usr/share/keyrings/storm.gpg] https://dewanshdt.github.io/Storm latest main" ]
}

@test "STORM_CHANNEL=latest writes the same apt line as --channel latest" {
  make_assets 0.19.0 server-deb
  STORM_CHANNEL=latest run storm_main --yes --server --version v0.19.0 --addr 10.0.0.5 --password-stdin <<<"pw"
  [ "$status" -eq 0 ]
  [[ "$(cat "$STORM_TEST_SYSROOT/etc/apt/sources.list.d/storm.list")" == *" latest main" ]]
}

@test "release resolution: latest picks the newest semver, a pre-release included" {
  platform
  CHANNEL=latest
  export FAKE_RELEASES_LIST="$(fake_releases_list v0.18.1:false v0.19.0:true)"
  [ "$(release_latest)" = 0.19.0 ]
}

@test "release resolution: order in the list doesn't matter" {
  platform
  CHANNEL=latest
  export FAKE_RELEASES_LIST="$(fake_releases_list v0.19.0:true v0.18.1:false v0.9.9:false)"
  [ "$(release_latest)" = 0.19.0 ]
}

@test "release resolution: stable still uses releases/latest, not the list" {
  platform
  export FAKE_RELEASES_LIST="$(fake_releases_list v0.18.1:false v0.19.0:true)"
  export FAKE_LATEST=0.18.1
  [ "$(release_latest)" = 0.18.1 ]
}

@test "release resolution: a latest-channel fetch failure fails like stable's does" {
  platform
  CHANNEL=latest
  curl() { return 6; }
  run release_latest
  [ "$status" -eq 1 ]
}

@test "release resolution: an empty or malformed list also fails cleanly" {
  platform
  CHANNEL=latest
  export FAKE_RELEASES_LIST='[]'
  run release_latest
  [ "$status" -eq 1 ]
}

@test "--version still overrides channel resolution" {
  make_assets 0.5.5 server-deb
  run storm_main --yes --server --channel latest --version v0.5.5 --addr 10.0.0.5 --password-stdin <<<"pw"
  [ "$status" -eq 0 ]
  grep -q 'releases/download/v0.5.5/checksums.txt' "$FAKE_LOG"
}

@test "upgrade with no --channel stays on the installed latest channel" {
  make_assets 0.19.0 server-deb
  run storm_main --yes --server --channel latest --version v0.19.0 --addr 10.0.0.5 --password-stdin <<<"pw"
  [ "$status" -eq 0 ]
  export FAKE_SERVER_VERSION=0.19.0
  make_assets 0.19.1 server-deb
  run storm_main upgrade --yes --server --version v0.19.1
  [ "$status" -eq 0 ]
  [ "$(cat "$STORM_TEST_SYSROOT/etc/apt/sources.list.d/storm.list")" = \
    "deb [signed-by=/usr/share/keyrings/storm.gpg] https://dewanshdt.github.io/Storm latest main" ]
}

@test "upgrade with an explicit --channel switches, even off the installed one" {
  make_assets 0.19.0 server-deb
  run storm_main --yes --server --channel latest --version v0.19.0 --addr 10.0.0.5 --password-stdin <<<"pw"
  [ "$status" -eq 0 ]
  export FAKE_SERVER_VERSION=0.19.0
  make_assets 0.18.5 server-deb
  run storm_main upgrade --yes --server --channel stable --version v0.18.5
  [ "$status" -eq 0 ]
  [ "$(cat "$STORM_TEST_SYSROOT/etc/apt/sources.list.d/storm.list")" = \
    "deb [signed-by=/usr/share/keyrings/storm.gpg] https://dewanshdt.github.io/Storm stable main" ]
}

@test "detect_installed_channel: the apt line wins on Linux, empty when there's none" {
  platform
  run detect_installed_channel
  [ "$status" -eq 1 ]
  mkdir -p "$(dirname "$LIST_PATH")"
  printf 'deb [signed-by=/usr/share/keyrings/storm.gpg] https://dewanshdt.github.io/Storm latest main\n' >"$LIST_PATH"
  [ "$(detect_installed_channel)" = latest ]
}

@test "macos: no apt, so upgrade with no --channel reads the marker install.sh wrote" {
  use_macos
  make_assets 0.19.0 runtime-tar
  run storm_main --yes --runtime --channel latest --version v0.19.0 --enroll-from-stdin <<<"storm-enroll:v1:abc"
  [ "$status" -eq 0 ]
  [ "$(cat "$HOME/Library/Logs/Storm/channel")" = latest ]
  export FAKE_RUNTIME_VERSION=0.19.0
  make_assets 0.19.1 runtime-tar
  run storm_main upgrade --yes --runtime --version v0.19.1
  [ "$status" -eq 0 ]
  [ "$(cat "$HOME/Library/Logs/Storm/channel")" = latest ]
}

@test "--dry-run never writes the channel marker" {
  use_macos
  make_assets 0.19.0 runtime-tar
  run storm_main --dry-run --runtime --channel latest --from-dir "$ASSETS" --enroll-from-stdin <<<"storm-enroll:v1:abc"
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/Library/Logs/Storm/channel" ]
}

@test "versions: ver_lt, normalize_version and version_from_dir with plain semver on both lines" {
  ver_lt 0.18.9 0.19.0
  ver_lt 0.18.0 0.18.1
  ! ver_lt 0.19.0 0.18.9
  [ "$(normalize_version v0.19.0)" = 0.19.0 ]
  [ "$(normalize_version v0.18.9)" = 0.18.9 ]
  make_assets 0.19.0 server-deb
  [ "$(version_from_dir "$ASSETS")" = 0.19.0 ]
}

#!/usr/bin/env bats
# Argument parsing.

load helpers

setup() {
  setup_world
}

@test "defaults" {
  parse_args
  [ "$OPT_YES" = 0 ]
  [ "$OPT_DRY_RUN" = 0 ]
  [ "$SUBCOMMAND" = install ]
  [ -z "$OPT_COMPONENTS" ]
  [ -z "$OPT_VERSION" ]
}

@test "flags and a pinned version" {
  parse_args --yes --dry-run --version v1.2.3 --server --runtime
  [ "$OPT_YES" = 1 ]
  [ "$OPT_DRY_RUN" = 1 ]
  [ "$OPT_VERSION" = 1.2.3 ]
  [ "$OPT_COMPONENTS" = "server runtime" ]
}

@test "--opt=value forms" {
  parse_args --version=0.6.0 --port=9000 --data=/srv/x --addr=10.1.2.3:9000
  [ "$OPT_VERSION" = 0.6.0 ]
  [ "$OPT_PORT" = 9000 ]
  [ "$OPT_DATA" = /srv/x ]
  [ "$OPT_ADDR" = 10.1.2.3:9000 ]
}

@test "--everything expands and de-duplicates" {
  parse_args --server --everything --app
  [ "$OPT_COMPONENTS" = "server runtime agents app mobile" ]
}

@test "--data expands ~ and makes relative paths absolute" {
  parse_args --data "~/notes"
  [ "$OPT_DATA" = "$HOME/notes" ]
  cd "$BATS_TEST_TMPDIR"
  parse_args --data rel/dir/
  [ "$OPT_DATA" = "$BATS_TEST_TMPDIR/rel/dir" ]
}

@test "bad values are refused with exit 2" {
  run parse_args --version banana
  [ "$status" -eq 2 ]
  [[ "$output" == *"--version wants a release like v0.6.0"* ]]
  run parse_args --port 0
  [ "$status" -eq 2 ]
  run parse_args --port 70000
  [ "$status" -eq 2 ]
  run parse_args --port 80a
  [ "$status" -eq 2 ]
  run parse_args --addr "a b"
  [ "$status" -eq 2 ]
  run parse_args --from-dir /nonexistent/dir
  [ "$status" -eq 2 ]
  run parse_args --data
  [ "$status" -eq 2 ]
  [[ "$output" == *"--data needs a value"* ]]
}

@test "a port with a leading zero is still decimal" {
  parse_args --port 08080
  [ "$OPT_PORT" = 8080 ]
}

@test "subcommands" {
  parse_args status
  [ "$SUBCOMMAND" = status ]
  parse_args --yes upgrade
  [ "$SUBCOMMAND" = upgrade ]
  parse_args uninstall --runtime
  [ "$SUBCOMMAND" = uninstall ]
  [ "$OPT_COMPONENTS" = runtime ]
}

@test "unknown options and the relay are refused" {
  run parse_args --frobnicate
  [ "$status" -eq 2 ]
  [[ "$output" == *"unknown option or command: --frobnicate"* ]]
  run parse_args --relay
  [ "$status" -eq 2 ]
  [[ "$output" == *"coming soon"* ]]
}

@test "two readers of stdin are refused" {
  run parse_args --enroll-from-stdin --password-stdin
  [ "$status" -eq 2 ]
}

@test "--from-dir is made absolute" {
  mkdir -p "$BATS_TEST_TMPDIR/rel"
  cd "$BATS_TEST_TMPDIR"
  parse_args --from-dir rel
  [ "$OPT_FROM_DIR" = "$BATS_TEST_TMPDIR/rel" ]
}

@test "--help prints usage and exits 0 without touching anything" {
  run storm_main --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"--enroll-from-stdin"* ]]
  [[ "$output" == *"--from-dir DIR"* ]]
  [ ! -s "$FAKE_LOG" ]
}

@test "a bad flag exits 2 from storm_main" {
  run storm_main --nope
  [ "$status" -eq 2 ]
}

@test "presets" {
  [ "$(preset_components everything)" = "server runtime agents app mobile" ]
  [ "$(preset_components server-app)" = "server app mobile" ]
  [ "$(preset_components runtime)" = "runtime agents" ]
  run preset_components nonsense
  [ "$status" -eq 1 ]
}

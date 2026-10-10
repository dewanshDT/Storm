#!/usr/bin/env bats
# The TUI's line mode (the raw-key mode needs a real terminal; CI drives it
# through a pty), the no-terminal refusal, colour fallbacks, and the sh entry.

load helpers

setup() {
  setup_world
}

typed() {
  printf '%s\n' "$@" >"$BATS_TEST_TMPDIR/tty.in"
  export STORM_TTY="$BATS_TEST_TMPDIR/tty.in" STORM_TTY_OUT="$BATS_TEST_TMPDIR/tty.out"
  OPT_YES=0
  ui_init
}

@test "no terminal and no --yes: refuse, explain the flags, exit 2, run nothing" {
  run storm_main --server
  [ "$status" -eq 2 ]
  [[ "$output" == *"there's no terminal to ask questions on, and --yes wasn't given"* ]]
  [[ "$output" == *"--yes --everything"* ]]
  [[ "$output" == *"--dry-run --everything"* ]]
  [ ! -s "$FAKE_LOG" ]
  [ ! -e "$HOME/.local/state/storm/install.log" ]
}

@test "no terminal, --yes, nothing chosen: says what to pass" {
  run storm_main --yes
  [ "$status" -eq 2 ]
  [[ "$output" == *"nothing to install: pass --everything"* ]]
}

@test "with a terminal, there is one: interactive line mode" {
  typed ""
  [ "$INTERACTIVE" = 1 ]
  [ "$RAW_KEYS" = 0 ]
}

@test "--yes is never interactive, even with a terminal" {
  printf '\n' >"$BATS_TEST_TMPDIR/tty.in"
  export STORM_TTY="$BATS_TEST_TMPDIR/tty.in"
  OPT_YES=1
  ui_init
  [ "$INTERACTIVE" = 0 ]
}

@test "ui_menu, numbered: separators aren't numbered; bad input is re-asked" {
  typed "9" "x" "4"
  ui_menu "Pick" "One|first" "Two" "-" "Three" "Four|last"
  [ "$UI_INDEX" = 5 ]
  grep -q ' 4) Four  - last' "$STORM_TTY_OUT"
  grep -q 'Type a number from the list' "$STORM_TTY_OUT"
}

@test "ui_menu: q quits" {
  typed "q"
  run ui_menu "Pick" "One" "Two"
  [ "$status" -eq 1 ]
}

@test "ui_menu without a terminal takes the first item" {
  ui_menu "Pick" "-" "One" "Two"
  [ "$UI_INDEX" = 2 ]
}

@test "ui_checklist, numbered: toggles, disabled items stay off and say why" {
  typed "2 3" ""
  ui_checklist "Pick" "a|Alpha|on|x" "b|Beta|off|y" "c|Gamma|disabled|coming soon"
  [ "$UI_RESULT" = "a b" ]
  grep -q 'Gamma  (unavailable: coming soon)' "$STORM_TTY_OUT"
}

@test "ui_input keeps the default on Enter" {
  typed "" "/data"
  ui_input "Data folder" "/srv/storm"
  [ "$UI_RESULT" = /srv/storm ]
  ui_input "Data folder" "/srv/storm"
  [ "$UI_RESULT" = /data ]
}

@test "ui_confirm: default, yes, no" {
  typed "" "y" "no"
  ! ui_confirm "Sure?" n
  ui_confirm "Sure?" n
  ! ui_confirm "Sure?" y
}

@test "interactive custom flow in line mode: checklist, questions, plan, quit" {
  make_assets 0.6.0
  # Custom… (4), keep server only (toggle off app 4 and mobile 5? none are on),
  # tick 1 (server), Enter; data, port, address defaults; then Quit (3).
  typed "4" "1" "" "" "" "" "3"
  run storm_main --from-dir "$ASSETS"
  [ "$status" -eq 0 ]
  out=$(cat "$STORM_TTY_OUT")
  [[ "$out" == *"Remote access"*"unavailable: coming soon"* ]]
  [[ "$out" == *"Install the storm-server package"* ]]
  [[ "$out" == *"Nothing was changed."* ]]
  ! grep -q 'apt-get' "$FAKE_LOG"
}

@test "NO_COLOR and TERM=dumb: no escape codes" {
  NO_COLOR=1 ui_style
  [ -z "$C_BOLD$C_RED$C_RESET" ]
  unset NO_COLOR
  TERM=dumb ui_style
  [ -z "$C_BOLD$C_RED$C_RESET" ]
}

@test "without UTF-8, plain glyphs" {
  LANG=C LC_ALL="" LC_CTYPE="" ui_style
  [ "$G_OK" = ok ]
  LANG=en_US.UTF-8 ui_style
  [ "$G_OK" = "✓" ]
}

@test "sh entry: dash hands the script to bash and cleans up after itself" {
  command -v dash >/dev/null || skip "no dash here"
  tmp="$BATS_TEST_TMPDIR/tmp"
  mkdir -p "$tmp"
  run env -u STORM_INSTALL_TEST TMPDIR="$tmp" dash "$INSTALLER" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"Storm installer"* ]]
  [ -z "$(ls -A "$tmp")" ]
}

@test "sh entry: piped into sh, like curl | sh" {
  run bash -c "env -u STORM_INSTALL_TEST sh -s -- --nope < '$INSTALLER'"
  [ "$status" -eq 2 ]
  [[ "$output" == *"unknown option or command: --nope"* ]]
}

@test "a truncated download runs nothing" {
  total=$(wc -l <"$INSTALLER")
  for n in 30 500 1500 $((total - 3)) $((total - 1)); do
    head -n "$n" "$INSTALLER" >"$BATS_TEST_TMPDIR/cut.sh"
    run env -u STORM_INSTALL_TEST sh "$BATS_TEST_TMPDIR/cut.sh" --help
    [[ "$output" != *"Usage:"* ]] || { echo "ran after $n lines"; false; }
    run env -u STORM_INSTALL_TEST bash "$BATS_TEST_TMPDIR/cut.sh" --help
    [[ "$output" != *"Usage:"* ]] || { echo "bash ran after $n lines"; false; }
  done
}

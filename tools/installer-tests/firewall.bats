#!/usr/bin/env bats
# Decision 119, Layer 1: the LAN-only firewall rule. Asked first; ufw gets a
# rule from each local subnet, firewalld the service in home/trusted zones
# only; uninstall removes exactly what was added; without administrator
# rights it prints the command and never fails the install.

load helpers

setup() {
  setup_world
  make_assets 0.6.0
}

install_server() {
  run storm_main --yes --server --from-dir "$ASSETS" --addr 10.0.0.5 --password-stdin "$@" <<<"pw"
}

record() { cat "$STORM_TEST_SYSROOT/etc/storm/lan-firewall"; }

@test "no ufw and no firewalld: no firewall step, nothing asked" {
  install_server
  [ "$status" -eq 0 ]
  [[ "$output" != *"local network only"* ]]
  ! grep -q 'ufw\|firewall-cmd' "$FAKE_LOG"
  [ ! -e "$STORM_TEST_SYSROOT/etc/storm/lan-firewall" ]
}

@test "ufw: the plan says the exact rules, one per local subnet, never anywhere" {
  use_ufw
  run storm_main --dry-run --server --from-dir "$ASSETS" --addr 10.0.0.5
  [ "$status" -eq 0 ]
  [[ "$output" == *"Allow Storm from your local network only (ufw, from 192.168.1.0/24, fd12:3456:789a:1::/64 only)"* ]]
  [[ "$output" == *"\$ sudo ufw allow from 192.168.1.0/24 to any app storm-server"* ]]
  [[ "$output" == *"\$ sudo ufw allow from fd12:3456:789a:1::/64 to any app storm-server"* ]]
  [[ "$output" != *"2001:db8"* ]]
  [[ "$output" != *"fe80::"* ]]
  run assert_sudo_marks_consistent "$output"
  [ "$status" -eq 0 ]
  [ ! -s "$FAKE_LOG" ]
}

@test "ufw, consent by default: the rules are added and recorded, the others kept" {
  use_ufw
  echo "22/tcp ALLOW Anywhere" >"$FAKE_DIR/ufw.rules"
  install_server
  [ "$status" -eq 0 ]
  grep -q 'ufw allow from 192.168.1.0/24 to any app storm-server$' "$FAKE_LOG"
  grep -q 'ufw allow from fd12:3456:789a:1::/64 to any app storm-server$' "$FAKE_LOG"
  ! grep -q 'ufw allow .*2001:db8' "$FAKE_LOG"
  ! grep -Eq 'ufw (allow|delete) [0-9]+(/tcp)?$' "$FAKE_LOG"
  [ "$(record)" = "ufw 192.168.1.0/24
ufw fd12:3456:789a:1::/64" ]
  [ "$(cat "$FAKE_DIR/ufw.rules")" = "22/tcp ALLOW Anywhere
storm-server ALLOW 192.168.1.0/24
storm-server ALLOW fd12:3456:789a:1::/64" ]
  [[ "$output" == *"Devices on your local network can reach Storm"* ]]
}

@test "ufw: a rule that was already there is not recorded, so uninstall keeps it" {
  use_ufw
  echo "storm-server ALLOW 192.168.1.0/24" >"$FAKE_DIR/ufw.rules"
  install_server
  [ "$status" -eq 0 ]
  [ "$(record)" = "ufw fd12:3456:789a:1::/64" ]
  [ "$(grep -c '192.168.1.0/24' "$FAKE_DIR/ufw.rules")" = 1 ]
}

@test "ufw, --no-firewall: nothing changes and the exact command is printed" {
  use_ufw
  install_server --no-firewall
  [ "$status" -eq 0 ]
  ! grep -q 'ufw allow' "$FAKE_LOG"
  [ ! -e "$STORM_TEST_SYSROOT/etc/storm/lan-firewall" ]
  [[ "$output" == *"Firewall: left as it is; devices on your network reach Storm through a relay. To allow them directly later: sudo ufw allow from 192.168.1.0/24 to any app storm-server && sudo ufw allow from fd12:3456:789a:1::/64 to any app storm-server"* ]]
}

@test "the question is asked, with the agreed text, and no means no" {
  use_ufw
  platform
  SEL=server
  set_mode server install
  printf 'n\n' >"$BATS_TEST_TMPDIR/tty.in"
  export STORM_TTY="$BATS_TEST_TMPDIR/tty.in" STORM_TTY_OUT="$BATS_TEST_TMPDIR/tty.out"
  OPT_YES=0
  ui_init
  fw_ask
  [ "$FW_CONSENT" = no ]
  grep -qF "Allow devices on your local network to reach Storm directly? (Faster and more private than going through a relay. Storm stays unreachable from the internet.) [Y/n]" "$STORM_TTY_OUT"
  printf '\n' >"$BATS_TEST_TMPDIR/tty.in"
  ui_init
  fw_ask
  [ "$FW_CONSENT" = yes ]
}

@test "the question is not asked again once a rule is recorded, nor on macOS" {
  use_ufw
  platform
  SEL=server
  set_mode server install
  mkdir -p "$STORM_TEST_SYSROOT/etc/storm"
  echo "ufw 192.168.1.0/24" >"$STORM_TEST_SYSROOT/etc/storm/lan-firewall"
  fw_ask
  [ -z "$FW_CONSENT" ]
  use_macos
  platform
  fw_ask
  [ -z "$FW_CONSENT" ]
}

@test "firewalld: the service goes into the home zone only, never public" {
  use_firewalld "home\n  interfaces: eth0\npublic (default)\n  interfaces: wlan0\n"
  install_server
  [ "$status" -eq 0 ]
  grep -q 'firewall-cmd --permanent --zone=home --add-service=storm-server' "$FAKE_LOG"
  ! grep -q 'zone=public' "$FAKE_LOG"
  ! grep -q 'add-port' "$FAKE_LOG"
  [ "$(record)" = "firewalld home" ]
  [ "$(cat "$FAKE_DIR/firewalld.services")" = "home storm-server" ]
  grep -q 'firewall-cmd --reload' "$FAKE_LOG"
}

@test "firewalld on a public-only network: home, and a note that this network stays closed" {
  use_firewalld
  install_server
  [ "$status" -eq 0 ]
  [ "$(cat "$FAKE_DIR/firewalld.services")" = "home storm-server" ]
  ! grep -q 'zone=public' "$FAKE_LOG"
  [[ "$output" == *"this network is in firewalld's public zone, so Storm stays closed on it"* ]]
}

@test "firewalld: external, drop, block and trusted zones are never touched" {
  use_firewalld "external\n  interfaces: a\ndrop\n  interfaces: b\nblock\n  interfaces: c\ntrusted\n  interfaces: d\nwork\n  interfaces: e\n"
  install_server
  [ "$status" -eq 0 ]
  [ "$(cat "$FAKE_DIR/firewalld.services")" = "work storm-server" ]
  [ "$(record)" = "firewalld work" ]
}

@test "uninstall removes only Storm's ufw rules" {
  use_ufw
  echo "22/tcp ALLOW Anywhere" >"$FAKE_DIR/ufw.rules"
  install_server
  [ "$status" -eq 0 ]
  echo "8080/tcp ALLOW Anywhere" >>"$FAKE_DIR/ufw.rules"
  : >"$FAKE_LOG"
  run storm_main --yes uninstall --server
  [ "$status" -eq 0 ]
  [[ "$output" == *"\$ sudo ufw delete allow from 192.168.1.0/24 to any app storm-server"* ]]
  grep -q 'ufw delete allow from 192.168.1.0/24 to any app storm-server$' "$FAKE_LOG"
  grep -q 'ufw delete allow from fd12:3456:789a:1::/64 to any app storm-server$' "$FAKE_LOG"
  [ "$(grep -c '^step=[0-9]* ufw delete' "$FAKE_LOG")" = 2 ]
  [ "$(cat "$FAKE_DIR/ufw.rules")" = "22/tcp ALLOW Anywhere
8080/tcp ALLOW Anywhere" ]
  [ ! -e "$STORM_TEST_SYSROOT/etc/storm/lan-firewall" ]
}

@test "uninstall removes only Storm's firewalld service, from the zones it added" {
  use_firewalld "home\n  interfaces: eth0\n"
  echo "work storm-server" >"$FAKE_DIR/firewalld.services"
  install_server
  [ "$status" -eq 0 ]
  run storm_main --yes uninstall --server
  [ "$status" -eq 0 ]
  grep -q 'firewall-cmd --permanent --zone=home --remove-service=storm-server' "$FAKE_LOG"
  ! grep -q 'zone=work --remove' "$FAKE_LOG"
  [ "$(cat "$FAKE_DIR/firewalld.services")" = "work storm-server" ]
}

@test "uninstall with no recorded rule touches no firewall" {
  use_ufw
  echo "storm-server ALLOW 192.168.1.0/24" >"$FAKE_DIR/ufw.rules"
  install_fake linux-server
  run storm_main --yes uninstall --server
  [ "$status" -eq 0 ]
  ! grep -q 'ufw' "$FAKE_LOG"
  [ "$(cat "$FAKE_DIR/ufw.rules")" = "storm-server ALLOW 192.168.1.0/24" ]
}

@test "without administrator rights the step prints the command and succeeds" {
  use_ufw
  platform
  ui_init
  fw_detect
  [ "$FW_KIND" = ufw ]
  export FAKE_SUDO_DENIED=1
  run server_firewall
  [ "$status" -eq 0 ]
  [[ "$output" == *"The firewall needs administrator rights, so it was left as it is."* ]]
  [[ "$output" == *"sudo ufw allow from 192.168.1.0/24 to any app storm-server"* ]]
  [ ! -s "$FAKE_DIR/ufw.rules" ]
  [ ! -e "$STORM_TEST_SYSROOT/etc/storm/lan-firewall" ]
}

@test "a firewall that refuses the rule never fails the install" {
  use_ufw
  ln -sf "$(type -P false)" "$FAKE_DIR/bin/ufw"
  install_server
  [ "$status" -eq 0 ]
  [[ "$output" == *"ufw refused the rule."* ]]
  [[ "$output" == *"sudo ufw allow from 192.168.1.0/24 to any app storm-server"* ]]
}

@test "subnets: every RFC 1918 and ULA route, nothing public, link-local or CGNAT" {
  export FAKE_IP4_LINK="10.0.0.0/24 dev a\n172.17.0.0/16 dev docker0 proto kernel scope link src 172.17.0.1 linkdown\n172.20.0.0/16 dev b\n172.32.0.0/16 dev c\n100.64.0.0/10 dev d\n192.168.7.0/24 dev e\n169.254.0.0/16 dev f\n"
  export FAKE_IP6="fdab::/64 dev a\nfe80::/64 dev a\n2001:db8::/64 dev a\n"
  [ "$(fw_subnets)" = "10.0.0.0/24 172.20.0.0/16 192.168.7.0/24 fdab::/64" ]
}

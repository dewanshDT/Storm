#!/usr/bin/env bats
# deploy/install.ps1 cannot run here (CI's Windows acceptance runs it), so
# these read it: the firewall rule stays program-scoped and Private-only
# (decision 119), it is asked for, and the script never `exit`s.

PS1="$BATS_TEST_DIRNAME/../../deploy/install.ps1"

code() { grep -v '^[[:space:]]*#' "$PS1"; }

@test "install.ps1: every rule it adds is program-scoped, Storm's port, Private only" {
  rules=$(code | grep -o 'New-NetFirewallRule[^"]*')
  [ -n "$rules" ]
  while IFS= read -r r; do
    [[ "$r" == *"-Profile Private "* ]]
    [[ "$r" == *"-Program '"*"storm-serverw.exe'"* ]]
    [[ "$r" == *"-LocalPort \$Port"* ]]
    [[ "$r" == *"-Direction Inbound"* ]]
  done <<<"$rules"
  ! code | grep -Eiq -- '-Profile[[:space:]]+(Any|Public|Domain)|Private,|,Private'
}

@test "install.ps1: an existing rule open beyond Private is narrowed, never widened" {
  code | grep -q "Where-Object { \"\$(\$_.Profile)\" -ne 'Private' }"
  code | grep -q 'Set-NetFirewallRule -Profile Private$'
  [ "$(code | grep -c 'Set-NetFirewallRule')" = 1 ]
}

@test "install.ps1: it asks first, with the agreed text, and -NoFirewall prints instead" {
  grep -qF "\$FirewallPrompt = 'Allow devices on your local network to reach Storm directly? (Faster and more private than going through a relay. Storm stays unreachable from the internet.)'" "$PS1"
  code | grep -q 'Ask "$FirewallPrompt (Y/n)"'
  code | grep -q '\[switch\]\$NoFirewall'
  code | grep -q 'To allow them directly later, as administrator'
}

@test "install.ps1: without administrator rights the rule is printed, not failed" {
  code | grep -q 'So your phone can reach it directly, run once in PowerShell as administrator'
}

@test "install.ps1 fails with throw, never exit" {
  ! code | grep -Eq '(^|[^A-Za-z$-])exit([^A-Za-z]|$)'
}

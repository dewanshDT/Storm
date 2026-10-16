---
title: Firewalls
description: What Storm opens, what it never opens, and how to check.
---

**Storm never needs an inbound port from the internet, and never opens one
silently.** Away from home, your devices reach the server through a
[relay](/docs/start/concepts/#relays) the server connects out to. On your home
network it is faster and more private to let devices reach Storm directly, so
the installer asks:

> Allow devices on your local network to reach Storm directly? (Faster and
> more private than going through a relay. Storm stays unreachable from the
> internet.)

Yes is the default. No — or `--no-firewall` / `-NoFirewall` — prints the exact
command instead, and so does a run without administrator rights; it never
fails the install. Never by UPnP, never to the internet.

| System | What allows Storm | Scope |
|---|---|---|
| Windows | the rule `Storm Server`, for `storm-serverw.exe` on Storm's port | **Private** networks only |
| macOS | the firewall's own "Allow incoming connections?" prompt | per app |
| Linux, ufw | `ufw allow from <each local subnet> to any app storm-server` | your local subnets |
| Linux, firewalld | the `storm-server` service in the home/trusted zones | never `public`, `external`, `drop` or `block` |
| Linux, neither | nothing to do | — |

Uninstalling removes exactly the rules the installer added, nothing else.
Changing the port with `storm-server up --port N` keeps the ufw profile and
firewalld service on the new port and prints the command that refreshes the
rule; it never edits a rule itself.

## Public networks

On a network your computer treats as untrusted, the server does not announce
itself and refuses password pairing; paired devices and relays work as
before. Windows' network category or the firewalld or NetworkManager zone
decides; macOS has no such signal, so there the server's
**Settings › Connection › Discoverable on this network** switch does. It is
re-read when the network changes.

## Check

```sh
sudo storm-server status
```

`lan` says whether the firewall lets Storm in (and the command to fix it if
not), and `network` whether each interface is trusted, and why. Use `sudo`
where ufw is on: its rules are root's.

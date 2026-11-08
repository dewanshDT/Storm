---
title: Troubleshooting
description: The common problems, and where to look first.
---

**Start with `storm-server status`** on the server (`sudo` on Linux). It
shows whether the service runs, a health probe, whether the firewall lets
Storm in (with the command to fix it), and the server's name, id, key and
relays. On Linux the log is `journalctl -u storm-server -f`.

| Symptom | Check |
|---|---|
| The app says it **couldn't reach the server** | Is the server running (`status`)? Is the device on the same network, or does the server have a relay? Does the firewall allow Storm from the local network ([Firewalls](/docs/install/firewalls/))? |
| The server is not under **On this network** | The network may be one the computer treats as public, discovery may be off (**Settings › Connection › Discoverable on this network**, `STORM_DISCOVERY`), or the network may block discovery. Scan a QR code instead ([Pair a device](/docs/install/pair/)). |
| **Server identity mismatch** when pairing | The code is for another server, or the server's identity changed (a restore without `state/identity/`). Every device must pair again after an identity change. |
| macOS refuses to open the app | It is ad-hoc signed, not notarized: allow it once in **System Settings › Privacy & Security**. |
| Android will not install the APK | Allow installing from the browser or file manager you opened it with. |
| **Offline** or **N unsent** in the status bar | Changes are queued on this device. They send when the server answers; keep the app open until it clears. |
| Conflict markers in a note | Two devices changed the same lines. Edit the note to keep what you want; see [Sync](/docs/notes/sync/). |
| A Runtime Host never comes online | `storm-runtime check` on the host proves the whole path; see the per-platform tables in [Runtime Hosts](/docs/agents/hosts/). |
| `storm-runtime check` or `enroll` says **permission denied**, or `enroll` refuses root | Run it as the host's own account, with the command the message prints (on Linux `sudo -u storm-runtime storm-runtime check`); see [Runtime Hosts](/docs/agents/hosts/). **Not enrolled** and **revoked** mean what they say: enroll it. |
| `claude` or `opencode` shows as **not installed** | On the service, the CLI must be reachable by the host's own account, not just yours ([Runtime Hosts](/docs/agents/hosts/)). On a host you run as yourself, see the next row. |
| A host you run as yourself does not find a CLI your terminal finds | It reads your login shell's `PATH` at start; its log says `login shell: nothing imported` with the reason when it could not. Fix your shell profile, or set `path = [...]` in your `runtime.toml` ([Running a host as yourself](/docs/agents/hosts/#running-a-host-as-yourself)). |
| A host you run as yourself goes **offline** when you log out | On a Mac (LaunchAgent) and on Windows (logon task) it runs only while you are logged in; your next login starts it. On Linux, turn on linger: `loginctl enable-linger`, or ask an administrator to run `sudo loginctl enable-linger <you>` ([As yourself on Linux](/docs/agents/hosts/#as-yourself-on-linux)). |
| A host you run as yourself lists **no workspaces** | Its root is `workspaces` in its state directory on every platform (`~/.local/state/storm-runtime/workspaces` on Linux, `~/Library/Application Support/StormRuntime/state/workspaces` on a Mac, `%LOCALAPPDATA%\StormRuntime\state\workspaces` on Windows): make a folder there, or use **New workspace…**. A root you named yourself is in `workspace_roots` in your `runtime.toml` ([Running a host as yourself](/docs/agents/hosts/#running-a-host-as-yourself)). |
| A host shows **Runs as an unknown account**, and doesn't offer **Agents** | The host's runtime is too old to say which account it runs as. Upgrade it. |
| A session failed with **working directory no longer exists** | The folder it was started in, or would resume in, is gone. Start it again in a folder that exists. |
| A Codex agent another session started seems **idle** and never shows Needs you | Codex has no hook for its own permission prompt, so Storm can't tell it is waiting. Open its terminal and answer the prompt there ([Crews](/docs/agents/crews/)). |
| You want a host to run as you instead of its service account, or back | Enroll a new host, then revoke the old one; old sessions stay as history ([Moving a host](/docs/agents/hosts/#moving-a-host-to-another-account)). |
| An integration says **Needs sign-in** | The service stopped accepting Storm's credential; sign in again from a Storm app. |
| An AI app's MCP calls fail with a `storm_error` | The message names the reason — usually MCP is off in **Settings › AI access**, the key was revoked, or the server is unreachable. |
| Forgot the password | `storm-server passwd` on the server ([Pair a device](/docs/install/pair/#a-forgotten-password)). |

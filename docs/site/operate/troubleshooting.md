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
| `claude` or `opencode` shows as **not installed** | The CLI must be reachable by the host's own account, not just yours ([Runtime Hosts](/docs/agents/hosts/)). |
| An integration says **Needs sign-in** | The service stopped accepting Storm's credential; sign in again from a Storm app. |
| An AI app's MCP calls fail with a `storm_error` | The message names the reason — usually MCP is off in **Settings › AI access**, the key was revoked, or the server is unreachable. |
| Forgot the password | `storm-server passwd` on the server ([Pair a device](/docs/install/pair/#a-forgotten-password)). |

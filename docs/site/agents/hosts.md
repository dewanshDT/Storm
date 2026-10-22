---
title: Runtime Hosts
description: Install the host that runs agents — on Linux, macOS or Windows — and enroll it with your server.
---

A **Runtime Host** (`storm-runtime`) runs agent sessions for one Storm server:
on the server's own machine, a VM, a Mac or a Windows PC. It connects out to
the server, so the server needs no route to it, and it holds no notes.

**The easy way:** choose a Runtime Host (or everything) in the
[installer](/docs/install/server/); it installs the host and enrolls it with
your server. The rest of this page is the by-hand route and the details.

It is the same `storm-runtime` on every platform. Each platform
runs it under the platform's own service manager, as a dedicated account that
exists for nothing else:

| | Linux | macOS | Windows |
|---|---|---|---|
| Install | `apt install storm-runtime` (`.deb`) | Homebrew or the release tarball, then `storm-runtime install` as root | `install.ps1`, or the release zip, then `storm-runtime install` as administrator |
| Service | systemd `storm-runtime.service` | launchd LaunchDaemon `dev.storm.runtime` | Windows Service `StormRuntime` |
| Account | `storm-runtime` | `_stormruntime` (hidden, no login) | `NT SERVICE\StormRuntime` (virtual, no password) |
| Binary | `/usr/bin/storm-runtime` | `/Library/StormRuntime/bin/storm-runtime` | `C:\Program Files\StormRuntime\storm-runtime.exe` |
| Config | `/etc/storm-runtime/runtime.toml` | `/Library/StormRuntime/runtime.toml` | `C:\ProgramData\StormRuntime\runtime.toml` |
| State, key, `HOME` | `/var/lib/storm-runtime` (`home/` inside) | `/Library/StormRuntime/state` (`home/` inside) | `C:\ProgramData\StormRuntime\state` (`home\` inside) |
| Workspaces | `/var/lib/storm-runtime/workspaces` | `/Library/StormRuntime/workspaces` | `C:\ProgramData\StormRuntime\workspaces` |
| Logs | `journalctl -u storm-runtime` | `/Library/Logs/StormRuntime/storm-runtime.log` | `C:\ProgramData\StormRuntime\logs\storm-runtime.log` |

On every platform:
- **The account is its own.** It is never in the server's group, `admin` or
  `staff`, so on a shared machine it cannot read the vaults or
  `state/auth.db`.
- **The enrollment string is read from a prompt or stdin, never from an
  argument**, so `ps` never shows it.
- **Stopping the host ends every session and everything the sessions
  spawned.** The service passes `--exclusive-account`, so at start and after
  shutdown the host also ends any other process left running under its
  account. Those sessions show as `failed (host_restart)` when it comes back.
- **A revoked host ends its sessions, renames `host.json` to
  `host.json.revoked` and exits 3, and stays stopped.** Enroll it again to
  bring it back; no `--force` is needed.

**Enrolling** is the same everywhere: in the app, open **Settings › Hosts &
default agent › Enroll a host**, then paste the one-time string at the host's
`enroll` prompt.

## Linux

```sh
sudo apt install storm-runtime
# Workspaces are the directories under /var/lib/storm-runtime/workspaces.
sudo -u storm-runtime mkdir /var/lib/storm-runtime/workspaces/myproject
# In the app: Settings › Hosts & default agent › Enroll a host. Paste the string here:
sudo -u storm-runtime storm-runtime enroll
sudo systemctl enable --now storm-runtime
sudo -u storm-runtime storm-runtime check        # proves the whole path
```

**Log the agent CLIs in as `storm-runtime`, once.** The unit's `HOME` is
`/var/lib/storm-runtime/home`, so log in under that account's home:

```sh
sudo -u storm-runtime -H claude                 # then /login
sudo -u storm-runtime -H opencode auth login
sudo -u storm-runtime -H env CODEX_HOME=/var/lib/storm-runtime/agents/codex \
  codex login --device-auth
```

**Codex** keeps its login and its conversations in Storm's own Codex home,
`agents/codex` under the host's state directory, not in `~/.codex`: Storm
writes its own session settings there, so log it in with `CODEX_HOME` set as
above (or `codex login --with-api-key`). Everything else in that home is
yours: MCP servers you add with `codex mcp add` (with the same `CODEX_HOME`),
plugins, ChatGPT apps, hooks. They load in Storm sessions beside your
integrations, and Codex asks its own questions about them, such as whether
to trust a workspace's own `.codex/` settings. Codex sessions run **without
Codex's own sandbox**: it needs user namespaces, which this unit forbids, so
the `storm-runtime` account is the boundary, as it is for every agent. Codex
still asks before the commands it is unsure of and before every Storm tool.

Or keep one Claude login in Storm and give it to the hosts you tick:
[Claude login sync](/docs/agents/claude-login/) (optional, for your own
machines only).

The CLIs must be on a system path (`/usr/local/bin`, `/usr/bin`). The unit's
`ProtectHome` hides `/home`, so a CLI installed into a user's home is reported
as not installed.

**Workspaces elsewhere** need the unit widened, and listing them in
`/etc/storm-runtime/runtime.toml`:

```sh
sudo systemctl edit storm-runtime
# [Service]
# ReadWritePaths=/srv/work
```

**The unit is sandboxed,** with one setting deliberately absent:
`MemoryDenyWriteExecute`, under which OpenCode cannot start. The unit's
`RestartPreventExitStatus=3` keeps a revoked host stopped.

Remove it with `sudo apt remove storm-runtime`.

## macOS

The host runs as a **LaunchDaemon under the `_stormruntime` account**, never
as you. It starts at boot whether or not anyone is logged in, and it has the
whole machine: there is no VM in the way, and the job runs at launchd's
`Standard` priority rather than `Background`.

**Install.** With Homebrew (this repository is the tap; it builds from the
release tag, so it needs Rust and takes a couple of minutes):

```sh
brew tap dewanshdt/storm https://github.com/dewanshDT/Storm
brew install storm-runtime
sudo "$(brew --prefix)/bin/storm-runtime" install
```

Or from the release's `storm-runtime-X-macos-universal.tar.gz` (arm64 and
x86_64; ad-hoc signed, not notarized):

```sh
tar -xzf storm-runtime-*-macos-universal.tar.gz
xattr -d com.apple.quarantine./storm-runtime    # only if Gatekeeper refuses it
sudo./storm-runtime install
```

`install` is idempotent. It:
- creates the hidden `_stormruntime` account and group;
- creates the layout in the table above;
- copies the binary to `/Library/StormRuntime/bin`, owned by root, so only
  root can replace what launchd runs;
- writes `runtime.toml` if there is none;
- writes `/Library/LaunchDaemons/dev.storm.runtime.plist` and bootstraps it.

**The job does not run yet.** launchd keeps it alive only while
`state/host.json` exists, so an unenrolled host never runs.

**Enroll.** In the app: Settings › Hosts & default agent › Enroll a host. Then paste
the string at this prompt:

```sh
cd / && sudo -u _stormruntime /Library/StormRuntime/bin/storm-runtime enroll
cd / && sudo -u _stormruntime /Library/StormRuntime/bin/storm-runtime check   # proves the whole path
```

launchd starts the host by itself as soon as enrollment writes `host.json`,
and it comes up `online` in the app. There is nothing to enable.

**Service lifecycle:**

```sh
sudo launchctl print system/dev.storm.runtime           # loaded? running? last exit?
sudo launchctl kickstart -k system/dev.storm.runtime    # restart it
sudo launchctl kill TERM system/dev.storm.runtime       # stop it once; launchd restarts it
tail -f /Library/Logs/StormRuntime/storm-runtime.log
```

**Upgrade** by running the new binary's `install`, which replaces the daemon's
copy and restarts the job:

```sh
brew upgrade storm-runtime && sudo "$(brew --prefix)/bin/storm-runtime" install
```

**Uninstall:**

```sh
sudo /Library/StormRuntime/bin/storm-runtime uninstall           # job, plist and binary
sudo /Library/StormRuntime/bin/storm-runtime uninstall --purge   # also state, logs, config, account
```

**Workspaces are never deleted.** They stay in
`/Library/StormRuntime/workspaces` until you remove them.

**Providers and `PATH`.** launchd gives a job almost no `PATH`, so the host
sets one explicitly, and it is the same `PATH` the agents get:

```
$HOME/.local/bin:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin
```

- **`HOME`** is `/Library/StormRuntime/state/home` and **`SHELL`** is
  `/bin/zsh`; the shell provider runs `/bin/zsh -l`.
- **Set `path = [...]` in `runtime.toml`** to choose the directories
  yourself.
- **Install the agent CLIs where `_stormruntime` can run them:**
  - **Homebrew** (`brew install opencode`, or Claude Code's formula or
    cask);
  - **or as the account itself**, which puts them in its own
    `~/.local/bin`:

    ```sh
    cd / && sudo -u _stormruntime -H sh -c 'curl -fsSL https://claude.ai/install.sh | bash'
    ```

  A CLI inside your own home (`~/.local/bin`, `~/.nvm`) is out of the
  account's reach, by design, and the host reports it as not installed.

**Log the agent CLIs in for `_stormruntime`, once.** The `cd /` matters: the
account cannot read your current directory.

**Claude Code: use a long-lived token.** Its interactive login keeps the
credentials in the login Keychain, which a hidden daemon account does not
have, and it fails there. The token route works:
1. Run `claude setup-token` as yourself, and copy the token.
2. Store it where only the host's account can read it, without it ever
   appearing in a command line:

   ```sh
   sudo -u _stormruntime sh -c 'umask 077; cat > /Library/StormRuntime/state/claude.env'
   ```

   Type `CLAUDE_CODE_OAUTH_TOKEN=<the token>`, then Enter, then Ctrl-D.
3. Name that file in `runtime.toml`. Listing providers replaces the
   defaults, so list all three:

   ```toml
   [[providers]]
   id = "claude-code"
   env_file = "/Library/StormRuntime/state/claude.env"

   [[providers]]
   id = "opencode"

   [[providers]]
   id = "shell"
   ```

**OpenCode** logs in interactively:

```sh
cd / && sudo -u _stormruntime -H opencode auth login
```

Storm never stores or transmits what is in it. Also, a login running as
`_stormruntime` is ended if the host restarts during it (the
`--exclusive-account` sweep), so log in while the host is up.

**Workspaces, permissions and macOS privacy (TCC).**
- **The workspace root is `/Library/StormRuntime/workspaces`.** It is outside
  every user home and every privacy-protected folder.
- **`install` gives it two inheritable ACL entries:** one for
  `_stormruntime`, and one for you (the `sudo` user, or `--operator <user>`).
  A checkout made by you or by an agent stays readable and writable by both,
  and you can work in it without `sudo`.
- **Git checks a repository's owner and ignores ACLs.** It refuses a checkout
  another account owns ("detected dubious ownership"). `install` already
  makes the host's git trust every workspace. Do the same for your own git,
  once:

  ```sh
  git config --global --add safe.directory '/Library/StormRuntime/workspaces/*'
  ```

- **To put an existing project in front of an agent, clone or move it into
  the root:**

  ```sh
  git clone git@github.com:you/project /Library/StormRuntime/workspaces/project
  ```

- **No Full Disk Access is needed, and none should be granted.** A
  LaunchDaemon cannot answer macOS's privacy prompts. A root under `/Users`
  (every home, `~/Documents`, `~/Desktop`, `~/Downloads`, iCloud Drive),
  `/Volumes` (external and network disks) or `/Network` is **refused when the
  host starts**. That keeps an agent out of your files by configuration as
  well as by permission. It also keeps it out of a Storm vault on a mounted
  share.
- **There is no counterpart to systemd's `ProtectSystem` and `ProtectHome`.**
  On macOS the boundary is the account plus file permissions and ACLs.
  Never add `_stormruntime` to `admin` or `staff`, and never give it access
  to a vault.

**Troubleshooting:**

| Symptom | Check |
|---|---|
| The host never appears after enrolling | `sudo launchctl print system/dev.storm.runtime`; `ls -l /Library/StormRuntime/state/host.json` must exist; read the log. |
| `claude` or `opencode` shows as not installed | Run `sudo -u _stormruntime -H sh -c 'command -v claude'` with the `PATH` above. A CLI in your home is not reachable; install it with Homebrew or as the account. |
| The host exits at start with "refused" or "root" | A `workspace_roots` entry is under `/Users`, `/Volumes` or `/Network`, or overlaps a Storm data root. Move it to `/Library/StormRuntime/workspaces`. |
| The host stays stopped and `host.json.revoked` exists | It was revoked. Enroll it again (the command above). |
| An agent cannot read a file you put in a workspace | The file came from outside (moved, not copied, from a home folder), so it lacks the inherited ACL. `ls -le` shows it. Re-copy it, or `chmod -R +a "user:_stormruntime allow read,write,append,delete,readattr,writeattr,readextattr,writeextattr,readsecurity,list,search,add_file,add_subdirectory,delete_child,file_inherit,directory_inherit" <dir>`. |

## Windows

The host runs as the **`StormRuntime` Windows Service under the virtual
account `NT SERVICE\StormRuntime`**, never as you. Windows creates
that account with the service: it has no password, is never an
administrator, and cannot open anyone's profile. Sessions run on ConPTY,
each inside a Job Object, so ending a session, stopping the service, or the
host dying in any way ends everything the session started. Windows 10 1809 /
Server 2019 or later.

**Install.** In PowerShell **as administrator**, either the installer:

```powershell
irm https://dewanshdt.github.io/Storm/install.ps1 | iex
```

or the release zip by hand (it is not code-signed, so SmartScreen may warn):

```powershell
Invoke-WebRequest https://github.com/dewanshDT/Storm/releases/latest/download/storm-runtime-windows-x86_64.zip -OutFile $env:TEMP\storm-runtime.zip
Expand-Archive -Force $env:TEMP\storm-runtime.zip $env:TEMP\storm-runtime
& "$env:TEMP\storm-runtime\storm-runtime.exe" install
```

`install` is idempotent; re-running a newer binary's `install` upgrades. It:
- creates the layout in the table above, each directory with a **protected**
  DACL (nothing inherited from `C:\ProgramData`, where every user may create
  files): SYSTEM and Administrators, plus the account where it needs it;
- shares the workspace root with you (`--operator` names someone else): you
  and the account can both modify everything either creates there;
- copies the binary into `Program Files` and writes `runtime.toml` if there
  is none;
- creates the service as **manual**, so an unenrolled host never runs, with
  restart-on-crash and the account's profile variables in its environment;
- adds `safe.directory` for the workspaces to the account's git config, and
  prints the line for yours.

**Enroll**, still as administrator (no one can run as a virtual account), and
paste the string from Settings › Hosts & default agent › Enroll a host at the
prompt. Enrolling starts the service and makes it start with Windows:

```powershell
& "$env:ProgramFiles\StormRuntime\storm-runtime.exe" enroll
```

**Operate:** `Get-Service StormRuntime`, `Restart-Service StormRuntime`,
`Stop-Service StormRuntime`; `storm-runtime.exe check` (as administrator)
proves the enrollment end to end; the log is in `C:\ProgramData\StormRuntime\logs`.
**Remove:** `& "$env:ProgramFiles\StormRuntime\storm-runtime.exe" uninstall`
(`--purge` also removes the identity, state, logs and config). Workspaces are
always kept.

**Agent CLIs** must be reachable by the account. The host's `PATH` is the
account's own `…\state\home\.local\bin` and `…\AppData\Roaming\npm`, then
the machine `PATH` as the registry has it when the host starts (restart the
service after installing a CLI machine-wide). `claude` finds `claude.exe` or
`claude.cmd`. A CLI installed in your own profile is not reachable.
- **Claude Code needs Git for Windows** (its Bash tool runs Git Bash):
  install it machine-wide.
- **Log a CLI in** from a Storm **shell** session on this host — it runs as
  the account, in PowerShell — or put a token (`claude setup-token`) in an
  `env_file` under `C:\ProgramData\StormRuntime` that `Users` cannot read.

**Workspaces.** Roots under `C:\Users` (every profile and OneDrive), on a
network share, or on a removable, network or optical drive are refused at
startup; other fixed drives (`D:\src`) are fine. To work on an existing
checkout, clone or move it into the workspace root.

**Troubleshooting:**

| Symptom | Check |
|---|---|
| The service stops at once with code 2 | It is not enrolled: run `enroll` as administrator. |
| It stopped with code 3 and `host.json.revoked` exists | It was revoked. Enroll it again. |
| `claude` or `opencode` shows as not installed | Install it machine-wide or as the account (from a Storm shell session), then `Restart-Service StormRuntime`. |
| It exits at start naming AM54 | A `workspace_roots` entry is under `C:\Users`, a share, or a removable drive. |
| git says "dubious ownership" in your own shell | Run the `safe.directory` line `install` printed, as yourself. |

## Never share an account with the vaults

**Never run a Runtime Host under the server's account, or under any account
that can read or write the vaults.** Storm enforces what a session may do
(read only, its one write vault, never delete) in the server, on the tools
it offers. An agent is a process: it can also do anything its OS account can
do to files. A host running as the server's user, or as a member of its group,
can edit or delete vault files on disk and bypass all of that. The 2026-10-08
real-run staging had server, vaults and host under one Unix user, and Claude
offered to edit a note's file directly from a read-only session. Filesystem
permissions are the only boundary there. On a shared machine, check it with
`sudo -u storm-runtime ls /srv/storm` (on a Mac, `sudo -u _stormruntime ls`
the vault root), which must be refused.

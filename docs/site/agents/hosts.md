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

## Two ways to run it

It is the same `storm-runtime` on every platform, and it runs one of two
ways:

- **As yourself.** `storm-runtime install --user`, and the installer's
  default (except beside the server on a Mac or Windows PC, where the
  server runs as you), has your own service manager run it, with no root or
  administrator. Its agents run as you: they
  use your CLIs, your logins and your folders, and they can do whatever you
  can do. Storm adds no sandbox of its own; an agent's own permission prompts
  and sandbox apply as you set them up. See
  [Running a host as yourself](#running-a-host-as-yourself).
- **As a service.** The platform's service manager runs it as a dedicated
  account that exists for nothing else. Its agents run as that account, with
  its own logins and the workspace root below. The installer's `--service`
  (`-Service` on Windows) installs it. See [Linux](#linux),
  [macOS](#macos) and [Windows](#windows).

| | As yourself | As a service |
|---|---|---|
| Linux | `systemd --user` unit, starts at boot with linger | systemd `storm-runtime.service`, from the `.deb` |
| macOS | LaunchAgent, runs while you are logged in | LaunchDaemon, starts at boot |
| Windows | logon task, runs while you are logged on | Windows Service `StormRuntime`, starts with Windows |
| Root or administrator | not needed | needed to install it |
| Agents run as | you | the host's own account |
| Agent CLIs and logins | yours | the account's, logged in once for it |

**Runs as.** Every host tells the server which account it runs as. Its row in
**Settings › Hosts**, and the launcher under **Host**, says so: *"Runs as
storm-runtime"* for the Linux service, or *"Runs as"* and your own account
name for a host you run as yourself. A host set to isolate sessions adds
*"· each session isolated"*. A runtime too old to say, or one that can't name
its account, shows *"Runs as an unknown account"*. When a session starts,
Storm keeps the account, isolation and identity the host had then, and the
session's Details shows them under **Ran as**, whatever the host says later.
A session from before Storm kept this shows *"Not recorded"*. This is status
only: Storm reads no agent setting to report it and changes none.

An agent that starts another agent (agent spawning) needs a host that says
which account it runs as. On a host with isolation `off` the new agent is an
ordinary session of that account, started in the parent's workspace, or in a
folder the parent names that is already canonical (no links, `.` or `..`)
and inside one of `workspace_roots`; anything else is refused. Storm makes
no copy, worktree or sandbox for it, and resuming it is refused once that
folder is gone (*"working directory no longer exists"*). On a host that
isolates sessions it works differently; see
[Linux](#linux).

To move a host from the service to yourself, or back, see
[Moving a host to another account](#moving-a-host-to-another-account).

## As a service

Each platform runs the service under its own service manager, as a dedicated
account that exists for nothing else:

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

For the service, on every platform:
- **The account is its own.** It is never in the server's group, `admin` or
  `staff`, so on a shared machine it cannot read the vaults or
  `state/auth.db`.
- **The service passes `--exclusive-account`**, so at start and after
  shutdown the host also ends any other process left running under its
  account.

For every host, as yourself or as a service:
- **The enrollment string is read from a prompt or stdin, never from an
  argument**, so `ps` never shows it.
- **Stopping the host ends every session and everything the sessions
  spawned.** Those sessions show as `failed (host_restart)` when it comes
  back.
- **A revoked host ends its sessions, renames `host.json` to
  `host.json.revoked` and exits 3, and stays stopped.** Enroll it again to
  bring it back; no `--force` is needed.

**Session identity (Linux).** A session's Storm tools and its Resume reports
reach the host through a local socket. On Linux the host also checks, with
the kernel's peer credentials, that each connection comes from a live process
inside that session; anything else is refused and logged as
`identity_mismatch`, even with a valid handle. At start the host logs
`bridge identity` as `verified`, or as `unavailable` with the reason — most
often `ptrace_scope` (the kernel's `kernel.yama.ptrace_scope` must be 1 or
more) or a kernel older than 6.5. macOS hosts report `unavailable`
(`macos_not_gated`): the macOS check is built, but stays off until it has
been tested with real agents on a Mac. Windows hosts report `unsupported`.
Both keep working as before. This binds a
connection to its session's processes. It does not isolate sessions of one
host from each other: they share an account and its files.

**Known limit: code running inside a session passes as that session.** One
session can get code run inside another's process tree through the files
the account shares — the agent's settings and hooks, the agent's own
install, its global config, a workspace and its git hooks. That code is a
descendant of the other session, so the host accepts it as that session.
On a `verified` host this check stops a session's handle being replayed
from outside it; it does not prove that a request comes from the session's
agent.

A host that is not `verified` — macOS, Windows, a Linux host missing a
precondition, or a runtime too old to say — shows *"Session identity not
verified — Storm can't tell which session is calling"* as an extra line on
its row in **Settings › Hosts** and in each of its sessions' Details. A
verified host shows no identity line. That is a
disclosure, not a restriction: every tool keeps working there. Each of an
agent's calls to Storm or an integration is audited as `verified` or
`unverified`, and a connection the host refused is recorded on the server as
an `identity_mismatch` event on the session it named.

**Enrolling** is the same everywhere: in the app, open **Settings › Hosts &
default agent › Enroll a host**, then paste the one-time string at the host's
`enroll` prompt. For the platform you pick, the sheet lists two sets of
commands: **As you** first, then **As its own account**.

## Limits

Each host has **Limits**, which you set from **Settings › Hosts & default
agent › Limits** on its row. Storm keeps them on your server; nothing is
written to the host, its `runtime.toml` or any agent's settings.

| Limit | Default | Range | What it counts |
|---|---|---|---|
| **Agents per session** | 3 | 1–10 | Agents one session has running at a time |
| **Agents per tree** | 6 | 1–24 | Sessions running in one tree: the session you started and every agent under it. At least agents per session |
| **Concurrent sessions** | the host's own | 1 or more | Sessions running on the host. It can only lower the host's `max_sessions`: a number above it is kept, and the host's number stays in force |
| **Depth** | 1 level | not adjustable | Agents a session starts can't start their own agents yet |

Lowering a limit ends and pauses nothing: what is running keeps running,
and only new starts are refused, each naming the limit it hit. Raising one
applies to the next start. Only you can change them, signed in on a paired
device: no MCP key, host credential or agent can. Every change is recorded
on your server, with the old and new values.

## Installing agents from the app

**Settings › Agents** (beta) lists every agent on every host, with its version
and whether you or Storm installed it, and can **Install**, **Update** and
**Remove** Claude Code, OpenCode, Codex, Cursor and Command Code on a Linux or
macOS host. No sudo and no terminal:

- The host installs into its own folder, `agents/managed` under its state
  directory. On the service it comes first on the search path; on a host
  you run as yourself it comes last, so a CLI on your own `PATH` wins and
  Storm's copy is the fallback. A CLI you installed yourself keeps working,
  and Storm only ever updates or removes its own copies.
- The host downloads from each vendor's own release channel and checks the
  checksum the vendor publishes; a download that does not match is never
  installed. Cursor publishes no checksum, and its row says so.
- Codex and Command Code install through npm, so they need Node.js on the
  host (Command Code needs Node 22).
- A running session keeps the version it started with; the next one gets the
  new one. An agent with a live session can't be removed.
- Once a day the host checks for newer versions and updates what it
  installed while none of that agent's sessions is running. Every install,
  update and removal, automatic or not, is recorded on your server.
- Signing in stays the agent's own: Claude Code from **Settings › Claude
  login**, the others from a shell session on the host (`opencode auth
  login`, `codex login --device-auth`, `cursor-agent login`).

To turn this off for a host, or keep its versions where they are, set in
its `runtime.toml`:

```toml
agent_install = "off"     # no installs, updates or removals from the app
agent_updates = "manual"  # installs from the app, but no automatic updates
```

## Linux

The `.deb` below runs the host as its own `storm-runtime` service account;
`install.sh --runtime --service` installs it this way. To run it as
yourself instead, with no root and on any distribution with systemd, see
[As yourself on Linux](#as-yourself-on-linux).

```sh
sudo apt install storm-runtime
# Workspaces are the directories under /var/lib/storm-runtime/workspaces.
# Make one here, or later from the launcher's "New workspace…".
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
```

**Codex**, on the service, keeps its login and its conversations in Storm's
own Codex home, `agents/codex` under the host's state directory, not in
`~/.codex`: Storm writes its own session settings there. (A host you run as
yourself uses your own; see [Running a host as
yourself](#running-a-host-as-yourself).) The easiest way to log it in, on every
platform, is a **Shell** session on the host (in the app, a new session with
the agent **Shell**). It already runs as the host's account, in a directory
that account can read, with the host's `PATH`:

```sh
export CODEX_HOME=/var/lib/storm-runtime/agents/codex
mkdir -p "$CODEX_HOME" && codex login --device-auth
```

Codex refuses a `CODEX_HOME` that does not exist yet, hence the `mkdir`.
`--device-auth` prints a link and a code, so you finish signing in on
whichever device you are using (or use `codex login --with-api-key`). On
macOS the Codex home is `/Library/StormRuntime/state/agents/codex`. On
Windows, in the PowerShell a Shell session runs:

```powershell
$env:CODEX_HOME = 'C:\ProgramData\StormRuntime\state\agents\codex'
New-Item -ItemType Directory -Force $env:CODEX_HOME | Out-Null; codex login --device-auth
```

From the host's own terminal instead:

```sh
cd / && sudo -u storm-runtime mkdir -p /var/lib/storm-runtime/agents/codex
sudo -u storm-runtime -H env CODEX_HOME=/var/lib/storm-runtime/agents/codex \
  "$(command -v codex)" login --device-auth
```

The `cd /` matters: Codex reads settings from the current directory, and the
account cannot read your home.

Everything else in that home is yours: MCP servers you add with `codex mcp
add` (with the same `CODEX_HOME`), plugins, ChatGPT apps, hooks. They load in
Storm sessions beside your integrations, and Codex asks its own questions
about them, such as whether to trust a workspace's own `.codex/` settings.
Codex sessions run **without Codex's own sandbox**: it needs user namespaces,
which this unit forbids, so the `storm-runtime` account is the boundary, as it
is for every agent. Codex
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

**Per-session isolation is being built, and is off.** The package ships a
second, separate root service, `storm-runtime-isolation`, disabled. When it
is finished, `sudo storm-runtime-isolation on` will run each session as its
own system user in its own sandbox, so sessions on one host cannot reach each
other. Turning it on needs systemd 255 or newer and `passt`, stops nothing by
itself (stop the host first), creates the user pools and gives each workspace
a group; `sudo storm-runtime-isolation off` puts every workspace back as it
was, except setuid and setgid bits, which it never restores. Both refuse a
workspace root that is a symbolic link or has one anywhere in its path, and
one that overlaps a system directory (`/etc`, `/usr`, `/home` and the like),
`/var/lib/storm-runtime-isolation`, or `/var/lib/storm-runtime` itself or its
`agents`, `sessions`, `launch` or `home`. `on` leaves a file with more than
one hard link, or one that neither `storm-runtime` nor a session user owns,
as it is, and lists it. With it on, the host starts each session through the
helper, in its own systemd unit (`storm-session-<id>.service`) under a leased pool user, with
its own `/tmp`, `/dev/shm` and IPC, its own home, and only its own workspace
and session files visible; it never runs one the old way. Each session also
has its own network: it reaches the internet and resolves names through
`pasta` (from `passt`), but not the host's loopback services nor another
session's. If a session must reach a service on the host's loopback (a local
model server, say), list its TCP port in `runtime.toml`:
`isolation_host_ports = [11434]`.

Each isolated session starts in a fresh home holding a copy of your agent's
configuration: its settings, instructions, MCP servers, plugins, skills and
your shell's start-up files. Most of it is read-only to the session, and
nothing it changes there reaches your own configuration; edit yours, and the
next session gets it. **Logins that refresh themselves are never copied**:
Claude Code's own `/login`, Codex's ChatGPT login and OpenCode's OAuth
logins would let one session rotate the tokens out from under another. An
isolated Claude Code session needs a synced token or an API key in its env
file; Codex an `OPENAI_API_KEY` there;
OpenCode an API key, or one of its free models. A session with none of these
fails with `credential_unavailable` rather than using your login. The agents
Storm installs belong to a separate `storm-install` account while isolation
is on: sessions can run them but not change them, and installs and updates
from Settings › Agents run as that account.

Each isolated session reaches Storm through **its own bridge socket**, in
its own session directory, which only it and the host can open; the host
takes a connection there only from the system user the session was leased,
and, where the host's identity is verified, only from under the session's
own process. Pasted images land in the same directory, readable to that
session alone. **Before an isolated host starts any session it runs a
self-test**: two probe sessions, the second trying to see, signal or read
the first, connect to its sockets and local ports, and write where agents
live. The host reports `isolation: verified` only when every attempt is
refused; until then, or if the helper stops answering or `pasta` goes
missing later, it reports `unavailable` with the reason, the app says so on
the host row and in each session's Details, and **new sessions fail with
"isolation unavailable on this host"** while running ones carry on. A host
set to isolate that has no helper to ask (its socket is not there because
`storm-runtime-isolation on` was not run, or the host does not use the
packaged state directory) reports `unavailable: no_helper` at once rather
than waiting on a self-test.

On an isolated host, an agent that starts another agent gives the new one
**a workspace of its own**, made under the
new session's user, in `/var/lib/storm-runtime-isolation/children/`, never
inside a workspace root. In a git repository it is an independent clone of
the commit you have checked out: only committed files, never your untracked,
ignored or uncommitted ones, and nothing from the repository or your git
configuration (hooks, filters, `core.fsmonitor`, includes) runs while it is
made. Submodules and Git LFS files are not fetched. Anywhere else, the new
session sees your workspace read-only and writes in a scratch directory.
Two caps in `runtime.toml`, in bytes, keep clones from filling the disk:
`child_workspace_max` (one child, default 2 GiB) and `children_max` (all of
them, default 20 GiB); a child that would go over either does not start.
When the new session ends its workspace is kept, and belongs to a separate
system user, `storm-review`, which `on` creates. You read what it did, and
**Apply** it if you want it: in a git repository its commits, and what it
left uncommitted (as one more commit by Storm, `.gitignore` respected),
become a new branch, `storm/<session>`, beside yours (Storm never merges, never moves an
existing branch and never touches the branch you have checked out); anywhere
else its files are copied into a new folder in your workspace that you name,
never over one that exists. Storm reads or applies a workspace only for a
session an agent started on that host, once it has ended. A kept workspace
stays until you delete it or dismiss the sessions it came from, and
anything under the children folder Storm has no record of is listed for you,
never removed on its own. On a host turned on before this version, run
`sudo storm-runtime-isolation on` again to create `storm-review`.
Isolation is still being finished, so leave it off for now. A workspace root outside `/var/lib/storm-runtime`
needs the same `ReadWritePaths` drop-in for `storm-runtime-isolation` as for
`storm-runtime`. `storm-runtime.service` itself is unchanged either way.

Remove it with `sudo apt remove storm-runtime`.

**Troubleshooting:**

| Symptom | Check |
|---|---|
| `check` or `enroll` says **permission denied** | `/var/lib/storm-runtime` belongs to the `storm-runtime` account, so run them as it: `sudo -u storm-runtime storm-runtime check`. For a directory you passed with `--state`, run them as the account that owns it. |
| `enroll` **refuses to run as root** | `sudo storm-runtime enroll` would leave a key and `host.json` that only root can read. Run `sudo -u storm-runtime storm-runtime enroll`. |
| It says **not enrolled: there is no `…/host.json`** | Nothing has been enrolled in that state directory. Enroll it (the command above), with the same `--state` if you use one. |
| It says the server **revoked** it, and `host.json.revoked` exists | It was revoked. Enroll it again; the old enrollment is kept under `revoked/`. |
| It says a **key is missing** | `host.json` names a key that is not in `identity/`. Enroll again with `--force`. |

## macOS

The host runs as a **LaunchDaemon under the `_stormruntime` account**, never
as you. It starts at boot whether or not anyone is logged in, and it has the
whole machine: there is no VM in the way, and the job runs at launchd's
`Standard` priority rather than `Background`.

Or run it **as yourself, as a LaunchAgent**: see
[As yourself on a Mac](#as-yourself-on-a-mac). That one needs no `sudo`, uses
your own CLIs, logins and folders, and stops when you log out.

**Install.** The installer's `--runtime --service` installs and enrolls
it this way, and so does `--server --runtime` (or `--everything`) with no
flag, or `--runtime` on a Mac where the server is installed already: the
server on a Mac runs as you, so the host beside it is this account, and
the installer refuses `--user` there. A host you already run as yourself
keeps running that way, and the installer warns. By hand, with Homebrew (this repository is the tap; it builds from the release tag, so it
needs Rust and takes a couple of minutes):

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
   defaults, so list every agent you want offered:

   ```toml
   [[providers]]
   id = "claude-code"
   env_file = "/Library/StormRuntime/state/claude.env"

   [[providers]]
   id = "opencode"

   [[providers]]
   id = "codex"

   [[providers]]
   id = "shell"
   ```

**OpenCode** logs in interactively:

```sh
cd / && sudo -u _stormruntime -H opencode auth login
```

**Codex** logs in most easily from a Shell session on this host (see
[Linux](#linux) for why it has its own home):

```sh
export CODEX_HOME=/Library/StormRuntime/state/agents/codex
mkdir -p "$CODEX_HOME" && codex login --device-auth
```

From your own terminal, name Homebrew's copy by its full path, since `sudo`
may find one in your own home that the account cannot run:

```sh
cd / && sudo -u _stormruntime mkdir -p /Library/StormRuntime/state/agents/codex
sudo -u _stormruntime -H env CODEX_HOME=/Library/StormRuntime/state/agents/codex \
  /opt/homebrew/bin/codex login --device-auth
```

Storm never stores or transmits these logins. Also, a login running as
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
| `check` or `enroll` says **permission denied** | Only `_stormruntime` can read `/Library/StormRuntime/state`. Run them as it, from `/`: `cd / && sudo -u _stormruntime /Library/StormRuntime/bin/storm-runtime check`. |
| `enroll` **refuses to run as root** | `sudo storm-runtime enroll` would leave files only root can read. Use the `sudo -u _stormruntime` command above. |
| An agent cannot read a file you put in a workspace | The file came from outside (moved, not copied, from a home folder), so it lacks the inherited ACL. `ls -le` shows it. Re-copy it, or `chmod -R +a "user:_stormruntime allow read,write,append,delete,readattr,writeattr,readextattr,writeextattr,readsecurity,list,search,add_file,add_subdirectory,delete_child,file_inherit,directory_inherit" <dir>`. |

## Windows

The host runs as the **`StormRuntime` Windows Service under the virtual
account `NT SERVICE\StormRuntime`**, never as you. Windows creates
that account with the service: it has no password, is never an
administrator, and cannot open anyone's profile. Sessions run on ConPTY,
each inside a Job Object, so ending a session, stopping the service, or the
host dying in any way ends everything the session started. Windows 10 1809 /
Server 2019 or later.

Or run it **as yourself, from a logon task**: see
[As yourself on Windows](#as-yourself-on-windows). That one needs no
administrator, uses your own CLIs, logins and folders, and stops when you sign
out.

**Install.** In PowerShell **as administrator**, either the installer
(elevated, `-Runtime` installs the service; `-Runtime -Service` says so
outright):

```powershell
& ([scriptblock]::Create((irm https://dewanshdt.github.io/Storm/install.ps1))) -Runtime -Service
```

or the release zip by hand (it is not code-signed, so SmartScreen may warn):

```powershell
Invoke-WebRequest https://github.com/dewanshDT/Storm/releases/latest/download/storm-runtime-windows-x86_64.zip -OutFile $env:TEMP\storm-runtime.zip
Expand-Archive -Force $env:TEMP\storm-runtime.zip $env:TEMP\storm-runtime
& "$env:TEMP\storm-runtime\storm-runtime.exe" install
```

`install` is idempotent; re-running a newer binary's `install` upgrades. It:
- creates the layout in the table above, each directory with a DACL that
  inherits nothing from `C:\ProgramData` (where every user may create
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
| `check` or `enroll` says **permission denied** | Only administrators and the service can read `C:\ProgramData\StormRuntime\state`: run it in PowerShell as administrator. |
| It stopped with code 3 and `host.json.revoked` exists | It was revoked. Enroll it again. |
| `claude` or `opencode` shows as not installed | Install it machine-wide or as the account (from a Storm shell session), then `Restart-Service StormRuntime`. |
| It exits at start naming AM54 | A `workspace_roots` entry is under `C:\Users`, a share, or a removable drive. |
| git says "dubious ownership" in your own shell | Run the `safe.directory` line `install` printed, as yourself. |

## Running a host as yourself

`--state` is optional for `enroll`, `check` and `serve`, and `--config` for
`serve`. Run as the service account (or as root, or elevated on Windows) they
mean the service's paths in the tables above. Run as anyone else, they mean
paths of your own, which you own:

| | State | Config |
|---|---|---|
| Linux | `$XDG_STATE_HOME/storm-runtime` (`~/.local/state/storm-runtime`) | `$XDG_CONFIG_HOME/storm-runtime/runtime.toml` (`~/.config/storm-runtime/runtime.toml`) |
| macOS | `~/Library/Application Support/StormRuntime/state` | `~/Library/Application Support/StormRuntime/runtime.toml` |
| Windows | `%LOCALAPPDATA%\StormRuntime\state` | `%LOCALAPPDATA%\StormRuntime\runtime.toml` |

`install --user` writes that `runtime.toml` once, if you have none. On every
platform it names one workspace root, `workspaces` in the state directory,
and creates it:

| | Workspace root in a new `runtime.toml` | Log |
|---|---|---|
| Linux | `~/.local/state/storm-runtime/workspaces` | `journalctl --user -u storm-runtime` |
| macOS | `~/Library/Application Support/StormRuntime/state/workspaces` | `~/Library/Logs/StormRuntime/storm-runtime.log` |
| Windows | `%LOCALAPPDATA%\StormRuntime\state\workspaces` | `%LOCALAPPDATA%\StormRuntime\logs\storm-runtime.log` |

Add other roots to `workspace_roots` beside it, then restart the host.
`uninstall --user --purge` never removes a workspace root.

So `storm-runtime enroll`, then `storm-runtime serve`, with no flags, runs a
host as you. `storm-runtime install --user` has your own service manager do
the `serve`: [systemd --user on Linux](#as-yourself-on-linux), a
[LaunchAgent on a Mac](#as-yourself-on-a-mac), or a
[logon task on Windows](#as-yourself-on-windows).
`STORM_RUNTIME_STATE` and `STORM_RUNTIME_CONFIG` still work, and
a flag wins over both.

- **Its agents are you.** They can read and change what you can, use your
  SSH keys, tokens and logins, and run what you can run, `sudo` and
  `docker` included if your account may. Storm does not isolate them from
  you or from each other. What an agent asks before doing is its own
  harness's setting: its permission mode, its rules and its own sandbox, as
  you set them up. See [Security model](/docs/operate/security/#agents-run-as-the-hosts-account).
- **A workspace root may be in your home.** The service's refusals of homes,
  volumes, profiles and shares (AM39, AM54) apply only to the service. A
  root that overlaps a Storm data root is refused to every host.
- **Nothing else of yours is ended.** The account sweep is the service's
  alone. Instead, when a host starts after a crash, it ends only the
  processes that carry the tag of a session it was running.
- **It finds your CLIs as your terminal does.** Once at start, `serve`
  runs your login shell (`$SHELL -l -c`, at most 5 seconds) and uses the
  `PATH` it prints, so a host a LaunchAgent starts finds Homebrew's
  `claude`. A `path = [...]` in your `runtime.toml` still wins; with no
  answer (no `SHELL`, a timeout, an error) the platform's default is kept,
  and the log says which. Under a `systemd --user` unit nothing is read:
  the environment is already your user manager's. On Windows nothing is
  read either: Task Scheduler gives the host your own environment.
- **`import_env = ["NAME", ...]`** in your `runtime.toml` also reads those
  variables from your login shell and gives them to every session; a
  provider's env file still wins. An older host refuses a `runtime.toml`
  with `import_env` (an unknown key stops it at start), so upgrade the host
  before you add it.
- **Codex uses your own home and login**: `$CODEX_HOME`, else `~/.codex`.
  Storm writes nothing there. Its session settings, your integrations and
  the hook that lets Resume find a conversation (with its trust record, so
  Codex runs it without asking) are `-c` options on that session's command
  line alone; the integrations' session handle stays in its environment.
  Your sandbox and approval settings apply as you set them: the service's
  `--sandbox danger-full-access --ask-for-approval on-request`, and its
  "ask before every Storm tool", are the service's only. `codex_home =
  "storm"` in `runtime.toml` keeps Storm's own home under the state
  directory instead. On Windows that is still the default, since cmd.exe
  cannot pass those options to npm's `codex.cmd`. After a host moves to your
  own home (`codex_home = "own"`), the Codex conversations started in Storm's
  (`agents/codex` under the state directory) can no longer be resumed:
  start new ones. Storm does not migrate them.
- **A state directory nested too deep is refused at start.** The host's
  socket lives at `run/mcp.sock` inside it, and a Unix socket's path is at
  most 107 bytes on Linux and 103 on macOS; pass a shorter `--state`.

### As yourself on Linux

`storm-runtime install --user` installs the host as a `systemd --user` unit
that runs as you, with no root, from the `.deb`'s binary or from the
`storm-runtime-<version>-x86_64-linux.tar.gz` release tarball (for
distributions without the `.deb`):

```sh
tar -xzf storm-runtime-<version>-x86_64-linux.tar.gz
./storm-runtime install --user
# In the app: Settings › Hosts & default agent › Enroll a host. Paste the string here:
~/.local/bin/storm-runtime enroll
systemctl --user start storm-runtime
~/.local/bin/storm-runtime check
```

Or let the installer do all of it: `--runtime`, which installs the host as
you by default (and always, on a distribution with no `.deb`);
`--service` installs the `.deb` service instead:

```sh
curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sh -s -- --runtime
```

`install --user`:

- copies the binary to `~/.local/bin/storm-runtime` (one a package put in
  place, owned by root, runs where it is instead), writes
  `~/.config/systemd/user/storm-runtime.service` and enables it, and writes a
  `runtime.toml` if you have none, whose workspace root is
  `~/.local/state/storm-runtime/workspaces`. It starts the host only if it is
  enrolled already, and restarts a running one, so re-running it with a newer
  binary upgrades.
- turns on **linger** (`loginctl enable-linger`), so your user manager, and
  with it the host, starts at boot without you logging in. Where that needs
  an administrator (polkit asks for one over ssh, and some distributions have
  no polkit), it prints the one command for them to run,
  `sudo loginctl enable-linger <you>`; Storm never runs sudo. Without linger
  the host runs while you are logged in. `--no-linger` leaves linger alone.
- writes a unit with **none of the service's sandbox**: no `User=`,
  `ProtectHome` or `RestrictNamespaces`. The host and its agents run as you,
  the way your terminal does, so an agent's own sandbox is not blocked by
  the unit; Storm does not configure one. It keeps
  `RestartPreventExitStatus=3`, so a revoked host stays stopped. Change it
  with `systemctl --user edit storm-runtime`: a re-run rewrites the unit
  itself.
- warns, without refusing, when a Storm server runs as you or you are in the
  `storm` group: either way your agents could read the vaults on disk (see
  [Never share an account with the vaults](#never-share-an-account-with-the-vaults)).

The agent CLIs are yours: log them in as you, once (`claude`, then `/login`;
`opencode auth login`), and the host finds them on your login shell's `PATH`.

**After `apt upgrade`**, when your unit runs the package's
`/usr/bin/storm-runtime`: the upgrade replaces the binary but does not
restart a user unit, so the host keeps running the old one until you run
`systemctl --user restart storm-runtime`.

`storm-runtime uninstall --user` stops and disables the unit and removes it,
the copy in `~/.local/bin`, and linger if install turned it on (or prints
`sudo loginctl disable-linger <you>` if that needs an administrator). It
leaves no file outside your home. Your state and config stay unless you add
`--purge`; workspaces stay either way.

### As yourself on a Mac

`install --user` installs the host as you, without `sudo`: a LaunchAgent in
your own launchd session (`gui/<uid>`), with the per-user paths above.

```sh
storm-runtime install --user
"$HOME/Library/Application Support/StormRuntime/bin/storm-runtime" enroll   # paste the string
launchctl print gui/$(id -u)/dev.storm.runtime                          # state = running
```

It:
- copies the binary to `~/Library/Application Support/StormRuntime/bin`;
- writes `runtime.toml` there if there is none, naming
  `state/workspaces` there as the workspace root, and creates it;
- writes `~/Library/LaunchAgents/dev.storm.runtime.plist`, which runs
  `serve` with `--state` and `--config` set to your paths, and loads it.

As with the LaunchDaemon, launchd runs the host only while `state/host.json`
exists: enrolling starts it, a crash restarts it, and a revoked host stays
stopped. **Logging out stops it**; your next login starts it again. The log
is `~/Library/Logs/StormRuntime/storm-runtime.log`.

- **Workspaces are the folders in `state/workspaces`**, and in any other
  root you add. Add one, then restart the host:

  ```toml
  workspace_roots = [
    "/Users/you/Library/Application Support/StormRuntime/state/workspaces",
    "/Users/you/Code",
  ]
  ```

  ```sh
  launchctl kickstart -k gui/$(id -u)/dev.storm.runtime
  ```

  A root under `~/Documents`, `~/Desktop`, `~/Downloads` or iCloud Drive
  works too: macOS asks you, in your session, before an agent reads it, and
  the answer is yours.
- **Agents run as you**, with the `PATH` your login shell gives, so
  Homebrew's `claude` is found. Claude Code's own login, kept in your login
  Keychain, works; no token file is needed.
- **Never install it on a Mac where a Storm server runs as you.** An agent
  could then read the vaults on disk; `install --user` warns when it finds
  one, and the installer refuses `--user` with `--server` or beside an
  installed server (there, `--runtime` installs the LaunchDaemon). Use the
  LaunchDaemon there.
- **Upgrade** by running the new binary's `install --user`, which replaces
  the copy and restarts the host. **Uninstall** with
  `"$HOME/Library/Application Support/StormRuntime/bin/storm-runtime" uninstall --user`;
  add `--purge` to remove its state, config and logs too. Your folders are
  never touched.

**With Homebrew** instead, `brew services` runs the same host from the same
state, straight from Homebrew's copy (use one or the other: `install --user`
refuses while brew's is there):

```sh
storm-runtime enroll
brew services start storm-runtime
```

Never `sudo brew services`, which would run the host as root. A host
`brew services` started does not start by itself after enrolling: run
`brew services restart storm-runtime`.

### As yourself on Windows

`install --user` installs the host as you, with no administrator: a Scheduled
Task named `Storm Runtime Host` that you register for yourself, with the
per-user paths above.
Run it in a PowerShell that is **not** elevated; elevated, it refuses, since
an elevated shell is the service's.

```powershell
& "$env:TEMP\storm-runtime\storm-runtime.exe" install --user
& "$env:LOCALAPPDATA\StormRuntime\bin\storm-runtime.exe" enroll   # paste the string
Get-ScheduledTask 'Storm Runtime Host' | Get-ScheduledTaskInfo      # LastTaskResult 267009 = running
```

Or with the installer, in a PowerShell that is not elevated, where
`-Runtime` installs the logon task as you (`-Service`, `-Server` or an
elevated shell installs the `StormRuntime` service instead; without `-Yes`
it asks, with as you first):

```powershell
& ([scriptblock]::Create((irm https://dewanshdt.github.io/Storm/install.ps1))) -Runtime
```

It:
- copies the binary to `%LOCALAPPDATA%\StormRuntime\bin`;
- writes `runtime.toml` there if there is none, naming
  `state\workspaces` there as the workspace root, and creates it;
- registers the task: at your logon, as you, only while you are logged on
  (`InteractiveToken`), never elevated, never stopped for its age, restarted
  if it fails. It runs `serve` with `--state`, `--config` and `--log-file`
  set to your paths, inside `conhost --headless`, so no console window opens
  (`--headless` is not documented by Microsoft; were a later Windows to drop
  it, the host would run in a visible console window);
- leaves the task off until you enroll. Enrolling turns it on and starts the
  host; from then on it starts at each logon.

**Signing out stops the host**, and with it every session (each session's
Job Object goes with it); they show as `failed (host_restart)` when your next
sign-in starts it again. The log is
`%LOCALAPPDATA%\StormRuntime\logs\storm-runtime.log`.

- **Workspaces are the folders in `state\workspaces`**, and in any other
  root you add. A root may be in your profile. Add one, then restart the
  host:

  ```toml
  workspace_roots = [
    'C:\Users\you\AppData\Local\StormRuntime\state\workspaces',
    'C:\Users\you\Code',
  ]
  ```

  ```powershell
  schtasks /end /tn "Storm Runtime Host"; schtasks /run /tn "Storm Runtime Host"
  ```

- **Agents run as you**, with your environment and `PATH`, so the CLIs you
  installed for yourself and their logins are the host's.
- **Never install it on a PC where a Storm server runs as you.** An agent could
  then read the vaults on disk; `install --user` warns when it finds one, and
  the installer refuses `-User` with `-Server`. Use the service there.
- **One user per PC.** The task's name is the same for everyone, so a second
  person on the same PC cannot register theirs.
- **Upgrade** by running the new binary's `install --user`, which stops the
  host, replaces the copy and starts it again. **Uninstall** with
  `& "$env:LOCALAPPDATA\StormRuntime\bin\storm-runtime.exe" uninstall --user`;
  add `--purge` to remove its state, config and logs too. Your folders are
  never touched.

The rule below still holds: never run it as an account that can read the
vaults.

## Moving a host to another account

A host's enrollment belongs to its state directory, and the service and a
host you run as yourself each have their own. So a host does not move from
one account to another: you enroll a new one as yourself, then revoke the
old one. Both can run on one machine while you switch.

1. **Install and enroll the new host as yourself**, with the steps for your
   platform: [Linux](#as-yourself-on-linux), [Mac](#as-yourself-on-a-mac) or
   [Windows](#as-yourself-on-windows). The installer keeps a host installed
   as the service running that way, so pass it `--user` (`-User`) here. It appears in **Settings › Hosts** as
   a second host, *"Runs as"* your account. Both take the machine's name
   unless you pass `enroll --name`, so rename one there to tell them apart.
2. **Give it your workspaces.** A new `runtime.toml` names `workspaces` in
   its state directory on every platform; add your own folders to
   `workspace_roots` if you want them too. The service's workspaces stay
   where they are and still belong to its account: clone or copy the
   projects you want into your own root.
3. **Log your agent CLIs in as yourself**, if they are not already. The
   service account's logins stay with it.
4. **Revoke the old host** from its row in **Settings › Hosts**. Its running
   sessions end and show as failed, *"host revoked"*; it stops, and stays
   stopped.
5. **Remove the old service**:
   - **Linux:** `sudo systemctl disable --now storm-runtime`. Keep the
     package if your own unit runs its `/usr/bin/storm-runtime`;
     `sudo apt remove storm-runtime` would remove that binary too.
   - **macOS:** `sudo /Library/StormRuntime/bin/storm-runtime uninstall`
     (`--purge` also removes its state, logs, config and account).
   - **Windows**, as administrator:
     `& "$env:ProgramFiles\StormRuntime\storm-runtime.exe" uninstall`
     (`--purge` also removes its identity, state, logs and config).

   Workspaces are kept either way.

**Old sessions stay as history.** They are listed under **ENDED** with what
they wrote, and their **Ran as** still names the service account (or reads
*"Not recorded"* for one from before Storm kept it). They can't
be resumed: a resume runs on its session's own host, and that host is
revoked.

Moving the other way, from yourself to the service, is the same: install
and enroll the service, revoke the host you ran, then
`storm-runtime uninstall --user`.

## Never share an account with the vaults

**Never run a Runtime Host under the server's account, or under any account
that can read or write the vaults.** Storm enforces what a session may do
(read only, its one write vault, deletes only with your Approve) in the server, on the tools
it offers. An agent is a process: it can also do anything its OS account can
do to files. A host running as the server's user, or as a member of its group,
can edit or delete vault files on disk and bypass all of that. The 2026-10-08
real-run staging had server, vaults and host under one Unix user, and Claude
offered to edit a note's file directly from a read-only session. Filesystem
permissions are the only boundary there. On a shared machine, check it with
`sudo -u storm-runtime ls /srv/storm` (on a Mac, `sudo -u _stormruntime ls`
the vault root), which must be refused.

**A host you run as yourself is you,** so this rule is about your own
account. On Linux the server's data belongs to `storm` and its group:
`ls /srv/storm`, as yourself, must be refused, and you must never join the
`storm` group. On a Mac or a Windows PC where you run the Storm server as
yourself, your account can read the vaults, so a host as yourself could
too: `install --user` warns there, and you should run the service instead.
On Linux `install --user` warns when the server runs as you (its drop-in's
`User=`, its data root's owner, or a `storm-server` process of yours) and
when you are in the `storm` group.

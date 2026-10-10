# Deploying Storm

A single static binary, a systemd unit, and a nightly backup. No container, no
runtime dependencies — the server is a ~5 MB musl binary that runs on a bare
Ubuntu or Debian box with nothing installed.

See decision 8 in `PLAN.md` for why this isn't a Docker image. Prefer the
packaged install (`apt install storm-server` then `sudo storm-server up`) once
a release exists; the steps below are the manual equivalent. Secrets and the
**clean-install** runbook are in [release-secrets.md](release-secrets.md).
After apt works, install under `/srv/storm` and remove the hand-rolled tree /
`~/storm-m15-cutover` — do not treat the NFS cutover kit as permanent.

## First-time setup (packaged)

```sh
# Bootstrap apt (key + source + install) — same idea as Tailscale:
curl -fsSL https://dewanshdt.github.io/Storm/install.sh | sudo sh
sudo storm-server up                    # data root = /srv/storm
sudo storm-server status
```

`install.sh` lives in `deploy/install.sh` and is published at the apt Pages
root by `apt-repo.yml`. Manual key + `sources.list` steps are in
[release-secrets.md](release-secrets.md) if you prefer not to pipe to shell.

`up` creates the `storm` user, writes `/etc/storm/storm.env` (mode 600, with a
generated token), and `systemctl enable --now storm-server`. The web client
lives at `/usr/share/storm/web` and upgrades with the package.

## Updating (packaged)

There is no `storm-server upgrade`. Once the apt source is registered, refresh
through apt. The package replaces the binary and `/usr/share/storm/web`; it
does not rewrite `/etc/storm/storm.env`. Restart so the new binary is what is
listening:

```sh
sudo apt update
sudo apt install --only-upgrade storm-server
sudo systemctl restart storm-server
sudo storm-server status
```

**A vault root on NFS (or any network mount) must be mounted before Storm
starts** (decision 73). The unit orders itself after `remote-fs.target`, and
`storm-server up` adds `RequiresMountsFor=` for its paths. An install set up by
an older `up` gets the second by re-running `up` with the same flags, or with:

```sh
sudo systemctl edit storm-server     # add these two lines, then save:
# [Unit]
# RequiresMountsFor=/mnt/media/Docs/storm /srv/storm/state /srv/storm/backups
```

Name an NFS server by IP or a name that resolves before DNS is fully up: a
`nas.lan` that fails to resolve at boot fails the mount outright.

**Nightly backups never ran from the package before this fix** (decision 75):
the unit named `/usr/local/bin/storm-backup.sh`, which the `.deb` does not
install, and ran as `storm` without being able to read `/etc/storm/storm.env`.
On an install set up by an older `up`, until the fixed package is installed,
give it the right command, environment and user (use the paths and user from
`/etc/systemd/system/storm-server.service.d/data-root.conf`):

```sh
sudo systemctl edit storm-backup     # add, then save:
# [Unit]
# RequiresMountsFor=/mnt/media/Docs/storm /srv/storm/state /srv/storm/backups
# [Service]
# ExecStart=
# ExecStart=/usr/bin/storm-backup.sh
# EnvironmentFile=/etc/storm/storm.env
# User=dewansh
# Group=dewansh
# ReadWritePaths=/srv/storm /mnt/media/Docs/storm /srv/storm/state /srv/storm/backups
sudo systemctl start storm-backup.service && journalctl -u storm-backup -n 12
```

**Up to and including 0.2.9, every local account could read Storm's data**
(decision 76): every note, every index, `vaults.json`, and `state/auth.db`,
which holds the password hashes. `postinst` created `/srv/storm` 0755 and the
unit set no `UMask`, so everything the server wrote came out world-readable.
The fixed package sets `UMask=0027` in both units and removes other-access
from `/srv/storm`, but only from the root directory: it cannot know what an
operator's tree holds below it. An existing install needs this once; until the
fixed package is installed, the drop-ins are what set the `UMask`.

**Run it as one `sudo sh -c`, not as separate `sudo` lines.** On prod, a pasted
block of separate lines stopped the server at the first password prompt, and
the prompt swallowed the remaining lines, including the `start`. This form asks
for the password once, and it ends by starting the server whatever happened
before:

```sh
sudo sh -c '
systemctl stop storm-server
chmod 0750 /srv/storm /srv/storm/state
# The local tree only. -xdev keeps find off any other filesystem, so a vault
# root mounted under /srv/storm is never touched (the NAS sets its modes).
# Symlinks are skipped, never followed.
find /srv/storm -xdev ! -type l -exec chmod o-rwx {} +
mkdir -p /etc/systemd/system/storm-server.service.d /etc/systemd/system/storm-backup.service.d
printf "[Service]\nUMask=0027\n" > /etc/systemd/system/storm-server.service.d/umask.conf
cp /etc/systemd/system/storm-server.service.d/umask.conf /etc/systemd/system/storm-backup.service.d/umask.conf
systemctl daemon-reload
systemctl start storm-server
'
systemctl show -p UMask storm-server               # UMask=0027
sudo -u nobody cat /srv/storm/state/auth.db        # Permission denied
```

Only other-access is removed, so the service user (`User=` in `data-root.conf`)
keeps every bit it had. **A vault root outside `/srv/storm` is deliberately
left alone:** on a NAS share other machines write it, and its modes are the
NAS's to set. An NFS server that assigns its own modes ignores the client's
umask (prod's assigns 664), so keeping the share from other local accounts is
a NAS-side change. Once the fixed package is installed the `umask.conf`
drop-ins are redundant and harmless. If another local account ever needs to
read the data, add it to the service user's group; never restore other-access.

**Upgrades up to and including 0.2.8 disabled the service.** The package's
`prerm` ignored whether it was being removed or upgraded, so each `apt upgrade`
left `storm-server` and `storm-backup.timer` disabled. Both kept running until
the next reboot, which the server did not survive, and the backups stopped.
Check with `systemctl is-enabled storm-server storm-backup.timer`. The fix
arrives with the first release after 0.2.8, and that upgrade re-enables both
once, on any box `storm-server up` configured. Until then, after every upgrade:

```sh
sudo systemctl enable --now storm-server storm-backup.timer
```

Native clients (macOS zip, Android APK) are separate downloads from
[GitHub Releases](https://github.com/dewanshDT/Storm/releases) — upgrade those
on each device when a new release lands.

**NFS / non-`storm` User:** the package `postinst` used to
`chown -R storm:storm /srv/storm` on every upgrade, which breaks a unit that
`up` configured as the state-dir owner (Permission denied on
`vaults.json`). Fixed in postinst (skip chown when `vaults.json` already
exists). If an older upgrade already flipped ownership, restore it to the
`User=` in `/etc/systemd/system/storm-server.service.d/data-root.conf`:

```sh
sudo chown -R dewansh:dewansh /srv/storm   # match your drop-in’s User=
sudo systemctl restart storm-server
sudo storm-server status
```

## First-time setup (manual)

On the server, once:

```sh
sudo useradd --system --home /srv/storm --shell /usr/sbin/nologin storm
sudo mkdir -p /srv/storm/{vaults,state,backups} /etc/storm
sudo chown -R storm:storm /srv/storm

sudo cp deploy/storm-server.service /lib/systemd/system/
sudo cp deploy/storm-backup.service deploy/storm-backup.timer /lib/systemd/system/
sudo cp deploy/storm.env.example /etc/storm/storm.env
sudo chmod 600 /etc/storm/storm.env      # it holds the token
sudoedit /etc/storm/storm.env            # set the paths and the backup path
sudo systemctl enable --now storm-server storm-backup.timer
```

Generate a token with `openssl rand -hex 32`. It lives in the env file rather
than on the command line because process arguments are readable by every local
user through `/proc`.

Then from your machine:

```sh
make deploy HOST=you@your-server
```

## Vaults and the storage root

`STORM_VAULT_ROOT` points at a directory that *contains* vaults — one
directory per vault, each a plain tree of markdown:

```
/srv/storm/vaults/
├── personal/
├── work/
└── recipes/
```

Storm adopts any directory it finds there at startup, and records it in
`state/vaults.json` with a stable UUID. That file maps id → directory → display
name, so renaming a vault does not orphan its index.

**The rescan is not continuous.** A directory dropped into the root over rsync
is picked up at the next restart, not on sight. The file watcher covers note
edits inside registered vaults; it does not register vaults.

**Storm never moves vault directories.** Changing the storage root — from the
app's server settings or this env file — points the server at directories you
have already moved. Change it to somewhere that holds none of the registered
vaults and the API refuses with a `409` listing what would be orphaned, rather
than booting healthy with nothing in it.

## Migrating a single-vault install

Existing deployments have `/srv/storm/vault` and `/srv/storm/state/index.db`.
The move is manual, and worth a backup first — `note_versions` is the merge
base and cannot be rebuilt from the markdown.

```sh
sudo /usr/bin/storm-backup.sh          # and check it wrote something
sudo systemctl stop storm-server
sudo -u storm mkdir -p /srv/storm/vaults
sudo -u storm mv /srv/storm/vault /srv/storm/vaults/personal
sudoedit /etc/storm/storm.env                # STORM_VAULT -> STORM_VAULT_ROOT
sudo systemctl start storm-server
```

On that first boot the server registers `personal` and moves
`state/index.db` into `state/<vault-id>/index.db`, keeping version history.
Confirm before pointing any client at it:

```sh
curl -s -H "Authorization: Bearer $TOKEN" localhost:8484/v1/vaults
```

`--vault` still works for one release: the storage root becomes that
directory's *parent* and the one directory is registered. It logs a deprecation
warning, and refuses to start if the path is missing rather than coming up with
zero vaults.

## Importing a real vault

**Always dry-run against a copy first.** The import adds `id` frontmatter to
every note that lacks one, which is a write to every file:

```sh
storm-server dry-run --vault-root /srv/storm/vaults --state /srv/storm/state
```

It reports, per vault, how many files would gain frontmatter, and writes
nothing. Only run it for real once the counts look right.

## Everyday use

```sh
make deploy HOST=...     # cross-compile, push binary + web client, restart
make deploy-check HOST=... # health, service state, next backup
journalctl -u storm-server -f
```

`make deploy` restarts the service and fails loudly if it doesn't come back.

## The account

Storm has one account (decision 82). You normally set it up from the app on
first pairing; on a headless box, or to reset a forgotten password:

```sh
sudo -u storm storm-server passwd --state /srv/storm/state
```

**Run it as `storm`, not as root**: `auth.db` is created on first use, and a
root-owned one is a database the service cannot write.

Upgrading a 0.3.x server keeps its oldest active owner and removes every
other account, after writing `state/auth.db.pre-v6`. If it has no active owner
it refuses to start; name the account to keep with
`sudo -u storm storm-server single-user --keep <username> --state /srv/storm/state`.

## Backups

`storm-backup.timer` runs nightly at 03:30 with a randomised delay, and
`Persistent=true` so a night the box was off is caught up rather than skipped.

Three things are backed up, for different reasons:

- **the storage root** — every vault's notes. Plain files, so
  `rsync -a --delete` into a dated directory. Dating it means an accidental
  mass-delete is still recoverable from yesterday. **It is the root the server
  uses**, which `storm-server storage-root` reads from `state/vaults.json`, not
  `STORM_VAULT_ROOT` (decision 79): the env file only seeds a first run, and a
  root changed in the app is never written back to it. Until 0.3.0 the script
  copied the env file's directory, which on an install whose root had moved was
  an empty one — a backup that verified and held no notes. The log's `root:`
  line names the directory copied, and the env file's value when they differ.
- **`state/<vault-id>/index.db`** — one index per vault, holding version
  history that the 3-way merge uses as its base, plus `vaults.json`. The rest
  of an index can be rebuilt by rescanning; the history cannot.
- **`state/auth.db` and `state/identity/`** — the server's own identity: its
  `server_id`, the public half of its Ed25519 credential, **its user accounts**,
  and later its sessions. **Nothing rebuilds this.** An index restores itself
  from the markdown; a server that loses its identity cannot prove who it is to
  a single paired device, and a restore without it is a server holding every
  note that nobody can log into. The private key files are copied
  as files at mode `0600` — they are written once and never modified, so there
  is no torn state to worry about, and they have to travel with `auth.db`:
  the database alone restores a server that knows which key is active and
  cannot sign with it.

The databases are **not** rsynced. They run in WAL mode with the server holding
them open, so a file copy can catch committed pages still sitting in the `-wal`
and produce a database that opens but silently lacks rows. `storm-server
backup-db <dir>` uses SQLite's `VACUUM INTO`, which is correct against a live
database, and writes everything in the same layout as `state/` — so restoring
is a plain copy.

`vaults.json` is copied alongside them. Without it the snapshots are a pile of
UUID-named directories nobody can match back to a vault.

The script then reopens the snapshots to check they work. A backup nobody has
read is a guess.

Point `STORM_BACKUP_DEST` at the TrueNAS mount. A local path only protects
against mistakes, not against losing the disk.

### Restoring

```sh
sudo systemctl stop storm-server
sudo rsync -a --delete /path/to/backup/vaults/ /srv/storm/vaults/
sudo rsync -a --delete /path/to/backup/index/ /srv/storm/state/
sudo find /srv/storm/state -name 'index.db-wal' -delete
sudo find /srv/storm/state -name 'index.db-shm' -delete
sudo chown -R storm:storm /srv/storm
sudo systemctl start storm-server
```

Delete the `-wal` and `-shm`: they belong to the databases you just replaced,
and leaving them behind can corrupt the restored ones.

That `rsync` of the snapshot directory carries `auth.db` and `identity/` with
it, which is the point of them being written into the same layout. Check them
afterwards — the private keys must still be `0600` and owned by the service
account:

```sh
sudo ls -l /srv/storm/state/identity/
curl -s http://127.0.0.1:8484/v1/server    # same server_id as before the restore
```

`/v1/server` is unauthenticated, so this works before anything is logged in. A
**different** `server_id` there means the identity did not come back and every
paired device would have to pair again.

If you only have the vaults, that is still enough to read your notes — start
the server against them and the scan rebuilds every index, re-registering each
directory. You lose version history, so the first sync from a device that
edited offline may conflict rather than merge cleanly, and the server comes up
with a **new** identity.

## Relay (optional)

`storm-relay` lets clients reach a server from outside its network
(`docs/srp-v1.md`). It is a **separate package** in the same apt repository,
and nothing needs it: `storm-server` does not depend on it and works the same
without one. It holds no vault data and authenticates no clients (R5, R12);
a client's credential rides inside the tunnel to the origin.

```sh
sudo apt install storm-relay
sudoedit /etc/storm-relay/storm-relay.env   # set STORM_RELAY_PUBLIC_BASE
sudo systemctl enable --now storm-relay
journalctl -u storm-relay -f
```

The package does not start the relay, because a relay with the wrong public
address hands every server a URL that goes nowhere. It runs as its own
`storm-relay` user, never `storm`, so it cannot read a vault on a shared box,
and it listens on `127.0.0.1:8486` (storm-server owns 8484).

**Binding server keys.** Set exactly one of these in the env file; setting
both stops the relay.

- `STORM_RELAY_BINDINGS` (the default): trust-on-first-use, persisted to
  `/var/lib/storm-relay/bindings`. The first key to register a `server_id` owns
  it. Safe only where you control who can reach the relay.
- `STORM_RELAY_ALLOWLIST`: only listed keys register. **Use this on a public
  address.** Same format as the bindings file, so a bindings file you trust can
  be copied over as the allowlist.

`/var/lib/storm-relay` is the relay's only state, and it **cannot be rebuilt**.
Lose the bindings file and every `server_id` is open to whoever registers
next. Back it up. Purging the package deliberately leaves it in place.

**On a public address, the relay terminates TLS itself** (decision 71). Do not
put Caddy or nginx in front of it. The relay takes each client's address from
the socket, never from a header, because a header is something the client
wrote. Behind a proxy every client would be the proxy: one `HELLO` rate limit
shared by everybody, and one `relay_peer_ip` for the origin's login limiter,
so one noisy client locks everyone out.

With certbot, standalone for the first certificate and a deploy hook for every
renewal:

```sh
sudo certbot certonly --standalone -d relay.example.com
sudo tee /etc/letsencrypt/renewal-hooks/deploy/storm-relay >/dev/null <<'HOOK'
#!/bin/sh
# certbot's own files are root-only. Copy them where the relay can read them.
install -d -m 0750 -g storm-relay /etc/storm-relay/tls
install -m 0640 -g storm-relay "$RENEWED_LINEAGE/fullchain.pem" /etc/storm-relay/tls/
install -m 0640 -g storm-relay "$RENEWED_LINEAGE/privkey.pem" /etc/storm-relay/tls/
systemctl reload storm-relay 2>/dev/null || true
HOOK
sudo chmod 755 /etc/letsencrypt/renewal-hooks/deploy/storm-relay
sudo RENEWED_LINEAGE=/etc/letsencrypt/live/relay.example.com \
    /etc/letsencrypt/renewal-hooks/deploy/storm-relay
```

Then in `/etc/storm-relay/storm-relay.env`:

```sh
STORM_RELAY_BIND=0.0.0.0:443
STORM_RELAY_PUBLIC_BASE=wss://relay.example.com
STORM_RELAY_TLS_CERT=/etc/storm-relay/tls/fullchain.pem
STORM_RELAY_TLS_KEY=/etc/storm-relay/tls/privkey.pem
# and an allowlist rather than TOFU, as above
```

`systemctl reload storm-relay` swaps the certificate without dropping a trunk;
a reload that fails (a half-written file, a key that does not match) keeps the
old one and says so in the journal. The relay refuses to start with `--tls-cert`
and a `ws://` public base, and warns about a `wss://` base without
`--tls-cert`. **storm-server trusts the public web PKI only**, so the relay
needs a real certificate; a self-signed one will not register.

**Pointing a server at it.** There is no app screen for this yet. As an
owner, `PUT /v1/config/relays` with `{"relays": ["wss://relay.example.com"]}`.
The server connects to an added relay and disconnects from a removed one (with
a `DEREGISTER`) right away, without a restart; relays in both lists are left
alone (decision 74). `GET /v1/server` lists the relays it has actually
registered with, which is the check that it worked; the `PUT` itself returns
before any connection is attempted. Clients learn the relay from the server when they pair.

## Runtime Hosts (Agent Runtime)

A Runtime Host is a machine that runs agents (Claude Code, OpenCode, a shell)
for one Storm Server. It may be a separate VM, the server's own machine, or a
Mac. It dials the server over the network, so the server needs no inbound
route to it. V1 runs on a direct network; the relay does not carry it yet
(decision 77).

It is the same `storm-runtime` on both platforms (decision 83). Each platform
runs it under the platform's own service manager, as a dedicated account that
exists for nothing else:

| | Linux | macOS |
|---|---|---|
| Install | `apt install storm-runtime` (`.deb`) | Homebrew or the release tarball, then `storm-runtime install` as root |
| Service | systemd `storm-runtime.service` | launchd LaunchDaemon `dev.storm.runtime` |
| Account | `storm-runtime` | `_stormruntime` (hidden, no login) |
| Binary | `/usr/bin/storm-runtime` | `/Library/StormRuntime/bin/storm-runtime` |
| Config | `/etc/storm-runtime/runtime.toml` | `/Library/StormRuntime/runtime.toml` |
| State, key, `HOME` | `/var/lib/storm-runtime` (`home/` inside) | `/Library/StormRuntime/state` (`home/` inside) |
| Workspaces | `/var/lib/storm-runtime/workspaces` | `/Library/StormRuntime/workspaces` |
| Logs | `journalctl -u storm-runtime` | `/Library/Logs/StormRuntime/storm-runtime.log` |

What both guarantee (decisions 77e and 83):
- **The account is its own.** It is never in the server's group, `admin` or
  `staff`, so on a shared machine it cannot read the vaults or
  `state/auth.db` (P3).
- **The enrollment string is read from a prompt or stdin, never from an
  argument**, so `ps` never shows it.
- **Stopping the host ends every session and everything the sessions
  spawned.** The service passes `--exclusive-account`, so at start and after
  shutdown the host also ends any other process left running under its
  account. Those sessions show as `failed (host_restart)` when it comes back.
- **A revoked host ends its sessions, renames `host.json` to
  `host.json.revoked` and exits 3, and stays stopped.** Enroll it again to
  bring it back; no `--force` is needed.

### Linux

```sh
sudo apt install storm-runtime
# Workspaces are the directories under /var/lib/storm-runtime/workspaces.
sudo -u storm-runtime mkdir /var/lib/storm-runtime/workspaces/myproject
# In the app: Settings > Agents > Hosts > Enroll a host. Paste the string here:
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

**The sandbox is §5.8's,** with one setting deliberately absent:
`MemoryDenyWriteExecute`, under which OpenCode cannot start. The unit's
`RestartPreventExitStatus=3` keeps a revoked host stopped.

Remove it with `sudo apt remove storm-runtime`.

### macOS

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
xattr -d com.apple.quarantine ./storm-runtime    # only if Gatekeeper refuses it
sudo ./storm-runtime install
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

**Enroll.** In the app: Settings > Agents > Hosts > Enroll a host. Then paste
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
have, and it fails there (found on a real Mac, F5). The token route works:
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

### Never share an account with the vaults

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

## Security

v1 is **LAN-only**: one shared bearer token, no TLS. That is defensible on a
home network and nowhere else. Before this is reachable from the internet it
needs TLS and per-device tokens — see decision 4 in `PLAN.md`. Do not
port-forward it as it stands.

The unit runs as a dedicated `storm` user with `ProtectSystem=strict` and
write access to `/srv/storm` only. That bounds where the storage root can go:
a root outside `/srv/storm` fails validation as unwritable, so moving it
elsewhere means widening `ReadWritePaths` in the unit first.

The data tree is the service user's and its group's alone (decision 76):
`/srv/storm` is 0750 and both units set `UMask=0027`, so no other local account
can read a note, an index or `state/auth.db`. Check it with
`sudo -u nobody ls /srv/storm`, which must be refused. A vault root on a NAS
keeps the modes the NAS assigns.

One token covers the whole server and every vault on it. There is no per-vault
access control — a second vault is organisation, not isolation.

# Deploying Storm

A single static binary, a systemd unit, and a nightly backup. No container, no
runtime dependencies — the server is a ~5 MB musl binary that runs on a bare
Ubuntu or Debian box with nothing installed.

See decision 8 in `PLAN.md` for why this isn't a Docker image.

## First-time setup

On the server, once:

```sh
sudo useradd --system --home /srv/storm --shell /usr/sbin/nologin storm
sudo mkdir -p /srv/storm/{vaults,state,web,backups} /etc/storm
sudo chown -R storm:storm /srv/storm

sudo cp deploy/storm-server.service /etc/systemd/system/
sudo cp deploy/storm-backup.service deploy/storm-backup.timer /etc/systemd/system/
sudo cp deploy/storm.env.example /etc/storm/storm.env
sudo chmod 600 /etc/storm/storm.env      # it holds the token
sudoedit /etc/storm/storm.env            # set STORM_TOKEN and the backup path
```

Generate a token with `openssl rand -hex 32`. It lives in the env file rather
than on the command line because process arguments are readable by every local
user through `/proc`.

Then from your machine:

```sh
make deploy HOST=you@your-server
sudo systemctl enable --now storm-server storm-backup.timer
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
sudo /usr/local/bin/storm-backup.sh          # and check it wrote something
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
storm-server --vault-root /srv/storm/vaults --state /srv/storm/state --dry-run
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

## Backups

`storm-backup.timer` runs nightly at 03:30 with a randomised delay, and
`Persistent=true` so a night the box was off is caught up rather than skipped.

Both halves are backed up, for different reasons:

- **the storage root** — every vault's notes. Plain files, so
  `rsync -a --delete` into a dated directory. Dating it means an accidental
  mass-delete is still recoverable from yesterday.
- **`state/`** — one index per vault, holding version history that the 3-way
  merge uses as its base, plus `vaults.json`. Everything else can be rebuilt by
  rescanning; these cannot.

The indexes are **not** rsynced. They run in WAL mode with the server holding
them open, so a file copy can catch committed pages still sitting in the `-wal`
and produce a database that opens but silently lacks history. `storm-server
--backup-db <dir>` uses SQLite's `VACUUM INTO` per vault, which is correct
against a live database, and writes them in the same layout as `state/` — so
restoring is a plain copy.

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

If you only have the vaults, that is still enough — start the server against
them and the scan rebuilds every index, re-registering each directory. You lose
version history, so the first sync from a device that edited offline may
conflict rather than merge cleanly.

## Security

v1 is **LAN-only**: one shared bearer token, no TLS. That is defensible on a
home network and nowhere else. Before this is reachable from the internet it
needs TLS and per-device tokens — see decision 4 in `PLAN.md`. Do not
port-forward it as it stands.

The unit runs as a dedicated `storm` user with `ProtectSystem=strict` and
write access to `/srv/storm` only. That bounds where the storage root can go:
a root outside `/srv/storm` fails validation as unwritable, so moving it
elsewhere means widening `ReadWritePaths` in the unit first.

One token covers the whole server and every vault on it. There is no per-vault
access control — a second vault is organisation, not isolation.

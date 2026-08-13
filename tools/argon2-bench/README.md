# argon2-bench — the instrument for Q18

Storm's decision **A1** fixes Argon2id for passwords and says the parameters
are **measured on the deployment box**, targeting 150–300 ms for one verify:

> A KDF tuned on an M-series laptop is a denial of service on a 2-vCPU guest.

This is how that measurement gets made, and re-made — A1's own revisit trigger
is "the target hardware changes materially, which is a re-measure, not a
redesign", so the measurement needed to be repeatable rather than a number
someone once posted in a chat.

Not part of the server build. It is a separate crate with one dependency, and
`make check` never sees it.

## Running it against the VM

```sh
./build.sh                  # static Linux binary, no zig needed
./vm-q18.sh                 # measure on the VM, prod untouched
./vm-q18.sh --with-staging  # also stand up a throwaway server (needs SERVER_BIN)
```

`vm-q18.sh` must run from the homelab LAN — the server is LAN-only
(`PLAN.md` decision 4) — and it fails with that explanation rather than a
timeout when it is not.

**It never touches prod.** No `storm-server up`/`down`, no systemctl, port 8585
instead of 8484, its own vault root and state under `/tmp/storm-staging`, full
teardown at the end, and prod is health-checked before and after.

## Reading the output

The sweep walks memory upward at low iteration counts, because memory is the
axis that makes a GPU attack expensive while iterations only cost the defender.
It recommends the most memory-hard configuration that still lands in the target
window.

The number that decides whether the box survives a login burst is the last one:
Storm verifies on `spawn_blocking`, so *N* simultaneous logins are *N* threads
each holding the full memory cost. The burst is capped at 512 MiB total by
default (`STORM_BENCH_BURST_MIB`) so measuring a live homelab server cannot
push it into swap.

## Reference: an M-series Mac, which is *not* the answer

Recorded only to show the gap A1 is warning about. On an M-series laptop
(10 cores) the sweep tops out needing **192 MiB, t=2, p=1** for 151 ms — a
setting that would be far too slow on a 3-vCPU guest. Measure the guest.

Once the VM numbers exist, write them into the vault note **Storm Auth
Protocol** (and A1 in **Storm Remote Decisions**), then tick Q18 in
**TODO — Storm Authentication**.

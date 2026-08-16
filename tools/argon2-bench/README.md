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

## The answer, measured 2026-08-16

On the homelab VM — 3 vCPU, 3815 MB RAM, Ubuntu, *while prod was serving*,
`nice -n 10`. Raw output in `q18-vm-results.txt`.

```
m = 196608 KiB (192 MiB), t = 1, p = 1   ->  173.6 ms per verify
Params::new(196608, 1, 1, None)
```

Stable across four runs: 173.6 / 173.0 / 173.6 / 174.8 ms.

Three things the sweep settled beyond the headline number:

- **`p` must stay 1.** The p=1 and p=2 columns are identical to within noise,
  because the `argon2` crate does not thread without its `parallel` feature.
  Raising `p` there would buy the memory cost of more lanes and no speed at
  all.
- **Memory over passes.** At a fixed time budget the attacker's cost is bounded
  by memory, so 192 MiB / t=1 beats 96 MiB / t=2 (150.5 ms) even though the
  latter is cheaper per verify. RFC 9106 makes the same ordering.
- **The login path needs a concurrency bound, and this is why.** Storm verifies
  on `spawn_blocking`, whose pool defaults to 512 threads — 512 × 192 MiB is an
  OOM that takes the vault server down with it. A semaphore of 2 keeps the peak
  at ~384 MiB, measured at 207 ms wall for two simultaneous verifies. Do not
  shrink the KDF to work around a missing bound; add the bound.

If the box ever cannot afford that burst, the leanest in-window setting is
**96 MiB, t=2, p=1** at 150.5 ms.

### Reference: an M-series Mac, which is *not* the answer

Recorded to show the gap A1 warns about. The same sweep on an M-series laptop
tops out needing **192 MiB, t=2, p=1** for 151 ms — nearly twice the work for
the same latency. Measure the guest, always.

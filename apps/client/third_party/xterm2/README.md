# xterm2, vendored for Storm

[xterm2](https://github.com/SoFluffyOS/xterm2) 5.2.0 (MIT, see `LICENSE`),
with one change on top (Storm decision 87): `TerminalView` takes Flutter's
`contentInsertionConfiguration`, so the on-screen keyboard can insert images.

To take a later xterm2: copy its `lib/` over this one, then reapply the
commit after "client: vendor xterm2 5.2.0 unchanged".

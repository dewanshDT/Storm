# M0 — Editor spike findings

Answers the one question that gates the project: **can a Flutter `TextField` with a
custom `TextEditingController` carry Storm's markdown editing experience?**

Run it: `flutter test` (67 tests) · `flutter run -d macos` · `flutter run -d chrome`

> **Closed.** The gate this document was written to answer has been run on a
> real device — see *Measured on Android* below. The spike itself was deleted
> once that passed; this file is what it was for.

## Verdict

**Yes for realistic notes; thin above ~5,000 lines.** Nothing here invalidates the
Flutter choice, and the residual risk is bounded and understood. Two caveats below
are worth carrying into M2 rather than forgetting.

## What was validated

### Correctness — the invariant that matters

Flutter lays out the span tree we return, not the raw string. If the spans drop,
duplicate or reorder one character, the rendered text silently diverges from the
buffer and every caret offset past that point is wrong.

`test/tokenizer_test.dart` asserts `span.toPlainText() == buffer` across 42 hostile
cases — unmatched `*`, unterminated fences and frontmatter, `[[a]][[b]]` adjacency,
CJK, emoji, tabs, a 5,000-char single line, `#1` vs `#tag`, URLs containing `#`.
All pass. Tokens are also asserted to tile each line with no gaps or overlaps.

### Performance

`buildTextSpan` only, measured in the test VM on an M-series Mac. This is a floor,
not a frame-time prediction — it excludes text layout and raster.

| Document | typing p50 | typing p95 | worst |
|---|---|---|---|
| 1,000 lines | 0.26 ms | **0.28 – 0.83 ms** | 1.9 ms |
| 4,800 lines | 1.28 ms | **3.0 – 3.8 ms** | 11.0 ms |
| caret movement only (either size) | 0.000 ms | **0.000 ms** | 0.013 ms |

Cold build of a 4,800-line note: **17.8 ms** (2,740 lines genuinely tokenized).
Paid once on open, not per keystroke.

The caret row is the headline. [flutter#114158](https://github.com/flutter/flutter/issues/114158)
is that `buildTextSpan` is re-invoked on cursor movement, not just text change; the
whole-span memo reduces that to a map comparison, so arrowing around a long note is
free. The per-line cache then holds a single keystroke to ~1 re-tokenized line out
of 4,800.

## Two findings to carry forward

**1. Above ~1,000 lines the cost is span-tree assembly, not tokenization.**
At 4,800 lines a keystroke re-tokenizes ~1 line yet still costs 1.28 ms p50. The
rest is splitting the buffer, ~4,800 cache lookups, and allocating a ~15,000-element
span list — all O(lines), all unavoidable while a note is one `TextField`. On a
mid-range Android device (assume 3–5× slower) a 4,800-line note plausibly lands at
6–15 ms of span building alone, which would eat the frame.

*Mitigation already in place:* `maxStyledLines = 5000` degrades to unstyled rather
than stuttering. *If it becomes a real problem:* cache the `children` list and splice
only changed lines, or move to a block-based editor (a `ListView` of per-paragraph
fields). Neither is needed for v1 — real vault notes are almost always < 1,000 lines,
where there is 20× headroom.

**2. Markers are dimmed, not hidden — and that is a deliberate ceiling.**
Obsidian's Live Preview hides `**` and `#` on inactive lines. A `TextEditingController`
cannot change the buffer's character count, so true hiding means rendering markers at
near-zero size, which breaks caret arithmetic and hit-testing. V1 ships Obsidian's
*source mode with good highlighting*. Real hiding needs the block-based editor and is
out of v1 scope.

## Benchmark bug worth remembering

The first run reported a 0.4 ms cold build for 5,000 lines — implausible, and wrong
twice over:

1. `sampleNote` repeated an identical 26-line block, so the per-line cache collapsed
   the document to ~26 distinct lines. Fixed by generating unique text per line.
2. `sampleNote(5000)` emitted **5,024** lines, crossing `maxStyledLines`, so every
   "5,000-line" measurement was timing the *unstyled* fallback. Fixed by trimming to
   an exact line count and benchmarking at 4,800.

Both are now guarded: every perf test asserts `lastDegraded == false`, and the cold
build asserts `lastLinesTokenized > 2000` so a too-repetitive corpus fails loudly
instead of reporting a fast lie.

## Environment blocker (machine, not code)

`flutter build macos` fails before compiling anything:

```
Failed to load code for plug-in com.apple.dt.IDESimulatorFoundation
Symbol not found: ...DownloadableAssetTypeO22developerDocumentation...
Expected in: /Library/Developer/PrivateFrameworks/DVTDownloads.framework
```

`/Library/Developer/PrivateFrameworks/DVTDownloads.framework` is **version 17.0**
(13 Dec 2025) while `/Applications/Xcode.app` is **Xcode 26.6**. Xcode's bundled
system components were never updated to match. Not an SPM issue — disabling Swift
Package Manager changes nothing.

Fix (needs sudo, so run it yourself):

```
sudo xcodebuild -runFirstLaunch
```

Web and `flutter test` are unaffected; both work today.

## Measured on Android (the gate this existed for)

Pixel 10, Android 17, **profile** build — debug Dart is JIT and would have
measured the wrong thing. Real rendered frame times sampled through
`SchedulerBinding.addTimingsCallback`, not span-building in isolation, so this
is what the device actually does rather than a floor.

The display is capped at 60 Hz (`peak_refresh_rate: 60.0`), so the budget is
16.7 ms.

| Document | frame p95 | verdict |
|---|---|---|
| up to 5,000 lines | **8.6 ms** | passes, ~2x headroom at 60 Hz |
| 8,000 lines | n/a — unstyled | degrade threshold engaged, as designed |

The desktop measurements assumed a 3–5x device penalty. Against the 0.4–0.8 ms
span-build figures below, ~8.6 ms of *whole-frame* time on device is consistent
with that guess, and it holds 60fps at the worst styled size. Real vault notes
are almost always under 1,000 lines, where there is far more room.

Crossing to 8,000 lines drops the styling and shows a `DEGRADED (unstyled)`
badge; going back to 5,000 restores it. That is `maxStyledLines = 5000` doing
its job — a responsive unstyled editor beats a styled one that drops frames —
and the switch is reversible rather than a one-way cliff.

## Notes that were outstanding when this was written

- Real frame timing on macOS — blocked on the Xcode fix above.
- Real frame timing on Android, plus IME behaviour with a soft keyboard. Needs the
  Android SDK + a JDK (~10–15 GB; only ~11 GB free on this machine).
- Chrome specifically. Chrome is not installed; the web build runs and can be checked
  in Safari at `flutter run -d web-server`.

The HUD in `main.dart` reports live frame p95, line count, and lines re-tokenized, so
these can be checked by hand the moment each platform is available.

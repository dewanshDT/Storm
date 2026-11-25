---
status: open
tags: [daybreak, bug]
---

Exporting a year of history locks the UI for several seconds on a mid-range phone. Looks like the CSV writer blocks the main thread.

## Repro

1. Seed 400+ days of habit entries.
2. Open Settings → Export.
3. Tap Export CSV.

## Notes

Probably needs to move the write off the UI thread, or chunk it.

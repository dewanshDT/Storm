# Bundled fonts

**Newsreader** — <https://github.com/productiontype/Newsreader>, via
<https://github.com/google/fonts/tree/main/ofl/newsreader>.
Licensed under the SIL Open Font License 1.1; the full text is in `OFL.txt`,
which must ship with the binaries.

Bundled rather than fetched at runtime: Storm is an offline-first app, and a
body font that only arrives when the network does is not a body font.

Used for `MarkdownTheme.base` — the note text only. App chrome stays on the
platform sans, so the serif reads as "this is the document" rather than as a
theme.

`Newsreader.ttf` is a variable font carrying the weight axis, so regular and
bold both come from it; `Newsreader-Italic.ttf` carries the italic axis.

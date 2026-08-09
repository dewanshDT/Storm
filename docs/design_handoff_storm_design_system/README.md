# Handoff: Storm design system + themed prototype

## Overview

A complete visual design system for Storm — the self-hosted, markdown-first notes app in `dewanshDT/Storm` — plus a clickable prototype of every phone and wide screen, both driven from a single token layer. The target is the existing Flutter client at `apps/client/`.

The system exists to end per-screen styling decisions: one token change (accent hue, surface lightness, type scale, radius, border weight) moves every atom, molecule, organism, and screen together. It ships with two working themes — **Storm dark** (the product's real identity) and **SlowFlow earth** (a warm, light alternate) — proving the token layer is genuinely theme-independent rather than dark-only with a light afterthought.

## About the design files

The files in this bundle are **design references created in HTML** — prototypes showing intended look and behavior. They are **not production code to copy**.

The task is to recreate them in **Flutter/Dart inside `apps/client/`**, using the codebase's established patterns: `ThemeData` / `ColorScheme` in `lib/ui/theme.dart`, the named accent enum in `lib/ui/accents.dart`, the existing shell widgets in `lib/ui/shell/`, and Riverpod for state. Do not introduce a web view, do not port the HTML, and do not add a styling package — the token layer maps cleanly onto `ThemeExtension` (see "Design tokens" below).

## Fidelity

**High-fidelity.** Colors, type scale, spacing rhythm, radii, and states are final and measured. Recreate them exactly. Two hard requirements carried from accessibility review:

- Every semantic color (accent, amber, green, danger, `--text3`) must measure **≥ 4.5:1 against the surface it paints on** — `--surface`, not `--bg`. In light mode `--surface` is *darker* than `--bg`, which is the trap that failed review twice.
- The smallest type step is floored at **11px**; the scale ratio must never drive a label below it.

## Design tokens

The whole system derives from ~15 numeric inputs. Implement as a `ThemeExtension<StormTokens>` so both themes share one derivation instead of two hand-written palettes.

### Derivation inputs

| Input | Storm dark | SlowFlow earth |
|---|---|---|
| `mode` | dark | light |
| `bgL` (background lightness %) | 22 | 84 |
| `hue` (neutral hue) | 55 | 62 |
| `chroma` (neutral warmth, /1000) | 8 | 22 |
| `accentH` / `accentC` / `accentL` | 293 / 0.15 / 68% | 55 / 0.06 / 45% |
| `fs` (base type size) | 16 | 16 |
| `scale` (type ratio) | 1.25 | 1.30 |
| `sp` (spacing base) | 8 | 8 |
| `rCard` / `rControl` | 16 / 10 | 2 / 10 |
| `bw` (border width) | 1.0 | 1.0 |
| `shadow` (depth 0–60) | 35 | 16 |
| `dur` (motion ms) | 180 | 320 |

### Derived colors (all OKLCH)

`dir = dark ? +1 : -1`, `n(L) = oklch(L, chroma/1000, hue)`, `bg = bgL/100`

```
--bg          n(bg)
--surface     n(bg + dir*0.05)      ← semantic colours are measured against THIS
--surface2    n(bg + dir*0.09)
--border      n(bg + dir*0.14)
--text        dark ? n(0.95) : n(0.18)
--text2       dark ? n(0.74) : n(0.34)
--text3       dark ? n(0.66) : n(0.40)
--accent      oklch(dark ? accentL/100 : min(0.40, accentL/100), accentC/100, accentH)
--accent-soft oklch(bg + dir*0.08, accentC/300, accentH)
--on-accent   accentL > 0.6 ? oklch(0.16 0.03 accentH) : #ffffff
--amber       oklch(dark ? 0.74 : 0.38, 0.14, 68)
--amber-soft  oklch(bg + dir*0.08, 0.05, 68)
--green       oklch(dark ? 0.74 : 0.38, 0.13, 148)
--danger      oklch(dark ? 0.70 : 0.38, 0.17, 25)
```

Semantic meaning is fixed and must not drift: **accent = interactive** (links, active state, primary action), **amber = tags and highlight only**, **green = synced/good**, **danger = failure/conflict**, **gray (`--text3`) = offline/inactive**.

### Type

| Role | Family | Size |
|---|---|---|
| Display | IBM Plex Sans (chrome sans) | `fs * ratio³` |
| Heading | chrome sans | `fs * ratio` |
| Body (note prose) | **Newsreader** — bundled at `assets/fonts/Newsreader.ttf` | `fs` (16 default, user-adjustable 12–24) |
| Label / metadata | IBM Plex Mono (SlowFlow: Space Mono) | `max(11, fs / ratio²)` |
| Code | IBM Plex Mono | `fs / ratio` |

Three families only. Newsreader must stay bundled — never a network font; the editor's text metrics cannot depend on connectivity.

### Spacing, radius, elevation

- Spacing: multiples of `sp` (8). Gap `sp*2`, card padding `sp*3`, section rhythm `sp*7`.
- Radius: `rCard` for cards/sheets/vault tiles, `rControl` for inputs/bubbles/buttons, `999` for the nav pill. The corner bubbles are **rounded squares** (`rControl`), the nav bubble is a **pill** — this shape difference is deliberate grammar, preserve it.
- Border: `bw` (1.0px), color `--border`.
- Shadow: `0 {shadow/3}px {shadow*1.2}px -8px rgba(0,0,0,{shadow/100})`.
- Motion: `dur` ms, restrained easing. Flutter implicit animations only (`AnimatedContainer`, `AnimatedAlign`, `AnimatedOpacity`), consistent with `nav_bubble.dart`'s existing note about keeping ephemeral UI state out of Riverpod.

## Atomic structure

Build in this order; each tier only composes the tier below it.

### Atoms
`StatusDot` (green/amber/gray) · `TagChip` (amber-soft ground, amber text, ~26px tall — not a 48px tap slab) · `KeyChip` (mono, surface2 ground, `rControl*0.6`) · `AccentSwatch` (the 10 named accents) · `StormButton` (filled / outlined / text) · `StormInput` · `StormSwitch` · `StormCheckbox` · icon set (folder, search, link, hash, plus) · `SaveStateLabel` · `BrandMark`.

**BrandMark uses the real asset** — `assets/icon/storm_icon_full.png`, the hand-drawn tornado on its `#96F2D7` mint ground with `#343A40` strokes. The mint ground is fixed brand color and does **not** re-theme. Never substitute a text wordmark for the mark.

### Molecules
`PropertyRow` (key chip + type-appropriate input) · `NoteRow` (title + one metadata line) · `FolderRow` (icon, name, count, chevron) · `StatusBar` (path · `v12` · save state) · `Breadcrumb` · `TagGroup` (first-segment heading + child chips) · `PopoverItem`.

### Organisms
`VaultCard` (initial on accent ground, name, count, status dot) · `VaultBubble` + `SettingsBubble` (persistent, top corners, every vault screen) · `NavBubble` (always expanded, 6 slots: Directory · Search · New note · New folder · Mentions with count badge · Tags; hidden at ≥900px and while the keyboard is open) · `PropertiesPanel` (bottom sheet on phone, right drawer on wide) · `FormattingToolbar` (11 items: Heading, Bold, Italic, Code, Strikethrough, Highlight, Bullet, Numbered, Task, Quote, Link — replaces the nav bubble whenever the keyboard is open; never both, never neither) · `NoteBody` (live preview: headings, lists, checklists, code, quote, wikilinks in accent, tags as chips) · `AttachmentStrip` · `MentionsSection` (collapsed by default).

### Templates
Dashboard · Directory · Note · Search · Tags · Server settings, at both breakpoints (single 900px threshold, `context.isExpanded`).

### States
Empty (folder / vault / search / tags), loading (skeleton rows), offline (condition, never an error — "Showing your cached copy, N edits queued"), conflict (danger-bordered card explaining the in-text markers).

## Interactions & behavior

- **Tap/click = primary, long-press = secondary options**, applied everywhere something has both.
- **Touch vs pointer diverge on links only:** tap navigates; on desktop a plain click positions the caret and Ctrl/Cmd+click navigates.
- **A block prefix button toggles; a picker does not.** Re-tapping "bullet" removes the bullet; choosing "Heading 1" means *make this H1*, with Paragraph as the explicit off state.
- Nav bubble and formatting toolbar are mutually exclusive, driven by `MediaQuery.viewInsetsOf(context).bottom > 0` **read above the Scaffold** (see the existing doc comment in `nav_bubble.dart` — reading it from inside the body returns zero).
- Accent picking writes the **name** into frontmatter (`color: sage`), never a hex.
- Properties writes must go through `lib/editor/frontmatter_edit.dart`, splicing lines — never re-serialise the YAML block.
- Toolbar buttons must call the existing `TextEditingController`'s own insertion methods; bypassing it is the most likely way to reintroduce a caret-offset corruption bug.

## State management

Existing providers are unchanged — this is a presentation-layer pass. New state is limited to:

- `StormTokens` theme extension resolved from the settings-backed token inputs (persisted alongside `darkMode`, `fontSize`, `bodyFont` in `Settings`).
- Ephemeral UI state (tree expansion, sheet open/closed, picker open) stays local to widgets, out of Riverpod — the rule already recorded in `nav_bubble.dart`.

## Assets

- `apps/client/assets/icon/storm_icon_full.png` — mark on mint ground (already in repo)
- `apps/client/assets/icon/storm_mark.png` — mark, transparent-less white ground
- `apps/client/assets/logo.svg` — vector mark
- `apps/client/assets/fonts/Newsreader.ttf` + `Newsreader-Italic.ttf` — bundled body serif

All already exist in the repo; nothing new to source.

## Files in this bundle

| File | What |
|---|---|
| `Storm Design System.dc.html` | The system: tokens → atoms → molecules → organisms → templates → states, with the live token panel |
| `Storm.dc.html` | The clickable prototype (phone + wide), themed from the same tokens |
| `support.js` | Runtime needed to open the two HTML files locally |
| `apps/client/assets/…` | The brand assets referenced above |

Open either HTML file in a browser; the floating panel at bottom-left drives both themes and every token.

## Suggested implementation order

1. `StormTokens` ThemeExtension + the derivation above, wired to both presets. Verify contrast against `--surface` before building anything on top.
2. Atoms, with a widgetbook/gallery route mirroring section 02 of the system page.
3. Molecules, then organisms — replacing the existing widgets in place rather than adding parallel ones.
4. Templates at both breakpoints.
5. Empty/loading/offline/conflict states — currently the largest genuine gap in the product.

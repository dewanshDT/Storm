# Storm — what the app does today

A functional inventory of every surface, written for someone designing against
it. Read out of the built app on 2026-08-08; routes, state strings, property
types and colour values are literal, not approximate.

Companion to the design briefs — `storm-ui-refactor.md` (M7/M8),
`storm-multi-vault.md` (M9/M10), `storm-properties.md` (M11),
`storm-adaptive.md` (M12). Those say why. This says what.

---

## 1. What Storm is

A notes app that keeps your notes on **your** server. Notes are plain markdown
files in a folder — greppable, backup-able, openable in Obsidian if Storm ever
goes away. A small Rust server in the homelab owns the canonical copy; the
phone, the Mac and the browser are clients that sync to it.

It replaced Obsidian + Syncthing for one person and is in daily use. That shapes
the design brief more than anything else:

- **No accounts, no onboarding funnel, no sharing, no collaboration.** One
  person, several devices, one server, on the home network.
- Notes live in **vaults** — separate collections, e.g. `personal`, `work`.
  Everything below the dashboard happens inside one vault at a time.
- Platforms: **macOS, Android, web, Linux.** One Flutter codebase.

---

## 2. Ground rules

Six constraints that come from how it is built. Not preferences — designing past
them means changing the architecture.

**The vault is plain markdown, always.** Nothing Storm-only goes in the notes
folder. Anything a feature needs to store goes in the note's own frontmatter in
words a human would write: a colour is `color: sage`, never a hex value, because
the file has to still make sense in another app.

**Offline is a normal state, not an error.** Every screen is reachable with no
server. Edits queue and replay when it returns. So every surface needs an
offline reading, and "you are offline" must never look like "something broke".

**The server decides, and conflicts are visible.** Two devices editing one note
is expected. The server merges; when it can't, the conflict is written *into*
the note as marked-up text and the person resolves it by deleting lines. No
hidden copy, no silent overwrite.

**One breakpoint, at 900px.** Below it, the phone layout — the default, and the
reason the project exists. At and above it, a sidebar appears beside the note.
No tablet-specific layout; portrait tablets get the phone one, deliberately.

**Dark first.** Dark is the default and the most used. Light is fully supported
and must be designed, not derived by inverting.

**There is no trash.** Deleting a note removes the file immediately. The server
keeps version history so the text is recoverable by someone who knows to look,
but nothing in the app offers it back.

---

## 3. The map

Every route is also the web client's URL — real, shareable deep links.

```
/connect                      Connect            shown until a server is saved
/                             Dashboard          vaults + recently opened
├── /settings/server          Server settings
├── /v/:vault/browse/…        Directory       ┐
├── /v/:vault/note/:id        Note            │  these four share one frame:
├── /v/:vault/search          Search          │  on a wide screen the vault
└── /v/:vault/tags            Tags            ┘  sidebar sits beside them
```

Back always retraces the real path and never leaves the app from inside a vault.

---

## 4. Surfaces

### Connect — `/connect`

First run, and the only screen shown until a server address is saved.

- **Contains** — product name and one line of orientation; server address field
  (pre-filled with a localhost example); access token field, masked; Connect
  button.
- **Actions** — test and save. The address is verified *before* it is stored, so
  a typo surfaces here rather than as an empty vault later.
- **States** — empty · testing · `The server rejected that token.` ·
  `Couldn't reach the server. Is it running?`

### Dashboard — `/` · adapts at 900px

Home. Which vault, or what you were last working on.

- **Contains** — a grid of vault cards (name, note count, the vault's accent
  colour as its fill); below, **Recently opened**: full-width rows with the note
  title and which vault it came from, merged across every vault.
- **Actions** — open a vault · open a recent note · create a vault · set a
  vault's colour · reach server settings.
- **States** — loading · no vaults yet · no recents yet · `Directory not found`
  · offline (served from cache).
- **Wide** — cards stop stretching and flow at a fixed size; recents move to a
  340px right-hand rail. At phone width it stays two columns.

> A vault whose folder has gone is shown **greyed and labelled**, not hidden.
> Disappearing would read as "my notes are gone".

### Directory — `/v/:vault/browse/…` · adapts at 900px

The folder tree, drilled into one level at a time.

- **Contains** — breadcrumb of the current path; rows for folders and notes,
  mixed, folders first.
- **Actions** — open a folder or note · create a folder · rename or delete a
  folder · long-press a row for its actions.
- **States** — loading · empty folder · empty vault · offline.
- **Wide** — replaced by the sidebar's expandable tree; the main pane shows
  **Select a note**.

### Search — `/v/:vault/search`

Full-text across the current vault.

- **Contains** — one field, placeholder `Search notes`; results as note title,
  path, and a snippet with matched words marked.
- **States** — nothing typed · no matches · search failed.
- Server-side and fast (~1ms across the vault). Design it as instant.

### Tags — `/v/:vault/tags`

Browse by tag instead of by folder.

- **Contains** — every tag with a count; nested tags grouped under their parent;
  the notes under a chosen tag.
- **States** — no tags in this vault · tag has no notes.

### Note — `/v/:vault/note/:id` · adapts at 900px

Where the time goes. **Reading and writing are the same screen** — there is no
separate preview mode.

Top to bottom:

1. **Status bar** — the note's path, any error, the version number (`v12`), and
   the save state.
2. **Properties** — the note's frontmatter as a typed key/value list. Nine
   types: `text`, `number`, `checkbox`, `date`, `datetime`, `list`, `select`,
   `url`, `color`. `created` and `modified` are read-only and shown as friendly
   dates; `id` is hidden unless switched on. This list is the **only** way to
   edit frontmatter — there is no raw-YAML mode.
3. **The editor** — markdown styled live *in the text itself*. Headings, bold,
   italic, code, quotes, lists, task boxes, highlights, tags and wikilinks all
   render as you type, with the syntax still present and editable.
4. **Attachments** — a strip of files the note references.
5. **Mentions** — other notes that link to this one.

- **Formatting toolbar** — Heading · Bold · Italic · Code · Strikethrough ·
  Highlight · Bullet list · Numbered list · Task · Quote · Link to a note.
- **Note menu** — Keep available offline · Attach a file · Rename or move ·
  Delete.
- **Other behaviours** — typing `[[` suggests notes to link; tapping a wikilink
  follows it; Enter in a list continues the list; autosave is debounced.
- **States** — `Unsaved` · `Saving…` · `Saved` · `Queued — offline` · `Failed` ·
  merged (server text adopted) · conflict (markers in the body) · not available
  offline yet.
- **Wide** — opens beside the sidebar; picking another note swaps only this pane.

> **The hard one.** A conflict puts `<<<<<<<` markers into the body and shows a
> banner. The person fixes it by editing the text. Honest and lossless, and the
> least designed thing in the app.

### Server settings — `/settings/server`

Everything about the **server**, rather than about this device.

- **Vault storage root** — the folder on the server holding the vaults,
  editable.
- **AI access** — two switches: let an assistant read the notes, and a nested
  one to let it change them. Off by default; the second is dead while the first
  is off.
- **Vaults** — the list, with create, rename and remove.
- **States** — not connected · "would orphan every vault" confirmation · AI
  access off / read-only / read and write.

> Two actions sound destructive and are not, and the copy says so: changing the
> storage root never moves files, and removing a vault only forgets it.

### Appearance & connection (device settings)

Dark mode · body font (Serif = bundled Newsreader, Sans, Monospace) · font size
(default 16) · show the note `id` in properties (off) · server address and
token.

---

## 5. Always-on chrome

Three floating elements sit over every vault screen. **No bottom tab bar, no
drawer.**

- **Vault bubble · top left** — the current vault's initial, its accent colour,
  and a dot showing whether the server is reachable. Tap to switch vault. It
  answers "which one am I in".
- **Settings bubble · top right** — reaches settings from anywhere.
- **Nav bubble · bottom centre** — a floating pill, **always expanded**, never
  collapsed behind a kebab. Carries Directory · Search · New note · New folder ·
  Mentions (with a count when the open note has any) · Tags.

At 900px and above the pill is hidden and those same actions become the toolbar
at the top of the sidebar. Both are drawn from one list, so they can never offer
different things.

### Desktop keyboard shortcuts (M18)

Platform-aware (⌘ on macOS / Mac web, Ctrl elsewhere). Phone touch layout
unchanged. Nested: global · note open · editor focus only.

| Action | macOS | Win / Linux |
|---|---|---|
| Search | ⌘ K | Ctrl K |
| New note | ⌘ N | Ctrl N |
| New folder | ⌘ ⇧ N | Ctrl ⇧ N |
| Toggle sidebar | ⌘ \\ | Ctrl \\ |
| Read ↔ Edit | ⌘ E | Ctrl E |
| Save | ⌘ S | Ctrl S |
| Find in note | ⌘ F | Ctrl F |
| Bold / Italic | ⌘ B / I | Ctrl B / I |
| Dismiss / leave | Esc | Esc |

Undo/redo stay with the system text field. No ⌘/Ctrl R or W (browser-hostile).
Deferred: shortcut overlay, command palette.

---

## 6. State vocabulary

The literal strings the app shows. This is the whole of its feedback language —
worth keeping or deliberately replacing.

| Shown | Means | Where |
|---|---|---|
| `Unsaved` | Edited, not yet sent | Note status bar |
| `Saving…` | In flight | Note status bar |
| `Saved` | The server has it | Note status bar |
| `Queued — offline` | Held locally, will replay | Note status bar |
| `Failed` | Rejected; the edit is still here | Note status bar |
| `v12` | Which version you are editing from | Note status bar |
| `Offline` | The server cannot be reached | Vault bubble, toasts |
| `N unsent` | Edits waiting to sync | Vault bubble |
| `Not connected` | No server saved yet | Settings |
| `Directory not found` | The vault's folder is gone | Vault card |
| `Kept available offline` | Pinned for offline reading | Note menu toast |
| Not available offline yet | Never fetched, and no server now | Note screen |

---

## 7. Colour and type

### Accents

Notes and vaults can carry one of ten accents. **The name is what gets written
into the file** (`color: sage`), so the set is fixed and the words are part of
the product. Each has a light and a dark value; a card uses it at full strength,
a page tints with it at ~40%.

| Name | Light | Dark |
|---|---|---|
| `none` | — | — |
| `coral` | `#FAD2CF` | `#5C2B29` |
| `peach` | `#FDE2CE` | `#614A19` |
| `sand` | `#FFF8B8` | `#635D19` |
| `sage` | `#E6F4D7` | `#345920` |
| `mint` | `#D4E4ED` | `#16504B` |
| `sky` | `#D3E3FD` | `#2D555E` |
| `lavender` | `#E9D9FB` | `#42275E` |
| `blossom` | `#FDCFE8` | `#6C394F` |
| `clay` | `#E9E3D4` | `#4B443A` |

The product mark is a hand-drawn tornado on a mint card: `#96F2D7` ground,
`#343A40` strokes.

### Type

- **Note bodies** — Newsreader, a serif, bundled with the app so it works
  offline. The default.
- **Alternatives** — the platform's own interface face, or its monospace.
- **App chrome** — the platform face throughout.
- **Size** — adjustable, default 16.

Only faces that ship with the app or the platform: downloading a font at runtime
is wrong for something that must work with no network, and it would make the
editor's text metrics depend on the connection.

---

## 8. Gaps and open questions

Known holes, and the decisions most worth a designer's answer.

**Tables don't render.** A markdown table shows as raw pipes that wrap mid-row.
Not an oversight: the editor is one text field whose styling must map to the
underlying characters exactly, and a table needs column layout, which that model
cannot express. Fixing it means a separate rendered reading view.
→ *Is reading a distinct mode with its own design, or should the editor stay the
only view and tables just be made legible?*

**Conflicts are raw.** A banner, then git-style markers to delete by hand.
→ *What should choosing between two versions look like on a phone?*

**Properties crowd a small screen.** The typed list is the only way to edit
frontmatter, and a note with many properties pushes the writing far down.
→ *Collapse by default, or somewhere else entirely?*

**Delete has no undo.** No trash anywhere in the product.
→ *Design a trash, or design a confirmation that earns the risk?*

**AI access is a settings row.** An assistant can read, and optionally change,
every note. Today that is two switches on a settings screen.
→ *Should it be visible while it is on — and should a note an assistant wrote be
marked as such? The file already records `source: ai`.*

**Empty states are unwritten.** New vault, empty folder, no search matches, no
tags: all present, none designed.

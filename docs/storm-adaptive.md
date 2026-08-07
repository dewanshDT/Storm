# Storm adaptive layout — implementation doc (v0.1)

Scope: make the client use a wide window. A folder-tree sidebar beside the note, a flowing card grid on the dashboard, and recents in a rail — without changing anything at phone width.

Status: **built.** M12 in `PLAN.md`, decisions 31–34.

---

## 1. What was wrong

There was no responsive handling anywhere in the client. The dashboard was `GridView.count(crossAxisCount: 2, childAspectRatio: 1.35)`, so a 2000px browser window produced vault cards roughly 980×725 — an inch of colour and a label. The floating nav pill sat at the bottom centre of the screen whatever its size, and only one pane ever showed when there was room for two.

The phone layout is right, and it is the reason the project exists. So this pass adds one breakpoint and makes the wide branch additive.

## 2. One breakpoint

`lib/ui/breakpoints.dart` — `kExpandedWidth = 900` and `context.isExpanded`. That is the whole abstraction.

Deliberately one threshold: the only structural question is "is there room for two panes", and each extra breakpoint multiplies the states that need testing. Tablet portrait stays on the phone layout.

`MediaQuery.sizeOf` rather than a `LayoutBuilder`, because it is a property of the *window* — every widget has to agree on it regardless of the box it is laid out in, and it must rebuild continuously while a browser edge is dragged.

## 3. Where the reuse is, and where it is not

The tree and the drill-down are **different navigation models** — one replaces the screen, the other opens a branch while everything else stays visible. One widget doing both would carry two sets of rules. What they share is one level down:

| Shared | Used by |
|---|---|
| `childrenOfFolder(notes, folder, known)` | the browser once per screen, the tree once per expanded node |
| `EntryTile` | both, so a folder row looks and behaves identically |
| `vaultActions(context, ref, uri)` | the floating pill and the sidebar toolbar |

So: one data source, one row, one action list — two arrangements.

## 4. `ShellRoute`

The four vault routes moved under a `ShellRoute` whose builder is `VaultShell`. This is not cosmetic: wrapping each route's child individually rebuilt the whole subtree on every navigation, so the tree would collapse the moment a note opened. `ShellRoute` builds the frame once and swaps only the pane.

`VaultGate` moved into `VaultShell`, so it wraps once instead of five times.

Tree expansion is then ordinary widget state, keeping the rule recorded in `nav_bubble.dart`: ephemeral UI state stays out of Riverpod, because the providers there drive sync and cache.

The paths are unchanged, so decision 17 still holds — `go` replaces, deeper `push`es, and back never leaves the app. `back_navigation_test.dart` is what proves it.

## 5. Behaviour by width

| Route | Compact | Expanded |
|---|---|---|
| `/v/:vault/browse[/path]` | full-screen listing | sidebar + "Select a note" |
| `/v/:vault/note/:id` | full-screen editor | sidebar + editor |
| `/v/:vault/search`, `/tags` | full-screen panel | sidebar + panel |
| dashboard | stacked grid + recents | flowing grid + 340px recents rail |

The sidebar auto-expands the folders leading to the note in the URL, so a deep link lands with its surroundings visible. Selecting a note uses `go`, not `push`: the sidebar stays and only the pane changes, so back leaves the vault rather than replaying everything glanced at.

`GridView.extent(maxCrossAxisExtent: 220)` replaces the fixed column count. At 411px that still works out to two columns — the phone is unchanged — and at 1900 it flows to eight card-sized cards.

## 6. Two things worth remembering

**A test that exercises a feature is not a test that would notice its absence.** The guard for the `ShellRoute` — "the tree keeps its expansion when a note is opened" — passed twice against deliberately broken code. First because `find.text('Ideas')` also matches the note's AppBar title; then, once scoped to the sidebar, because `_revealOpenNote` re-expands the ancestors of whatever note is in the URL, so opening a note *from the folder under test* reopens that folder from a freshly built tree. It only became a guard once it expands `Daily` and opens a note at the root.

**An assertion was hiding a defect.** Three existing wide-screen tests began failing with "Multiple exceptions detected", which was Flutter's *"ListTile background color or ink splashes may be invisible"*: the sidebar used a coloured `Container`, and `ListTile` paints its ripple onto the nearest `Material`. The fix was a `Material` — not silencing an assert, but making every tap in the tree actually ripple.

## 7. Non-goals

- Three panes (folders | notes | editor). Two new widgets and a second selection concept, and it collapses badly under 1400px.
- A resizable or collapsible sidebar, split panes, more than one note open at once.
- Any change to the server, the wire format, sync, or the editor itself.

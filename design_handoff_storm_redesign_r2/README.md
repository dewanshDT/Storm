# Handoff: Storm redesign round 2 (v0.15.0 surfaces)

## Overview
Round 2 redesigns the surfaces that shipped in v0.12.0–v0.15.0 and work well but look bolted on. It answers the 12 Nov 2026 redesign brief and builds on the approved v2 handoff (`design_handoff_storm_v2/`) and the crews design (`design_handoff_storm_crews/`). Read those first. Anything not covered here stays as they specify.

It covers:
- the app bar;
- the Agents sidebar;
- the session view (decision strip, inspector icon rail, Agents tab);
- the launcher;
- Trash and retention;
- Settings › Agents & hosts, Integrations and AI access copy;
- the phone versions of the session view and the launcher.

## About the design files
The `.dc.html` files are **design references built in HTML**, not code to port. Recreate them in the Flutter client with its existing widgets, `lib/ui/tokens.dart` and patterns. Open them in a browser with `support.js` beside them.

| File | What it is |
|---|---|
| `Storm Redesign R2.dc.html` | **The approved design.** 16 frames, each captioned with what it replaces, plus open questions. Theme tweak: dark, light, earth. |
| `Agents Sidebar Rules.dc.html` | The Agents sidebar rule set. **Variant A is approved.** B is kept for comparison. |
| `Window Chrome.dc.html` | The window's top. **B (full-width app bar) is approved.** A and today's build are kept for comparison. |
| `Storm As Built.dc.html` | Today's v0.15.0 surfaces, for reference. **Don't build from it.** |
| `Redesign Frame`, `Storm Blocks`, `Agents Hosts Page`, `As Built Frame` | Frame renderers used by the pages above. |

**Fidelity:** high. The frames use the real tokens, type and spacing. Use token names, never literals (`token_conformance_test`).

---

## 1. App bar (Window Chrome B)
- **Bar:** 44px tall, full width, above the rail, sidebar and main pane. Background `bg`, the rail's colour, so the bar and rail read as one L-shaped frame. A 1px `border` line under it; it's the only full-width line in the window. Content sits 2px below centre, padding-top 2px.
- **macOS:**
  - Window buttons at x 20, centred.
  - Back and forward 18px after them; forward is dimmed when unavailable.
  - The health dot (9px) sits at the right end. It moves here from the rail.
- **⌘K field:**
  - Centred, 420px wide, shrinking to 280 at narrow widths.
  - `bg`, with a 1px `border` outline and `rControl` radius.
  - Placeholder: "Search notes, jump to a session…", with ⌘K on the right.
  - It replaces the Notes sidebar's own search field, which is removed.
- **Behaviour:** blank space in the bar drags the window, and double-clicking it zooms. When the window isn't focused, the buttons turn grey and the field `text3`. In full screen the buttons go away and back and forward move to x 20. The bar never hides.
- **Windows and Linux:**
  - Caption buttons (46px each, full bar height) sit at the right end; close turns red on hover.
  - The health dot sits left of the caption buttons.
  - Linux follows the desktop environment's button order.
- **Web:** no window buttons; the bar starts with back and forward.
- **Phone:** no bar.
- **Below the bar:**
  - The rail's first icon starts 8px below the bar.
  - The sidebar's first row starts 12px below it.
  - Page titles start 14px below it.

  All three share a top line.

## 2. Agents sidebar (Sidebar Rules A)
**Header:** one row in place of the title, the New session button and the Overview row. "Agents" (14/600) opens the overview when clicked. On the right, a 28px ＋ button (`accentSoft` background, `accent` icon) starts a new session; its tooltip shows ⇧⌘N.

**One signal per channel.**

| Channel | Meaning |
|---|---|
| **Row outline** | **Waiting for you:** a permission prompt, a question, or a delete approval. A 1px solid `danger` inset outline on a 9% `danger` tint. This is the only state that raises the count on the rail and on the phone's place bubble. It's solid, because dashed already means lineage. |
| **Mark badge** | **What the agent is doing:** working = filled `accent`, pulsing; idle = `accent` ring; starting = filled `text3`; failed = filled `danger`. Parked, out of usage and unknown have no badge, and their name turns `text2`. |
| **Sub-line** | **Why, in words**, for example "permission prompt · 3m", "blocked · waiting on test-gateway", "failed · host restarted", "out of usage · resets 15:00" or "host offline · last seen 9m". Otherwise "workspace · age". The host shows only when the crew header doesn't already name it. **No tag pills and no amber.** |

**Sorting**
- Order: needs you → blocked → failed (for 1 hour) → unknown → out of usage → working → idle → parked.
- A row only moves when it enters or leaves the top four tiers. Below those tiers, your order holds.
- The order freezes while the pointer is in the sidebar.
- Crews sort among crews by their most urgent member, and so do trees within RUNNING.
- Inside a crew, the urgent tiers come first, then your drag order.
- Children stay under their parent, and a tree moves as one.

**Roll-ups when collapsed:** a collapsed crew or tree shows its most urgent state. "1 needs you" adds the outline. "1 failed" turns the count `danger`, with no outline. ENDED stays collapsed, showing its count and Dismiss all.

**Gaps that are now covered**
1. **Unread output:** a dot on the row, per device, like the unseen dot on notes.
2. **Ended with work to take:** the sub-line reads "2 commits to apply", and the session stays out of ENDED until its work is applied or dismissed.
3. **A child in a different crew from its parent:** no indent; the sub-line reads "from {parent}".
4. **Unknown (host offline):** described in the table above. It sorts with failed.
5. **Making a crew without gestures:** a row's ··· menu (on hover; in long-press on phone) gets "Add to crew…". A crew header's menu gets "Add sessions…", which opens a checklist.

## 3. Session view
**Decision strip** (one pattern for anything that needs you to decide)
- **Placement:** the strip sits under the session header, one at a time. If more decisions are waiting, its label says so, for example "NEEDS YOU · 1 OF 2".
- **Style:** padding 10/20/12 and a bottom `border` line. Waiting decisions use a `danger` 8% tint with a `danger` label. Confirmations use `surface` with a `text3` label.
- **Layout:** the label column is 148px wide, so the title and its key–value rows start in the same column.
- **Delete request:** "{session} wants to delete {item}". It keeps WHO, "{AGENT} SAYS" (quoted word for word) and WHAT, then "If you approve: it moves to {vault}'s Trash and is kept for {n} days." Buttons: Later · Decline (ghost) · **Move to Trash** (primary, not danger, because it can be undone).
- **End tree:** "End {name} and the {n} agents it started?" It lists RUNNING and ALREADY ENDED, warns about unapplied commits, and has the "Let them report first (waits 30 s)" tick. The button is danger: "End {n} sessions".
- **These replace:**
  - the NEEDS YOU card above the terminal;
  - the delete modal on every device;
  - the End tree and Dismiss tree banners;
  - the AlertDialogs for Delete forever and Empty trash.

**Inspector icon rail**
- **Rail:** 48px, on the inspector's left edge, with icons for Context, Wrote, Agents, About, and Work (sandbox children only).
  - Icon buttons are 36px and `rControl`; the active one is `accent` on `accentSoft`.
  - Each icon has a tooltip with its name.
  - A count badge (15px) sits top-right: on Agents it's `danger` when a child needs you; on Wrote it's `surface2`/`text2`.
- **Panel:** 320px; it stays resizable 260–500 and isn't saved. Its header is the tab name (13/600) with ×.
- **Opening and closing:** clicking the active icon closes the panel. Closing leaves the rail visible, so the terminal goes full width and the counts stay visible.

**Agents tab** (replaces CHILDREN in About and the Activity tab)
- **"Started by this session":** one row per child. Each shows its state as a word tag (Needs you `danger`, Parked `text3`, Ended), its reason, and an Open or Work link, followed by "End all of them…".
- **"What it sent them":** the START, MESSAGE and END log, with the exact text quoted.
- **Round stats:** not shown (open question 5).

**About, slimmed down**
- Kept: Crew (Move to crew ▾), Role (Change label ▾), Starts agents (state and limits, with "Turn off · Change limits in Agents & hosts ›"), Ran as, Started by.
- Plugin problems become one line, such as "figma isn't loaded: it needs its address. Fix in Integrations ›".

**Header and terminal**
- **Header:** a "Starts agents ▾" chip replaces the switch in About.
- **Terminal:** shows the hint "Tab and ⇧Tab go to the agent · Esc, then Tab, leaves the terminal" in `text3`, 12px.

## 4. Launcher
**Default view** (480 wide on desktop; a sheet on phone)
- **From:** the context note chip.
- **Agent:** defaults to the default agent.
- **Where:** one field joining workspace and host, such as "storm on build-vm". Its Runs as line and the "another session is working here" warning sit under it, and New workspace… is an inline field, not a stacked dialog.
- **"This session can":** a row of chips: Writes to {vault} · Starts agents · Uses {n} integrations · Network: {host}'s policy. Tapping a chip opens its setting.
- **Then:** "More: model, permission mode, crew, role ›", and Launch.

**More**
- **Toggles:** the chips become toggles: Write to (with a vault picker), Start agents (up to 3 at once, 6 in a tree), and "Let the agents it starts write to {vault}".
- **Risk sentence:** the full network and integrations sentence is kept, as the constraints require.
- **Fields:**
  - Model offers suggestions (opus, sonnet, haiku) over the agent's default.
  - Permission mode notes "Claude Code and OpenCode only. Tab and ⇧Tab change it in the terminal."
  - Crew and Role.
- **No shifting rows:** rows never appear or disappear when the agent changes. A row the agent can't use is disabled, with the reason.

## 5. Trash and retention
- **Title and intro:** "Trash · {vault}". The intro says how long items are kept and points to Settings › Vaults.
- **Item cards:** show time left ("27 days left"), the "Deleted … by …" line and the agent's quoted reason. A checkbox allows multi-select; a selection bar offers Restore and Delete forever…. Restore needs no confirm.
- **Delete forever and Empty trash:** confirm in the decision strip, not a dialog.
- **Settings › Vaults:** each vault gets a "Trash keeps" setting (7 days, 30 days, 90 days, Never delete) under its row, with Open Trash ›. Retention moves out of the Trash page.

## 6. Settings
**Navigation**
- Order: This device · Terminal · Devices & access · STORM · Vaults · AI access · **Agents & hosts** · Integrations · Claude login · Storage · Connection · Advanced · About & health.
- No Beta tags in the navigation.

**Agents & hosts** (merges Hosts & default agent with Agents)
- **Header:** "＋ Enroll a host" (`accentSoft`) sits at the top right.
- **Grid:** a `surface` card with `rCard` radius. Columns are 170px for the host, then five equal agent columns. The headers name the agents; the default one carries a `default` tag in `accent`, and clicking another name makes it the default.
- **Host cell:** a neutral dot (online: filled `text2`; offline: `text3` ring), the name at 14/500, and "Runs as {account}" or the offline line in mono 11.
- **Agent cell:** one word on top, with a version or reason in mono 10.5 `text3` under it:

  | Word | When |
  |---|---|
  | Ready | Installed and signed in |
  | Sign in | Installed, not signed in (`accent`) |
  | Update | A newer version exists (`accent`) |
  | Install | Not installed (`accent`) |
  | Installing | Shows the step and a 3px bar |
  | Can't install | With the reason, e.g. "needs Node 22" |
  | — | The host is offline |

  Only words you can act on use `accent`.
- **A revoked host:** one dimmed row reading "It can't connect again.", with Re-enroll… and Remove.
- **Selected host** (shown below the grid, its row on `surface2`):
  - Name and state, with Rename and Revoke… (`danger`).
  - "Runs as: {account} · each session isolated".
  - Limits as inline steppers that never wrap mid-label: "Agents per session", "Per tree", "Sessions at once". They're followed by "Lowering a limit ends nothing that is running; it refuses new starts."
- **Cell popover** (300px, `surface2`):
  - "{Agent} on {host}", then the version and who installed it.
  - What the state means for sessions.
  - The single fix, e.g. **Sign in in a Shell**, which opens a Shell on that host with the login command typed in.
  - An install Storm didn't make says "Installed outside Storm, so Storm won't update or remove it."
- **No hosts:** one dashed empty card with "Enroll a host".
- **Copy rule:** host states are never green and never say trusted, secure, protected or sandboxed. "Runs as" is always shown.

**Integrations**
- Plugin suggestions use the same row style as connected services, under "Your plugins use these", with Connect and Dismiss.
- "Use your own OAuth app…" is a visible button beside "＋ Add integration".

**AI access copy (P25)**
- "Let them create, edit and delete" → sub-line "A deleted note goes to its vault's Trash."
- Storm agents → "You pick one vault per session. An agent's delete always waits for you, then goes to Trash."
- Remove "Storm has no trash" and "Agents never delete notes" everywhere.
- Delete footnotes use each vault's own retention, not a hard-coded 30 days.

## 7. Phone
- **Decision strip:** compact, under the session header, with a `danger` outline and tint; Review opens a sheet.
- **Request sheet:** WHO, SAYS and WHAT, with full-width buttons stacked from bottom to top: Move to Trash, Decline, Later.
- **Launcher:** the same three fields and chip row, as a sheet.
- **Needs-you count:** shown on the place bubble.
- **Inspector:** the phone still uses the session details sheet; there's no icon rail.

## Constraints (unchanged from the brief; violating one is a defect)
- **Orchestration only:** Storm has no sandbox toggle and no profile picker.
- **Deletes always ask,** even in Auto mode. Requests time out after 10 minutes. Esc means Later. Approved items go to Trash.
- **Spawning acts at once** and is logged. There are no approval cards.
- **Crews and roles never grant access.**
- **Device-local only:** unread, collapse state and inspector size.
- **Tokens only.** Accent = active, `danger` = attention, and amber = tags only (it isn't used in the sidebar any more).
- **One breakpoint (900px).** There's no drag on phone.

## Server/API work this needs
1. **Attention reasons** on sessions: blocked-on, out-of-usage reset time, last seen.
2. **A list of pending decisions per session**, in order, for "1 of 2".
3. **Remove and Re-enroll** for hosts.
4. **"Open a Shell with a command typed"** for Sign in.
5. **Unread output counts** per session.
6. **⌘K across all vaults:** search is per vault today. The palette itself is **not designed yet**.

## Open questions (ask before building)
1. **Delete requests on other devices:** they now get only the rail count, with no modal. Is that enough without push notifications?
2. **⇧⌘N for New session:** check that it doesn't clash with an existing shortcut.
3. **Round stats:** keep them behind "Show usage" in About, show them only in the Agents tab, or drop them?
4. **Agents & hosts at 900px:** agent columns are about 80px wide. Shrink the type, wrap the words, or scroll the grid sideways?
5. **The ⌘K palette:** what it contains (sessions, notes across vaults, actions, settings) and its empty and offline states.

## Acceptance checklist
- [ ] The app bar is on every desktop page: 44px, `bg`, one line under it. Buttons, back and forward, ⌘K and the health dot are placed per platform. The rail no longer has a health dot, and the Notes sidebar has no search field.
- [ ] The rail icon, sidebar heading and page title share one top line below the bar.
- [ ] The Agents sidebar header is a single row with ＋; there's no Overview row.
- [ ] Needs you is the only outlined state and the only thing counted on the rail. Failed has no outline.
- [ ] Sidebar rows have no attention tag pills. Reasons appear in the sub-line, and amber isn't used.
- [ ] The sort order matches §2, freezes while the pointer is in the sidebar, and trees move as one.
- [ ] Unread dot, "commits to apply", "from {parent}" and Add to crew… all work.
- [ ] Every decision uses the strip. No AlertDialog remains for delete requests, Delete forever or Empty trash, and banners no longer stack above the terminal.
- [ ] The inspector rail keeps its icons and counts when the panel is closed, and clicking the active icon toggles the panel.
- [ ] The Agents tab merges children and activity. About holds only the five items in §3.
- [ ] The launcher's default view has three fields and the chip row. More opens the rest. Rows never appear or disappear.
- [ ] Trash shows time left and multi-select. Retention is set in Vaults.
- [ ] Agents & hosts is one grid with a popover per cell, the default set from a column header, and Remove and Re-enroll for revoked hosts.
- [ ] The AI access copy is fixed, and footnotes use each vault's retention.
- [ ] All three themes pass contrast. Phone frames match §7.

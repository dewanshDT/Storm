---
title: Keyboard shortcuts
description: Desktop shortcuts in the macOS, Windows, Linux and web apps.
---

`⌘` on macOS, `Ctrl` elsewhere. They work on every screen — Notes, Agents
and Settings — at any window width; phones and tablets have none.

| Shortcut | Does |
|---|---|
| `⌘/Ctrl K`, `⌘/Ctrl P` | Open the command palette |
| `⌘/Ctrl ⇧ P` | Open the palette's commands |
| `⌘/Ctrl N` | New note in the open vault, from any screen |
| `⌘/Ctrl ⇧ N` | New folder in Notes; New session in Agents |
| `⌘/Ctrl \` | Show or hide the sidebar (wide windows) |
| `⌘/Ctrl E` | Switch between Read and Edit |
| `⌘/Ctrl S` | Save now |
| `⌘/Ctrl F` | Find in the note |
| `Esc` | Close the find bar or dialog |
| `⌘/Ctrl B` | Bold (Edit mode) |
| `⌘/Ctrl I` | Italic (Edit mode) |
| `⌘ [` on macOS, `Alt ←` on Windows and Linux | Back |
| `⌘ ]` on macOS, `Alt →` on Windows and Linux | Forward |

Back and forward retrace where this window has been, like the arrows at
the left of the app bar. In a text field on Windows and Linux, `Alt ←` and
`Alt →` keep moving to the line's start and end. On the web, use the
browser's own back and forward.

Undo, redo and select all are the system's own.

## The command palette

`⌘/Ctrl K`, or a click on the search field in the app bar, opens one
search over everything: note titles in every vault, agent sessions,
commands (with their shortcuts), Settings pages, folders, tags and hosts,
then **In note text**: the notes whose text matches, from every vault, with
the matching words highlighted. It works from every screen, with or without
a vault open. With nothing typed it shows what needs you, your recent notes
and a few commands. On a phone it opens full screen from the pill's Search
button, or from the search bubble in Agents and Settings.

| Key | Does |
|---|---|
| `↑` `↓` | Move through the results (they wrap) |
| `Enter` | Open the selected result, or run the command |
| `⇧ Enter` | New note named what you typed, in the open vault |
| `⌘ Enter` on macOS, `Ctrl Enter` elsewhere | On a note: start a session from it (opens the launcher) |
| `Esc` | Clear what you typed; again to close |

Start with a character to search one kind of thing:

| Prefix | Searches |
|---|---|
| `>` | Commands |
| `@` | Sessions |
| `#` | Tags in the open vault |
| `/` | Folders |
| `?` | Lists these prefixes |

Text search starts at two letters and only with no prefix. A note already
listed by its title is not listed again under In note text. If a vault
could not be searched in time, the palette says "Couldn't search {vault}";
if the server could not search at all, the other results stay and one line
says so.

Offline, the palette searches what this device holds, and says so; there
is no text search then.

## In the agent terminal

While the terminal has focus, keys go to the agent, so most of the app's
shortcuts do nothing there. The exceptions:

- On macOS, `⌘ K`, `⌘ P`, `⌘ ⇧ P`, `⌘ [` and `⌘ ]` still reach Storm; the
  terminal never uses `⌘`.
- On Windows and Linux, `Ctrl K` and `Ctrl P` stay the agent's. Use
  `Ctrl ⇧ K` to open the palette, or `Ctrl ⇧ P` for its commands, instead.

To leave the terminal, press `Esc`, then `Tab`.

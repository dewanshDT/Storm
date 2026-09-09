# Install — opencode

opencode maps onto the role set well: `mode: primary` for main-loop roles,
`mode: subagent` for workers, with automatic delegation when a task matches a
subagent's `description`.

**It is the only host of the three that can *enforce* a role's write
surface**, via its `permissions` rules. Elsewhere "the coder writes nothing in
the vault" is an instruction; here it is a denial.

| Role | `runs` | `mode` | File |
|---|---|---|---|
| Storm Architect | main-loop | `primary` | `storm-architect.md` |
| Storm Lead | main-loop | `primary` | `storm-lead.md` |
| Storm Coder | subagent | `subagent` | `storm-coder.md` |
| Storm Researcher | subagent | `subagent` | `storm-researcher.md` |
| Storm Reviewer | subagent | `subagent` | `storm-reviewer.md` |

**That table is the baseline, not the list.** Install an adapter for every note
your vault holds, not only these five.

## What to install, and where

Enumerate the vault rather than working from a hardcoded list — `get_vault` for
its folders, `search` for the note ids. Then map **folder to scope**: the
folder a note lives in decides where its adapter goes.

| Vault folder | Installs to | Loads for |
|---|---|---|
| `agents/` | `~/.config/opencode/agents/<name>.md` | every project |
| `skills/` | `~/.config/opencode/skills/<name>/SKILL.md` | every project |
| `projects/<p>/agents/` | `<repo>/.opencode/agents/<name>.md` | that repo only |
| `projects/<p>/skills/` | `<repo>/.opencode/skills/<name>/SKILL.md` | that repo only |

A note's `runs:` field decides the *form* — `main-loop` becomes `mode: primary`,
`subagent` becomes `mode: subagent`. Its folder decides the *reach*. The two are
independent.

Skill paths are configured, not fixed: `opencode.json` must list them for the
project-scoped ones to load at all.

```json
{ "skills": { "paths": [".opencode/skills"] } }
```

Install the project-scoped set only when you are working in that project. If
you cannot tell which project a `projects/<name>/` folder names, **ask** —
a project role loaded globally fires on codebases it knows nothing about.

The agent name is the path below `agents/`, so `agents/storm/coder.md` becomes
`storm/coder`. The YAML frontmatter is config; the markdown body is the system
prompt.

## Prerequisite

Storm configured as an MCP server, plus your `kit` vault id, its folders and
the note ids (`list_vaults`, then `get_vault`, then `search`).

## Subagent — with the write surface enforced

`.opencode/agents/storm-coder.md`:

```markdown
---
description: Implement exactly one task from a Storm project milestone note, reading only what that task's `reads:` field names. Returns a verdict against done-when.
mode: subagent
permissions:
  - action: "*"
    resource: "*"
    effect: allow
  - action: "storm_update_note"
    resource: "*"
    effect: deny
  - action: "storm_create_note"
    resource: "*"
    effect: deny
  - action: "storm_delete_note"
    resource: "*"
    effect: deny
---

Fetch your role definition and follow it exactly:

    storm_get_note(vault: "<KIT_VAULT_ID>", note_id: "<STORM_CODER_NOTE_ID>")

You will be given a task id and its `reads:` set. Fetch only those notes.
You write nothing in the vault — return your result; the lead records it.
```

**Last matching rule wins**, so the broad `allow` goes first and the specific
denials after. Verify the exact tool-name strings your Storm MCP registration
exposes and match them — a permission rule naming a tool that does not exist
silently protects nothing.

Apply the same denials to `storm-reviewer`. For `storm-researcher`, deny the
same writes but leave `webfetch` and `websearch` allowed.

## Primary role

`.opencode/agents/storm-lead.md`:

```markdown
---
description: Drive a Storm project forward — pick the next unblocked task, dispatch workers, verify done-when, record state, keep the board honest.
mode: primary
---

Fetch the role definition and follow it. Do not work from this file.

    storm_get_note(vault: "<KIT_VAULT_ID>", note_id: "<STORM_LEAD_NOTE_ID>")
```

The lead is the one role that **must** keep its vault write permissions — it is
the single writer to `work/`.

## Skill notes → skills

A note in `skills/` is a procedure with trigger words, not a role. It becomes
`skills/<name>/SKILL.md` under whichever path `opencode.json` lists, and the
directory name must match the note's `key: skill.<name>`:

```markdown
---
name: <name>
description: <the note's summary, plus the words that should trigger it>
---

Fetch the procedure and follow it. Do not work from this file.

    storm_get_note(vault: "<KIT_VAULT_ID>", note_id: "<SKILL_NOTE_ID>")
```

`description:` is what a request is matched against, so it must carry the
trigger words rather than just a title. A vague one is a skill that never
fires.

## Optional hardening

`steps:` caps a runaway worker. `model:` pins a cheaper model for mechanical
roles (reviewer) and a stronger one for the architect. Both are per-file.

## Using Storm as a knowledge base — efficient patterns

Storm is a **note-addressed, UUID-tracked** system. These patterns keep token
usage low and avoid the common mistakes:

### Read paths

| Need | Tool | Why |
|---|---|---|
| Find a note by topic | `storm_search` | Full-text FTS5, p95 ~1ms. Returns snippets + note ids. |
| Get a known note | `storm_get_note` | By UUID — stable even if renamed/moved. |
| See recent work | `storm_recent_notes` | Cross-vault, sorted by last opened. |
| Explore a vault's shape | `storm_get_vault` | Returns folders + note count. |
| Find related | `storm_get_related_notes` | Backlinks + shared tags, exact not semantic. |

### Vault structure to know

- **Notes are addressed by UUID, never path.** Renames/moves are metadata updates.
- **`kit` vault** holds reusable tooling (this layout spec, agent roles). Content projects live in separate vaults.
- **Wikilinks `[[...]]` resolve only within a vault.** Cross-vault references use plain text + UUID.
- **Frontmatter is never serialized** — Storm splices lines, preserving your YAML order, comments, quoting.
- **`modified:` is server-owned.** Clients must not write it; it's normalized out before merge.

### Search discipline

- **Search first, then `get_note` by UUID.** Two calls, but `search` returns snippets so you often don't need the full note.
- **Tag hierarchy:** `proj/storm` groups under `proj`. `storm_list_tags` returns all tags with counts.
- **Recents are cross-vault.** `storm_recent_notes` is the honest "what was I working on".

### Write discipline (lead only)

- **Read → edit → send whole body with `base_version`.** `update_note` replaces; partial sends destroy the rest.
- **`merged`/`conflict` = adopt server's text.** Re-read before editing again.
- **Create before delete.** No trash; a deleted note is gone immediately.
- **Milestone note is the source of truth.** BOARD/CHECKLIST are mirrors; regenerate them from the milestone note.

### MCP tool names (for permissions)

Exact strings as registered by the Storm MCP server:
- `storm_list_vaults`
- `storm_search`
- `storm_get_note`
- `storm_get_note_history`
- `storm_get_note_version`
- `storm_get_related_notes`
- `storm_get_vault`
- `storm_list_tags`
- `storm_recent_notes`
- `storm_create_note`
- `storm_update_note`
- `storm_delete_note`

Use these exact names in `permissions:` rules — a rule naming a non-existent tool silently protects nothing.

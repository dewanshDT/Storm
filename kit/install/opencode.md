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

Paths:

```text
project   .opencode/agents/<name>.md
global    ~/.config/opencode/agents/<name>.md
```

The agent name is the path below `agents/`, so `agents/storm/coder.md` becomes
`storm/coder`. The YAML frontmatter is config; the markdown body is the system
prompt.

## Prerequisite

Storm configured as an MCP server, plus your `kit` vault id and the note ids
(`list_vaults`, then `search`).

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

## Optional hardening

`steps:` caps a runaway worker. `model:` pins a cheaper model for mechanical
roles (reviewer) and a stronger one for the architect. Both are per-file.

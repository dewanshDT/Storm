# Install — Claude Code

Claude Code has both pieces the role set needs: **skills** for main-loop roles
that talk to the user, and **subagents** for workers that run in their own
context and can be spawned in parallel.

| Role | `runs` | Installs as | Path |
|---|---|---|---|
| Storm Architect | main-loop | skill | `~/.claude/skills/storm-architect/SKILL.md` |
| Storm Lead | main-loop | skill | `~/.claude/skills/storm-lead/SKILL.md` |
| Storm Coder | subagent | agent | `~/.claude/agents/storm-coder.md` |
| Storm Researcher | subagent | agent | `~/.claude/agents/storm-researcher.md` |
| Storm Reviewer | subagent | agent | `~/.claude/agents/storm-reviewer.md` |

Use `.claude/` inside a project instead of `~/.claude/` to scope them to one
repo.

## Prerequisite

The Storm MCP server must be configured, and you need your `kit` vault id:

```
mcp__storm__list_vaults        → find the vault named "kit"
mcp__storm__search(vault: <kit id>, query: "Storm Coder")   → note ids
```

Every adapter below is a **thin loader**. It carries no role content — it
fetches the note and follows it. That way editing the note in Storm changes
behaviour everywhere, with no reinstall.

## Main-loop role → skill

`~/.claude/skills/storm-lead/SKILL.md`:

```markdown
---
name: storm-lead
description: Drive a Storm project forward — pick the next unblocked task, dispatch workers, verify done-when, record state and keep the board honest. Use when the user wants to make progress on a Storm project, asks what is next, or asks to run/continue a milestone.
---

# Storm Lead — loader

Fetch the role definition and follow it. Do not work from this file.

    mcp__storm__get_note(vault: "<KIT_VAULT_ID>", note_id: "<STORM_LEAD_NOTE_ID>")

Also fetch, on first use in a session:

    <PROJECT>/INDEX · <PROJECT>/CONVENTIONS · <PROJECT>/work/BOARD

If the note cannot be fetched, say so and stop. Do not reconstruct the role
from memory.
```

Same shape for `storm-architect`, pointing at the Storm Architect note.

## Subagent role → agent

`~/.claude/agents/storm-coder.md`:

```markdown
---
name: storm-coder
description: Implement exactly one task from a Storm project milestone note, reading only what that task's `reads:` field names. Returns a verdict against done-when. Use when a specific task id needs implementing.
tools: Read, Edit, Write, Bash, Grep, Glob, mcp__storm__get_note
---

Fetch your role definition and follow it exactly:

    mcp__storm__get_note(vault: "<KIT_VAULT_ID>", note_id: "<STORM_CODER_NOTE_ID>")

You will be given a task id and its `reads:` set. Fetch only those notes.

**You write nothing in the vault.** Return your result; the lead records it.
```

Note the **`tools:` line is the enforcement**. `storm-coder` gets
`mcp__storm__get_note` but **not** `mcp__storm__update_note` or `create_note` —
so "writes nothing in the vault" is a capability boundary, not just an
instruction. Do the same for `storm-reviewer`.

> **Do not drop the `tools:` line.** Omitting it does not mean "no tools" — a
> subagent with no `tools:` field **inherits every tool the main agent has**,
> including the vault write tools. The allowlist is the only thing standing
> between a coder and `update_note`, and its absence fails open and silently.

`disallowedTools:` is the denylist counterpart if you would rather grant broadly
and subtract. It accepts the same MCP patterns — `mcp__storm__update_note` for
one tool, `mcp__storm__*` for a whole server. Either works; an allowlist is the
safer default because a newly added Storm tool is excluded by default rather
than included by default.

`storm-researcher` needs `WebSearch` and `WebFetch` added, and still no vault
write tools — it *proposes* spec edits rather than applying them.

`model:` is available per agent (`sonnet`, `opus`, `haiku`, `fable`, a full
model id, or `inherit`) if you want a cheaper model on the mechanical roles.

## Parallel work

The lead spawns coders with the Agent tool; several can run at once when their
tasks touch different areas and neither depends on the other. The lead writes
every result itself, serially — that single-writer rule is what makes the
fan-out safe.

## Caveat

Subagents **cannot ask the user questions** — `AskUserQuestion` is stripped
from a subagent's tool pool by design, even if you list it in `tools:`. That is
why architect and lead are skills rather than agents: both interview. Converting
either into a subagent does not degrade it, it breaks it, and the failure is
silent — the agent simply proceeds on an assumption instead of asking.

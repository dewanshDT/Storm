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

**That table is the baseline, not the list.** Install an adapter for every note
your vault holds, not only these five — see *What to install* below.

## What to install

Enumerate the vault; do not work from a hardcoded list:

```
mcp__storm__get_vault(vault: <kit id>)      → its folders
mcp__storm__search(vault: <kit id>, …)      → the note ids in each
```

Then map **folder to scope** — the folder a note lives in decides where its
adapter goes:

| Vault folder | Installs to | Loads for |
|---|---|---|
| `agents/` | `~/.claude/agents/<name>.md` | every project |
| `skills/` | `~/.claude/skills/<name>/SKILL.md` | every project |
| `projects/<p>/agents/` | `<repo>/.claude/agents/<name>.md` | that repo only |
| `projects/<p>/skills/` | `<repo>/.claude/skills/<name>/SKILL.md` | that repo only |

A note's `runs:` field decides the *form* — `main-loop` becomes a skill,
`subagent` becomes an agent. Its folder decides the *reach*. The two are
independent: a project-scoped main-loop role is a skill in `<repo>/.claude/`.

Install the project-scoped set only when you are working in that project. If
you cannot tell which project a `projects/<name>/` folder refers to, **ask**
rather than installing it globally — a project role loaded everywhere is worse
than one loaded nowhere, because it fires on codebases it knows nothing about.

## Prerequisite

The Storm MCP server must be configured, and you need your `kit` vault id:

```
mcp__storm__list_vaults        → find the vault named "kit"
mcp__storm__get_vault(vault: <kit id>)                      → folders
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

## Skill notes → skills

A note in `skills/` is not a role; it is a procedure with trigger words. It
becomes `~/.claude/skills/<name>/SKILL.md` — the directory name **is** the
skill name, and must match the note's `key: skill.<name>`:

```markdown
---
name: <name>
description: <the note's summary, plus the words that should trigger it>
---

Fetch the procedure and follow it. Do not work from this file.

    mcp__storm__get_note(vault: "<KIT_VAULT_ID>", note_id: "<SKILL_NOTE_ID>")
```

The `description:` is the only part that matters for *discovery* — it is what
Claude Code matches a request against, so it must carry the trigger words, not
just a title. A loader with a vague description is a skill that never fires.

## Caveat

Subagents **cannot ask the user questions** — `AskUserQuestion` is stripped
from a subagent's tool pool by design, even if you list it in `tools:`. That is
why architect and lead are skills rather than agents: both interview. Converting
either into a subagent does not degrade it, it breaks it, and the failure is
silent — the agent simply proceeds on an assumption instead of asking.

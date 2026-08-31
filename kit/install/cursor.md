# Install — cursor-agent

**Read this first: Cursor has no subagent concept.** `.cursor/rules/*.mdc`
files are context injection, not spawnable workers. There is nothing to
dispatch and nothing to run in parallel.

So the role set degrades honestly rather than pretending:

| Role | Elsewhere | On Cursor |
|---|---|---|
| Storm Architect | main loop | rule, invoked with `@storm-architect` |
| Storm Lead | main loop | rule — runs the loop **inline, one task at a time** |
| Storm Coder | subagent | rule, invoked manually per task |
| Storm Researcher | subagent | rule, invoked manually per spike |
| Storm Reviewer | subagent | rule, invoked manually on a diff |

**What is lost:** parallel coders, context isolation between roles, and
automatic delegation. What survives is the *discipline* — read sets, write
surfaces, `done-when` verification — which is most of the value.

The lead's loop is unchanged except that step 3 is "do the work" rather than
"dispatch a worker". Slower, not different. That fallback is written into the
Storm Lead note itself.

## Paths

```text
.cursor/rules/<name>.mdc     project rules, version-controlled
AGENTS.md                    project root; the CLI reads it as rules
```

The Cursor CLI reads `.cursor/rules`, plus `AGENTS.md` and `CLAUDE.md` at the
project root.

## Frontmatter → rule behaviour

| `alwaysApply` | `description` | `globs` | Behaviour |
|---|---|---|---|
| `true` | — | — | always in context |
| `false` | — | set | auto-attaches on matching files |
| `false` | set | — | agent decides from the description |
| `false` | — | — | manual only, via `@rule-name` |

Roles want the **last** row — manual. A role is something you invoke
deliberately, not ambient context. Auto-attaching all five would put every
role's instructions in every request and they would contradict each other.

## A role → a manual rule

`.cursor/rules/storm-coder.mdc`:

```markdown
---
description: Implement exactly one task from a Storm project milestone note
---

Fetch your role definition and follow it exactly:

    storm_get_note(vault: "<KIT_VAULT_ID>", note_id: "<STORM_CODER_NOTE_ID>")

You will be given a task id. Fetch only the notes its `reads:` field names.

You write nothing in the vault. Report your result against `done-when:`;
the lead records it.
```

No `globs`, no `alwaysApply` — manual invocation. Same shape for the other
four.

**The write surface is instruction-only here.** Cursor has no per-rule tool
permissions, so nothing stops a coder writing to the vault except the rule text
and your review. If enforcement matters to you, use opencode or Claude Code for
worker roles.

## One always-on rule is worth it

`.cursor/rules/storm-vault.mdc` with `alwaysApply: true`, carrying only the
non-negotiables — the milestone note is the source of truth for task state,
`update_note` replaces the whole body so always send `base_version`, never put
progress in a spec note. Short, and it prevents the mistakes that are expensive
to undo.

Do **not** put role definitions in it. Those stay manual.

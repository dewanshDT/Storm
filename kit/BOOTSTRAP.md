# Bootstrap

Paste this whole file into your coding agent. It installs the Storm agent roles
for whichever host you are running.

---

You are setting up **Storm Agents** — a set of roles for working on projects
stored in Storm. Work through these steps in order and report what you did.

## 1. Check the connection

Call `list_vaults`. If Storm is not configured as an MCP server, stop and tell
me how to configure it for this host — do not continue.

Find the vault named **`kit`**. Your Storm server creates and seeds it on first
start.

**If `kit` does not exist**, the server predates automatic seeding, or it was
deleted. Tell me, and ask whether to create it — creating a vault needs a
session credential, so it is a step I take in the Storm app, not one you can do
with an MCP key. Once it exists, seed it from `kit/vault/` in the Storm
repository, preserving the folder structure.

## 2. Read what is there

Fetch and read, in this order:

1. `README` in the `kit` vault — what the vault is and what the roles are
2. `Project Architecture Guidelines` — the layout spec every role operates on

Record the note ids as you go; you need them in step 4.

## 3. Identify the host

Which agent am I running you in?

- **Claude Code** → follow `kit/install/claude-code.md`
- **opencode** → follow `kit/install/opencode.md`
- **cursor-agent** → follow `kit/install/cursor.md`
- **something else** → tell me what capabilities it has: can it define
  subagents that run in their own context? can it restrict a role's tools?
  Then adapt the closest guide and say which compromises you made.

If you are not certain which host you are in, **ask** rather than guessing —
installing to the wrong paths leaves files that never load and no error.

## 4. Install the five roles

From the guide for your host, create the adapter for each role:

```text
Storm Architect     main loop
Storm Lead          main loop
Storm Coder         subagent
Storm Researcher    subagent
Storm Reviewer      subagent
```

Three rules for every adapter:

- **Keep it thin.** It fetches the role note and follows it. It does not
  summarise the role — the moment it does, you have two definitions drifting.
- **Substitute the real ids.** Every `<KIT_VAULT_ID>` and `<..._NOTE_ID>`
  placeholder becomes an actual uuid from step 2.
- **Enforce the write surface where the host can.** Coder and reviewer must not
  hold vault-write tools. On Claude Code, omit them from `tools:`. On opencode,
  deny them in `permissions:`. On Cursor it is instruction-only — say so in
  your report rather than implying it is enforced.

## 5. Verify, then report

Confirm each adapter file exists at the right path and that its ids resolve —
fetch one note through one of them.

Then tell me:

- which host you detected, and how
- the files you created, with paths
- the vault and note ids you wired in
- **which roles are enforced vs instruction-only on this host**
- anything you could not do, and what I need to do myself

Do not report success for a role whose adapter you could not verify.

## Then

Start a project with the architect: give it your architecture documents and it
will interview you, expose gaps, and lay out the vault. Or point the lead at an
existing Storm project and ask what is next.

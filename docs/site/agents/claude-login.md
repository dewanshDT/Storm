---
title: Claude login sync
description: Optional. Keep your own Claude subscription login in Storm and give it to the Runtime Hosts you tick, so you sign in once.
---

Storm can keep **your own** Claude subscription login and give it to the
Runtime Hosts you choose, so you sign in once instead of on every machine.

How it works: you run `claude setup-token` in a terminal outside Storm,
approve it in your browser, and paste the token into **Settings ▸ Claude
login**. Storm stores it encrypted on your Storm server. When a Claude Code
session starts on a host you have ticked, Storm hands the token to that
session. Claude Code then talks to Anthropic directly; Storm never sees your
prompts or Anthropic's replies. No host gets the token until you tick it.

**This is for one person, on their own machines, for their own work.**
Storm has a single user. Do not use this feature to share a Claude account,
to give other people access to your plan, or on machines you do not control.

**You are responsible for following Anthropic's terms.** Anthropic's
documentation says third-party developers "may not collect, store, or
intermediate Claude.ai credentials or session tokens". Storm stores your
token at your request, on your server. If you are not comfortable with that
risk to your account, don't turn this on: log in on each host instead (see
[Runtime Hosts](/docs/agents/hosts/)).

Know what you are turning on:

- Every agent session on a ticked host can read the token.
- It is one token for all ticked hosts, valid for a year. Revoking it signs
  out every host at once.
- Your Storm server and its backups hold the token, encrypted, next to the
  key that opens it. Protect them like the token itself.
- Removing it in Storm stops new sessions from getting it. It does not
  revoke it, and sessions that are already running keep it until they end.

To revoke the token, go to your Claude account settings on claude.ai and
remove it there, then end any running sessions on ticked hosts. Then remove
it in **Settings ▸ Claude login**. To renew it before it expires, mint a new
token, use **Replace token**, and revoke the old one.

## What a ticked host does

- Only `claude-code` sessions get it; OpenCode, shell and custom agents never
  do.
- The host fetches the token for each session as it starts, keeps it in
  memory only, and passes it to Claude Code as `CLAUDE_CODE_OAUTH_TOKEN`. It
  is never written to a file, a command line or a log.
- On a ticked host the synced login wins over the host's own: the env file's
  credential variables (`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, a
  different `CLAUDE_CODE_OAUTH_TOKEN`, the cloud switches) are not applied to
  that session; its other variables still are. Untick the host and it goes
  back to its own login on the next session.
- The session's settings pin the API address and the proxy and certificate
  variables, so a workspace's `.claude/settings.json` cannot send the token
  anywhere else.
- If the host cannot fetch the token, the session fails to start with "could
  not fetch the synced Claude login". It never starts on the host's own login
  instead.
- A host running an older `storm-runtime` cannot receive it: its sessions use
  the host's own login, and the app says "update storm-runtime".

**Known gap:** a workspace whose `.claude/settings.json` sets its own
`CLAUDE_CODE_OAUTH_TOKEN` runs on that token instead of yours. Your token is
not sent anywhere it should not go; the session is just not on your plan.

## When it stops working

Storm cannot check the token, revoke it or see that Anthropic has. If
sessions on synced hosts say the login is revoked or expired, mint a new
token, use **Replace token**, and revoke the old one. Settings warns 30 days
and 7 days before the year is up; after that the token is not delivered, and
sessions on ticked hosts use their own login with a notice.

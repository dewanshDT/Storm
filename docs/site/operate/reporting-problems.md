---
title: Report a problem
description: What's in a report, what's never sent, and how it reaches the Storm project.
---

Storm has no telemetry: nothing about your server is ever seen unless you
choose to send it. **Report a problem** is that one path, and it costs no
GitHub account and no copying logs by hand.

## Opening it

- **⌘K** (Ctrl+K), then **Report a problem**.
- **Settings › About & health › Report a problem**.
- **Report this** on an error snackbar — on an error, such as an agent that
  failed to resume, a sync notice about a recovered edit, or a host that
  couldn't be removed — this one opens with the failing action, the screen
  and the error already filled in.

The sheet has:

- a text box, **What happened?**;
- a kind — **Bug**, **Idea** or **Other** (Bug by default);
- a screenshot of the app at that moment, taken automatically; remove it if
  you don't want to send it;
- an optional reply email;
- **Include diagnostics**, ticked by default.

Press **Send**, and a snackbar shows the report's id.

## Exactly what is sent

Your text, the kind, your email if you gave one, and the screenshot if you
kept it. **With "Include diagnostics" on**, the report also carries:

- the version of every component (this app, your server, each Runtime Host);
- OS and architecture;
- the execution profile (whether a host isolates its sessions);
- host and relay state — how many vaults, relays and hosts you have, and
  which hosts are online;
- the last ~300 log lines from your server and from each host, run through
  the same redaction as everything else here;
- the app's own connection log (already scrubbed) — up to 200 lines;
- the screen you were on and your last ~20 actions.

**Nothing here is a guess**: press **See what's sent** and the sheet shows
the exact bytes a send would forward — because that is what a send
*does* forward, byte for byte (`POST /v1/feedback/preview` builds it; `POST
/v1/feedback` sends exactly that preview, not a fresh copy).

## What is never sent

- note content;
- vault and note names;
- tokens and keys;
- enrollment and pairing strings;
- hostnames — your server's own name and each host's name, never a bare
  string a bundle could carry; a host in a bundle is "host-1", "host-2", and
  so on, never its name;
- home paths, masked to `~/`;
- email addresses in logs, masked to `<email>` — the only email Storm ever
  sends is the reply address you typed into the sheet yourself.

LAN and public IP addresses are masked too (`<lan>`, `<ip>`), as is anything
shaped like a token, a JWT, a `Bearer` header, or a `.md`/image path. Your
written text is the one field sent as you typed it — it's yours, and you see
it before sending.

## Where it goes

Pressing Send needs your server to be reachable — that leg isn't queued, so
the app must reach it to hand the report over. Once your server has it, the
server queues it durably (a report surviving a restart or a dead network) and
retries delivery to the intake with backoff from 1 second up to an hour,
giving up after 7 days. From there it reaches the Storm project's own intake
(the server's default), where it is kept for up to 30 days unless it has
already been picked up and filed.

Replies, if you leave an email, come back by email only.

## Turning it off

Feedback is on by default. **Settings › About & health › Send problem
reports** turns it off: with it off, the report sheet says reporting is
turned off and refuses to send.

Or, from a script:

```sh
curl -X PUT http://127.0.0.1:8484/v1/config/feedback \
  -H "Authorization: Bearer <a session token>" \
  -H "Content-Type: application/json" \
  -d '{"enabled": false}'
```

## Changing where reports go

There's no Settings field for the intake address yet. A server operator can
point it at another intake with the same endpoint:

```sh
curl -X PUT http://127.0.0.1:8484/v1/config/feedback \
  -H "Authorization: Bearer <a session token>" \
  -H "Content-Type: application/json" \
  -d '{"intake_url": "https://your-own-intake.example.com"}'
```

## From the command line

A headless machine, or a report you'd rather build without the app, can use
the diagnostics bundle directly: `storm-server diag` on the server and
`storm-runtime diag` on a host print the same bundle, redacted the same way.
See [storm-server](/docs/reference/cli/storm-server/#diag) and
[storm-runtime](/docs/reference/cli/storm-runtime/#diag).

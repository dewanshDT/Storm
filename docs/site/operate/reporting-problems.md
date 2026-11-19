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
- **Report this** on an error snackbar (an agent that failed to start, a sync
  error, a host that went offline) — this one opens with the failing action,
  the screen and the error already filled in.

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
- tokens, keys and reply-email addresses other than the one you typed;
- enrollment and pairing strings;
- hostnames — your server's own name and each host's name, never a bare
  string a bundle could carry; a host in a bundle is "host-1", "host-2", and
  so on, never its name;
- home paths, masked to `~/`.

LAN and public IP addresses are masked too (`<lan>`, `<ip>`), as is anything
shaped like a token, a JWT, a `Bearer` header, or a `.md`/image path. Your
written text is the one field sent as you typed it — it's yours, and you see
it before sending.

## Where it goes

A send goes to your own server first. The server queues it durably (a report
surviving a restart or a dead network) and retries delivery with backoff from
1 second up to an hour, giving up after 7 days. From there it reaches the
Storm project's own intake, where it is kept for up to 30 days unless it has
already been picked up and filed.

Turning **Include diagnostics** off, or your server being offline when you
press Send, changes none of this ordering — the report still queues on your
server and sends once it can.

Replies, if you leave an email, come back by email only.

## Turning it off, or changing where reports go

Feedback is on by default. A server operator can turn it off, or point it at
another intake, with:

```sh
curl -X PUT http://127.0.0.1:8484/v1/config/feedback \
  -H "Authorization: Bearer <a session token>" \
  -H "Content-Type: application/json" \
  -d '{"enabled": false}'
```

`enabled: false` means nothing leaves your server: the sheet says so and Send
is disabled. `intake_url` takes a report somewhere other than the Storm
project's own intake (`https://feedback.storm.dev` by default):

```sh
curl -X PUT http://127.0.0.1:8484/v1/config/feedback \
  -H "Authorization: Bearer <a session token>" \
  -H "Content-Type: application/json" \
  -d '{"intake_url": "https://your-own-intake.example.com"}'
```

There is no Settings page for either setting yet — this `PUT` (session tier,
so any paired device's own token works) is the only way to change them today.

## From the command line

A headless machine, or a report you'd rather build without the app, can use
the diagnostics bundle directly: `storm-server diag` on the server and
`storm-runtime diag` on a host print the same bundle, redacted the same way.
See [storm-server](/docs/reference/cli/storm-server/#diag) and
[storm-runtime](/docs/reference/cli/storm-runtime/#diag).

---
title: Notifications
description: What Storm notifies you about, where those settings live, and how to get them on your phone with ntfy.
---

Storm keeps a shared inbox of what needs your attention — a session
needing you, a delete request, a finished turn, an error, a usage limit, a
host going offline — plus anything an agent sends you on purpose. Every
device sees the same inbox and the same read state.

## Where it shows up

- **In the app**, always: a toast while you're looking at Storm, and the
  bell's own list otherwise.
- **As a desktop notification** on macOS, Windows and Linux, while Storm
  is running (even hidden) — see **Settings › Notifications › This
  device**.
- **On your phone**, only if you set up [ntfy](#on-your-phone-ntfy) below.
  Without it, a notice on Android or iOS only ever shows while the app is
  open — see [Android and iOS](#android-and-ios) for why.

**Settings › Notifications** holds what Storm tells you about (each event
has its own on/off switch and, for some, a delay), quiet hours, a pause,
agent messages' own limits, and the inbox's retention. These are
account-wide: changing them on one device changes them everywhere.

## Android and iOS

Storm does not yet raise a system notification on Android or iOS — there
is no OS permission to grant, because nothing asks for one. If your
phone's own notification settings show Storm as "on", that is Android's
default for an app that has never requested the permission; it does not
mean a notice is ever delivered outside the app. Settings ›
Notifications' **This device** section says this plainly on a phone, and
points at ntfy, below, which is the way to actually reach a closed phone
today.

## On your phone (ntfy)

[ntfy](https://ntfy.sh) is a small, independent push service: you publish
to a "topic" (a plain URL), and anything subscribed to it — the ntfy
phone app, in this case — gets it, even if Storm's own app is closed.
Storm can publish there too, from **Settings › Notifications › ntfy
(optional)**:

1. Install the ntfy app on your [Android](https://play.google.com/store/apps/details?id=io.heckel.ntfy)
   or [iPhone](https://apps.apple.com/us/app/ntfy/id1625396347).
2. In Storm, tap **Generate private topic** (or paste the URL of a topic
   you already use). This fills in a topic on the public `ntfy.sh`
   server, named so a stranger can't guess it — but a topic has no
   password of its own, so treat the URL itself as something not to share.
3. Open the same topic in the ntfy app (scan or paste the URL) to
   subscribe.
4. Tap **Test** to send one through the real path and confirm it arrived.

A **Token** field is there for a topic that requires one (your own ntfy
server, or a reserved topic on `ntfy.sh`) — leave it blank otherwise.
Storm never shows a token back to you once it's saved; the field just
says "Set".

### What's sent, and how much

Every notice that would otherwise interrupt you (not one held by quiet
hours, a pause or a mute) is also published to your topic, whether or not
any device is open — that's the point. **Show** controls how much of it
leaves this server, independent of your other devices' own preview
settings:

| Show | What reaches ntfy |
|---|---|
| **Session name only** (default) | A short Storm-written line, e.g. "tidier needs you" — the session's name and what happened, never an agent's own words. |
| **"Storm" only** | Just "Storm · needs you" — not even the session's name. |
| **Full text** | The same as above, plus — only for a message an agent sent on purpose, or one forwarded from its terminal — that message's actual text. |

**ntfy.sh stores what you publish as plain text**, and sees your Storm
server's address and the time of every notice. For most people this is a
sensible trade for notifications that still arrive when Storm is closed;
if you'd rather not depend on a third party for any of it, you can run
your own [ntfy server](https://docs.ntfy.sh/install/) and generate or
paste a topic on that instead — nothing here is specific to `ntfy.sh`,
only its default.

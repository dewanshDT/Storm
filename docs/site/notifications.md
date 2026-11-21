---
title: Notifications
description: What Storm notifies you about, where those settings live, and how to get them on your phone — UnifiedPush on Android, ntfy everywhere else.
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
- **On Android, with the app closed**, once you set up push below — see
  [Android](#android).
- **On any phone**, only if you set up [ntfy](#ntfy-optional) below.
  Without it, a notice on iOS only ever shows while the app is open, and
  the same was true on Android before push was set up.

**Settings › Notifications** holds what Storm tells you about (each event
has its own on/off switch and, for some, a delay), quiet hours, a pause,
agent messages' own limits, and the inbox's retention. These are
account-wide: changing them on one device changes them everywhere.

## Android

Android has no equivalent of "keep a background connection open" — the
app's connection to Storm closes as soon as it's backgrounded, same as it
always has. To still hear about a finished agent or a decision waiting on
you, Storm delivers through **UnifiedPush**, an open protocol that lets
you pick which app carries the push instead of tying every app to one
company's service.

### Set it up

1. Install **[ntfy](https://ntfy.sh)** — the distributor Storm tells you
   about — from the [Play
   Store](https://play.google.com/store/apps/details?id=io.heckel.ntfy) or
   [F-Droid](https://f-droid.org/packages/io.heckel.ntfy/). Open it once
   after installing (it does nothing else from here — Storm talks to it in
   the background from then on).
2. In Storm, open Settings → **Notifications** → **This device**, and tap
   **Enable notifications**.
3. Android asks for permission to show notifications — allow it.
4. Storm registers with ntfy and shows **Push via ntfy · working**.

If no push app is installed, the row instead says to install ntfy, with
links to both stores. If you'd rather use a different UnifiedPush
distributor (NextPush, or your own self-hosted one), install and open it
*before* tapping Enable — Storm uses whichever one it finds.

This is separate from, and independent of, the account-wide **ntfy
(optional)** sink below: that one works on any phone (including iOS) and
needs no per-device setup, at the cost of a plainer "session name only"
message by default; the device-level push above is richer (your own Show
and per-class switches) and end-to-end encrypted to this device alone. You
can turn on neither, either, or both.

### What it sees

The message that reaches ntfy (or whichever distributor you chose) is
encrypted end to end before it leaves the server — the distributor and its
own server see only size, timing and your device's push endpoint, never the
text. What that text itself contains is your own **Show** setting: full
text, the session's name only, or just "Storm". The same **Send to this
device** switches that control in-app toasts also control what gets pushed.

### Delivery

The **Delivery** row tells you the truth: a working endpoint, how many
recent attempts failed, or "Only inside the app while it's open" if push
isn't set up. **Disable** unregisters from the distributor and removes the
registration from the server — nothing is pushed to this device again until
you enable it once more.

### Manual check

To prove it end to end on a real phone: install ntfy, update Storm, tap
**Enable notifications**, grant the permission, then from another device
(or an agent) trigger something that needs you — swipe Storm away first. The
notification should still arrive.

## iOS

Storm does not yet raise a system notification on iOS — there is no OS
permission to grant, because nothing asks for one. The way to reach a
closed iPhone today is the account-wide **ntfy (optional)** sink below.

## ntfy (optional)

[ntfy](https://ntfy.sh) is a small, independent push service: you publish
to a "topic" (a plain URL), and anything subscribed to it — the ntfy
phone app, in this case — gets it, even if Storm's own app is closed. It
works on both Android and iOS, and needs no per-device registration — one
topic, every subscribed device. Storm can publish there too, from
**Settings › Notifications › ntfy (optional)**:

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
only its default. Android's own UnifiedPush push (above) is already
end-to-end encrypted regardless of which distributor you choose.

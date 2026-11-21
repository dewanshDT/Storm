---
title: Notifications
description: Get told when an agent needs you, finishes or hits an error — in the app, on the desktop, and on Android even with Storm closed.
---

Storm notifies you about your agents: one needs you, one finished its turn,
one hit an error, a delete request is waiting, a host went offline. Agents
can also send you a message directly. Settings → **Notifications** holds
every setting, with **This device**'s own settings at the top.

## This device

What "This device" can do depends on the platform:

| Platform | While Storm is open | While it's backgrounded or closed |
|---|---|---|
| macOS, Windows, Linux | In-app toasts | A real OS notification, as long as Storm is still running |
| Web | In-app toasts; an OS notification once you tap **Enable notifications** (HTTPS only) | Nothing — closing the tab closes the connection |
| Android | In-app toasts | A real notification, once you set up push below |

## Android

Android has no equivalent of "keep a background connection open" — the app's
connection to Storm closes as soon as it's backgrounded, same as it always
has. To still hear about a finished agent or a decision waiting on you, Storm
delivers through **UnifiedPush**, an open protocol that lets you pick which
app carries the push instead of tying every app to one company's service.

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

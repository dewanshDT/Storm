/// The web notifications round's actual DOM calls — the
/// [Notification](https://developer.mozilla.org/en-US/docs/Web/API/Notification)
/// interface used directly from the page, never through a service worker
/// (the task's own constraint): this works only while the tab's own
/// JavaScript context is still running, which `agent_events.dart`'s
/// `_keepsStreamWhileHidden` is what keeps true while hidden.
library;

import 'dart:js_interop';

import 'package:web/web.dart' as web;

/// Notice id → the open `Notification` it is shown as, so a read, a
/// removal or "clear the inbox" can close exactly the one the server told
/// it to (`rawClose`) rather than every open notification.
final _open = <int, web.Notification>{};

bool rawIsSecureContext() => web.window.isSecureContext;

/// `'granted' | 'denied' | 'default'` on a secure context (the only three
/// values `Notification.permission` itself ever reports); `'unsupported'`
/// on an insecure origin, where the constructor would throw regardless of
/// what the browser last answered — `window.isSecureContext` is checked
/// here too, not only by the caller, so a stale answer from a page that
/// changed origin mid-session is never trusted.
String rawPermission() {
  if (!web.window.isSecureContext) return 'unsupported';
  return web.Notification.permission;
}

/// Must only ever be called from inside a click handler (the Settings
/// page's "Enable notifications" button) — browsers refuse (Chrome) or
/// silently ignore (Firefox) a request made outside a user gesture, and
/// some treat a gesture-less request as a permanent denial. Nothing here
/// enforces that; the caller does.
Future<String> rawRequestPermission() async {
  if (!web.window.isSecureContext) return 'unsupported';
  final result = await web.Notification.requestPermission().toDart;
  return result.toDart;
}

/// Shows (or, by `tag: '$id'`, replaces) one notification. Silently does
/// nothing on any failure — permission not granted, an unsupported
/// browser, a thrown constructor — the same best-effort contract the
/// native plugins already keep.
void rawShow({
  required int id,
  required String title,
  String? body,
  String? icon,
  String? payload,
  required bool silent,
  required void Function(String? payload) onClick,
}) {
  try {
    final notification = web.Notification(
      title,
      web.NotificationOptions(
        tag: '$id',
        body: body ?? '',
        icon: icon ?? '',
        silent: silent,
      ),
    );
    notification.onclick = (web.Event _) {
      // The one reliable place `Window.focus()` works across browsers:
      // inside a trusted event a notification click itself raised.
      web.window.focus();
      onClick(payload);
      notification.close();
    }.toJS;
    _open[id] = notification;
  } catch (_) {}
}

void rawClose(int id) {
  _open.remove(id)?.close();
}

void rawCloseAll() {
  for (final notification in _open.values) {
    notification.close();
  }
  _open.clear();
}

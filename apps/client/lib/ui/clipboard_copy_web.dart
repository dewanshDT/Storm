import 'dart:js_interop';

import 'package:web/web.dart' as web;

/// Copies `text`, and says whether it worked.
///
/// Two strategies, in order, because Storm's web client is routinely served
/// from a plain-HTTP LAN address where the first one does not exist.
Future<bool> copyToClipboard(String text) async {
  // **The modern API is secure-context only.** `http://192.168.x.x:8585` is
  // not one, so `navigator.clipboard` is undefined there — reading it at all
  // is guarded rather than caught, because an undefined property access is a
  // different failure from a rejected promise.
  if (web.window.isSecureContext) {
    try {
      await web.window.navigator.clipboard.writeText(text).toDart;
      return true;
    } catch (_) {
      // Permission refused, or a browser that gates it behind a user gesture
      // we are no longer inside. Fall through — `execCommand` may still work.
    }
  }
  return _copyByExecCommand(text);
}

/// The pre-Clipboard-API way: put the text in a textarea, select it, and ask
/// the document to copy the selection.
///
/// Deprecated, unpleasant, and the only thing available over plain HTTP. It
/// must run inside the click that triggered it, which is why the caller does
/// no `await` before reaching here.
bool _copyByExecCommand(String text) {
  final area = web.HTMLTextAreaElement();
  area.value = text;
  area.setAttribute('readonly', 'true');
  // Off-screen rather than `display:none` or `hidden` — an element the
  // browser considers invisible cannot hold a selection, and the copy is a
  // copy *of a selection*.
  area.style
    ..position = 'fixed'
    ..top = '0'
    ..left = '0'
    ..width = '1px'
    ..height = '1px'
    ..padding = '0'
    ..border = 'none'
    ..outline = 'none'
    ..boxShadow = 'none'
    ..background = 'transparent'
    ..opacity = '0';

  web.document.body?.append(area);
  try {
    area.focus();
    area.select();
    return web.document.execCommand('copy');
  } catch (_) {
    return false;
  } finally {
    area.remove();
  }
}

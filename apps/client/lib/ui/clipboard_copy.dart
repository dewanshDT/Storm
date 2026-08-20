/// Copying to the clipboard, in a way that survives a plain-HTTP LAN origin.
///
/// **`Clipboard.setData` is not enough on the web.** `navigator.clipboard` is
/// a *secure context* API: the browser does not expose it at all over plain
/// `http://` to an IP address — which is exactly how Storm is served on a LAN,
/// and therefore exactly how the person copying a freshly minted MCP key
/// reaches it. The copy silently did nothing, and the button cheerfully said
/// it had worked.
///
/// So: use the real API where it exists, fall back to the old
/// `document.execCommand('copy')` where it does not, and **return whether it
/// actually happened** so the caller can tell the truth either way.
///
/// Split by conditional import because `package:web` exists only on the web,
/// the same way [web_bootstrap] is.
library;

export 'clipboard_copy_stub.dart'
    if (dart.library.js_interop) 'clipboard_copy_web.dart';

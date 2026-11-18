/// What a browser `paste` event over the page becomes.
enum BrowserPaste {
  /// Staged as images; the browser inserts nothing.
  images,

  /// Sent as the terminal's paste, bracketed when the program asked; the
  /// browser inserts nothing.
  text,

  /// Left to the browser.
  browser,
}

/// Images win, as before. Text is the terminal's only while it has focus, so
/// a paste into any other field is that field's.
BrowserPaste routeBrowserPaste({
  required bool images,
  required String? text,
  required bool focused,
}) {
  if (images) return BrowserPaste.images;
  if (focused && text != null && text.isNotEmpty) return BrowserPaste.text;
  return BrowserPaste.browser;
}

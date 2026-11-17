import 'dart:js_interop';

import 'package:web/web.dart' as web;

import 'image_paste.dart';
import 'paste_route.dart';

/// The browser's `paste` events, routed by [routeBrowserPaste].
void Function() listenForPaste({
  required bool Function() focused,
  required void Function(List<PastedImage>) onImages,
  required void Function(String) onText,
}) {
  final listener = ((web.ClipboardEvent e) {
    final data = e.clipboardData;
    if (data == null) return;
    final files = data.files;
    final picked = [
      for (var i = 0; i < files.length; i++) ?files.item(i),
    ].where((f) => f.type.startsWith('image/')).toList();
    final text = data.getData('text/plain');
    switch (routeBrowserPaste(
      images: picked.isNotEmpty,
      text: text,
      focused: focused(),
    )) {
      case BrowserPaste.browser:
        return;
      case BrowserPaste.text:
        e.preventDefault();
        onText(text);
      case BrowserPaste.images:
        e.preventDefault();
        () async {
          final images = <PastedImage>[];
          for (final f in picked) {
            final buffer = await f.arrayBuffer().toDart;
            final image = await normalizeImage(buffer.toDart.asUint8List());
            if (image != null) images.add(image);
          }
          if (images.isNotEmpty) onImages(images);
        }();
    }
  }).toJS;
  web.window.addEventListener('paste', listener, true.toJS);
  return () => web.window.removeEventListener('paste', listener, true.toJS);
}

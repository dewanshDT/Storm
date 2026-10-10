import 'dart:js_interop';

import 'package:web/web.dart' as web;

import 'image_paste.dart';

/// Images in the browser's `paste` events; text pastes pass untouched.
void Function() listenForPastedImages(
  void Function(List<PastedImage>) onImages,
) {
  final listener = ((web.ClipboardEvent e) {
    final files = e.clipboardData?.files;
    if (files == null) return;
    final picked = [
      for (var i = 0; i < files.length; i++) ?files.item(i),
    ].where((f) => f.type.startsWith('image/')).toList();
    if (picked.isEmpty) return;
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
  }).toJS;
  web.window.addEventListener('paste', listener, true.toJS);
  return () => web.window.removeEventListener('paste', listener, true.toJS);
}

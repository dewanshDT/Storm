import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// An image on its way into a session (D15 AM45).
@immutable
class PastedImage {
  const PastedImage(this.bytes, this.mime);

  final Uint8List bytes;
  final String mime;
}

/// The type the server stages an image as, from its own bytes.
String? sniffImageMime(Uint8List b) {
  bool starts(List<int> magic, [int at = 0]) =>
      b.length >= at + magic.length &&
      Iterable<int>.generate(magic.length).every((i) => b[at + i] == magic[i]);
  if (starts(const [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])) {
    return 'image/png';
  }
  if (starts(const [0xFF, 0xD8, 0xFF])) return 'image/jpeg';
  if (starts('GIF87a'.codeUnits) || starts('GIF89a'.codeUnits)) {
    return 'image/gif';
  }
  if (starts('RIFF'.codeUnits) && starts('WEBP'.codeUnits, 8)) {
    return 'image/webp';
  }
  return null;
}

/// [bytes] as a type the server stages: as they are when they already are
/// one, else re-encoded as PNG. Null when they are not an image at all.
Future<PastedImage?> normalizeImage(Uint8List bytes) async {
  final mime = sniffImageMime(bytes);
  if (mime != null) return PastedImage(bytes, mime);
  try {
    final codec = await ui.instantiateImageCodec(bytes);
    final frame = await codec.getNextFrame();
    final png = await frame.image.toByteData(format: ui.ImageByteFormat.png);
    frame.image.dispose();
    codec.dispose();
    return png == null
        ? null
        : PastedImage(png.buffer.asUint8List(), 'image/png');
  } catch (_) {
    return null;
  }
}

const _clipboard = MethodChannel('storm/clipboard');

/// The image on the system clipboard, if there is one and this platform can
/// read it. The web has no such read: it takes images from `paste` events.
Future<PastedImage?> readClipboardImage() async {
  if (kIsWeb) return null;
  try {
    final bytes = await _clipboard.invokeMethod<Uint8List>('readImage');
    return bytes == null || bytes.isEmpty ? null : normalizeImage(bytes);
  } on MissingPluginException {
    return null;
  } on PlatformException {
    return null;
  }
}

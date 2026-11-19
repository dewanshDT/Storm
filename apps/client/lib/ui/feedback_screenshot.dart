/// Captures the whole app for a report (decision 162a), from the
/// [rootScreenshotKey] boundary [main.dart] wraps the app root in.
library;

import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';

import '../state/app_state.dart' show rootScreenshotKey;

/// A PNG of the whole app, downscaled as needed to stay at or under
/// [maxBytes]. Returns `null` if there is nothing to capture, or if capture
/// throws — which on the web just means no screenshot is offered, never a
/// crash reaching the person reporting a problem in the first place.
Future<Uint8List?> captureAppScreenshot({
  double pixelRatio = 1.0,
  int maxBytes = 4 * 1024 * 1024,
}) async {
  try {
    final renderObject = rootScreenshotKey.currentContext?.findRenderObject();
    if (renderObject is! RenderRepaintBoundary) return null;
    // Halves the ratio (roughly) until the PNG fits rather than guessing one
    // ratio up front — a dense desktop monitor's full-resolution PNG can be
    // tens of megabytes before any downscaling at all.
    for (var ratio = pixelRatio; ratio > 0.1; ratio /= 1.5) {
      final image = await renderObject.toImage(pixelRatio: ratio);
      final data = await image.toByteData(format: ui.ImageByteFormat.png);
      image.dispose();
      if (data == null) continue;
      final bytes = data.buffer.asUint8List();
      if (bytes.length <= maxBytes) return bytes;
    }
    return null;
  } catch (_) {
    return null;
  }
}

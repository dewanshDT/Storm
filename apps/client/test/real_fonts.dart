import 'dart:io';

import 'package:flutter/services.dart';

/// Loads the bundled Plex faces into the test engine, which otherwise draws
/// every glyph as a square as wide as the font size: a check of what fits on
/// a line has to measure the font the app ships.
Future<void> loadRealFonts() async {
  Future<ByteData> read(String name) async =>
      ByteData.sublistView(await File('assets/fonts/$name').readAsBytes());
  for (final (family, files) in [
    (
      'IBMPlexSans',
      [
        'IBMPlexSans-Regular.ttf',
        'IBMPlexSans-Medium.ttf',
        'IBMPlexSans-SemiBold.ttf',
      ],
    ),
    ('IBMPlexMono', ['IBMPlexMono-Regular.ttf', 'IBMPlexMono-Medium.ttf']),
  ]) {
    final loader = FontLoader(family);
    for (final f in files) {
      loader.addFont(read(f));
    }
    await loader.load();
  }
}

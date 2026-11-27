import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:storm/ui/panels.dart';
import 'package:storm/ui/session_status.dart';
import 'package:storm/ui/theme.dart';
import 'package:storm/ui/widgets.dart';

Widget _host(Widget child, {double width = 120}) => MaterialApp(
  theme: StormTheme.dark(),
  home: Scaffold(
    body: Align(
      alignment: Alignment.topLeft,
      child: SizedBox(width: width, child: child),
    ),
  ),
);

void main() {
  const long = 'A label far too long to fit in a narrow box at all';

  testWidgets('StatusPill ellipsises a long label', (tester) async {
    await tester.pumpWidget(
      _host(
        StatusPill(
          label: long,
          color: Colors.green,
          dot: const Icon(Icons.circle, size: 8),
        ),
      ),
    );
    expect(tester.takeException(), isNull);
  });

  for (final wrap in [false, true]) {
    testWidgets('PanelTabs long labels, wrap: $wrap', (tester) async {
      await tester.pumpWidget(
        _host(
          PanelTabs<String>(
            tabs: const [('a', long), ('b', long)],
            selected: 'a',
            onSelected: (_) {},
            wrap: wrap,
          ),
        ),
      );
      expect(tester.takeException(), isNull);
    });
  }

  testWidgets('ImageThumbnail falls back when the image is broken', (
    tester,
  ) async {
    await tester.pumpWidget(
      _host(
        ImageThumbnail(
          image: MemoryImage(Uint8List.fromList(const [1, 2, 3])),
          onRemove: () {},
        ),
      ),
    );
    await tester.pump();
    await tester.pump();
    expect(tester.takeException(), isNull);
  });
}

import 'dart:convert';

import 'package:flutter/gestures.dart' show PointerDeviceKind;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:xterm2/xterm.dart' show TerminalView;

import 'package:storm/agent/terminal_surface.dart';
import 'package:storm/ui/theme.dart';

/// Decision 122: links in a session's terminal open in the browser.
void main() {
  StormTerminal terminalWith(String output) {
    final t = StormTerminal();
    addTearDown(t.dispose);
    t.write(utf8.encode(output));
    return t;
  }

  group('terminalLinkAt', () {
    test('a plain URL is the link only where it is printed', () {
      final t = terminalWith('see https://example.com/a.');
      expect(t.linkAtForTest(4, 0), Uri.parse('https://example.com/a'));
      expect(t.linkAtForTest(24, 0), Uri.parse('https://example.com/a'));
      expect(t.linkAtForTest(25, 0), isNull, reason: 'the full stop is prose');
      expect(t.linkAtForTest(1, 0), isNull);
    });

    test('a closing bracket stays only when the URL opened it', () {
      final t = terminalWith('(https://en.wikipedia.org/wiki/Foo_(bar))');
      expect(
        t.linkAtForTest(1, 0),
        Uri.parse('https://en.wikipedia.org/wiki/Foo_(bar)'),
      );
    });

    test('a URL wrapped onto the next row is read whole from either row', () {
      final url = 'https://example.com/${'p' * 80}';
      final t = terminalWith('${'x' * 70} $url');
      expect(t.linkAtForTest(75, 0), Uri.parse(url));
      expect(t.linkAtForTest(5, 1), Uri.parse(url));
    });

    test('an OSC 8 link opens its target, not its text', () {
      final t = terminalWith(
        '\x1b]8;;https://target.example/x\x07click here\x1b]8;;\x07',
      );
      expect(t.linkAtForTest(2, 0), Uri.parse('https://target.example/x'));
    });

    test('only http, https and mailto are links', () {
      for (final text in [
        'javascript:alert(1)',
        'file:///etc/passwd',
        'http://',
        'ssh://host/x',
      ]) {
        expect(terminalWith(text).linkAtForTest(2, 0), isNull, reason: text);
      }
      expect(
        terminalWith('mailto:me@example.com').linkAtForTest(2, 0),
        Uri.parse('mailto:me@example.com'),
      );
      final osc = terminalWith(
        '\x1b]8;;javascript:alert(1)\x07here\x1b]8;;\x07',
      );
      expect(osc.linkAtForTest(1, 0), isNull);
    });
  });

  group('StormTerminalView', () {
    late List<Uri> opened;
    late List<MethodCall> platform;

    setUp(() {
      opened = [];
      platform = [];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, (call) async {
            platform.add(call);
            return null;
          });
    });

    /// A terminal showing [url] at its first cell, and where that cell is.
    Future<Offset> pump(WidgetTester tester, String url) async {
      final t = terminalWith(url);
      await tester.pumpWidget(
        MaterialApp(
          theme: StormTheme.dark(),
          home: Scaffold(
            body: SizedBox(
              width: 600,
              height: 400,
              child: StormTerminalView(
                terminal: t,
                openLink: (link) async {
                  opened.add(link);
                  return true;
                },
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      final view = find.byType(TerminalView);
      final padding = tester.widget<TerminalView>(view).padding!;
      return tester.getTopLeft(view) +
          Offset(padding.left + 3, padding.top + 3);
    }

    testWidgets('a Ctrl-click opens the link; a plain click does not', (
      tester,
    ) async {
      final cell = await pump(tester, 'https://example.com/docs');

      await tester.tapAt(cell, kind: PointerDeviceKind.mouse);
      await tester.pump();
      expect(opened, isEmpty, reason: 'a plain click selects and focuses');

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.tapAt(cell, kind: PointerDeviceKind.mouse);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();
      expect(opened, [Uri.parse('https://example.com/docs')]);
    });

    testWidgets('a tap asks first, then copies or opens', (tester) async {
      final cell = await pump(tester, 'https://example.com/docs');

      await tester.tapAt(cell);
      await tester.pumpAndSettle();
      expect(opened, isEmpty, reason: 'a tap never opens directly');
      expect(find.text('https://example.com/docs'), findsOneWidget);

      await tester.tap(find.text('Copy'));
      await tester.pumpAndSettle();
      final copy = platform.firstWhere((c) => c.method == 'Clipboard.setData');
      expect((copy.arguments as Map)['text'], 'https://example.com/docs');
      expect(find.text('Link copied'), findsOneWidget);

      await tester.tapAt(cell);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Open'));
      await tester.pumpAndSettle();
      expect(opened, [Uri.parse('https://example.com/docs')]);
      expect(find.text('Copy'), findsNothing, reason: 'the sheet closed');
    });

    testWidgets('a tap away from a link shows nothing', (tester) async {
      final cell = await pump(tester, 'no link here');
      await tester.tapAt(cell);
      await tester.pumpAndSettle();
      expect(find.text('Open'), findsNothing);
    });
  });
}

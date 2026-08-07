import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/editor/markdown_theme.dart';
import 'package:storm/editor/storm_markdown_controller.dart';
import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/editor_toolbar.dart';

import 'shell_harness.dart';
import 'fake_server.dart';

/// Reported from the phone: the buttons above the keyboard do nothing, and
/// Heading 1 in particular leaves the text alone.
///
/// It was doing something — it was *removing* the heading. Most notes open with
/// `# Title`, which is exactly the line you would try the button on, and
/// re-applying a prefix a line already had was treated as a toggle-off.
void main() {
  group('the heading picker is a choice, not a toggle', () {
    testWidgets('picking H1 on a line that is already H1 leaves it H1', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: const Size(411, 900));
      c.read(routerProvider).go(Routes.note(FakeServer.primaryVault, 'n0'));
      await tester.pumpAndSettle();
      await pumpShell(tester, c, size: const Size(411, 900), keyboard: 320);
      await tester.pumpAndSettle();

      final field = find.byType(TextField);
      await tester.tap(field);
      await tester.pumpAndSettle();
      final ctrl = tester.widget<TextField>(field).controller!;
      ctrl.selection = const TextSelection.collapsed(offset: 4);
      await tester.pump();
      expect(
        ctrl.text.split('\n').first,
        '# Welcome.md',
        reason: 'precondition',
      );

      await tester.tap(find.byIcon(Icons.title));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Heading 1'));
      await tester.pumpAndSettle();

      expect(ctrl.text.split('\n').first, '# Welcome.md');
      expect(
        c.read(noteSessionProvider).body.split('\n').first,
        '# Welcome.md',
        reason: 'and the session must agree, or the next save writes the loss',
      );
      await disposeShell(tester, c);
    });

    test('Paragraph is how the menu removes a heading', () {
      final c = _controller('# Title', offset: 3);
      c.setBlockPrefix(null, toggle: false);
      expect(c.text, 'Title');
    });

    test('a different level still replaces', () {
      final c = _controller('# Title', offset: 3);
      c.setBlockPrefix('### ', toggle: false);
      expect(c.text, '### Title');
    });

    test('the lone on/off buttons still toggle', () {
      // Bullets and quote have nowhere else to say "off", so for them
      // re-applying must remove. That difference is the whole point of the
      // flag, so both halves are pinned down.
      final c = _controller('- item', offset: 4);
      c.setBlockPrefix('- ');
      expect(c.text, 'item');

      final q = _controller('> quoted', offset: 4);
      q.setBlockPrefix('> ');
      expect(q.text, 'quoted');
    });
  });

  testWidgets(
    'tapping the toolbar does not steal focus from the editor',
    (tester) async {
      // TextField's default onTapOutside unfocuses on desktop and web, and an
      // unfocused field closes the keyboard, which hides the toolbar mid-tap.
      final controller = StormMarkdownController(
        theme: MarkdownTheme.dark(const TextStyle()),
        text: 'hello world',
      );
      final focus = FocusNode();
      addTearDown(focus.dispose);

      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Column(
              children: [
                Expanded(
                  child: TextField(controller: controller, focusNode: focus),
                ),
                EditorToolbar(controller: controller),
              ],
            ),
          ),
        ),
      );

      await tester.tap(find.byType(TextField));
      await tester.pumpAndSettle();
      controller.selection = const TextSelection(
        baseOffset: 6,
        extentOffset: 11,
      );
      await tester.pump();
      expect(focus.hasFocus, isTrue, reason: 'precondition');

      await tester.tap(find.byIcon(Icons.format_bold));
      await tester.pumpAndSettle();

      expect(controller.text, 'hello **world**');
      expect(
        focus.hasFocus,
        isTrue,
        reason: 'an unfocused field closes the keyboard and hides the toolbar',
      );
      // Run on the platforms whose onTapOutside actually unfocuses.
    },
    variant: TargetPlatformVariant.desktop(),
  );
}

StormMarkdownController _controller(String text, {required int offset}) {
  final c = StormMarkdownController(
    theme: MarkdownTheme.dark(const TextStyle()),
    text: text,
  );
  c.selection = TextSelection.collapsed(offset: offset);
  return c;
}

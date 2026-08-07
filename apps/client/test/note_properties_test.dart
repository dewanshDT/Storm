import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:storm/state/vault_config.dart';
import 'package:storm/ui/note_properties.dart';
import 'package:storm/ui/theme.dart';

/// The properties panel.
///
/// Two things it must never do, both of which the old read-only panel avoided
/// by simply not writing: reformat YAML it did not target, and claim to have
/// changed something it could not represent.
void main() {
  /// The panel alone, at phone width, with whatever the last edit produced.
  Future<String> pump(
    WidgetTester tester,
    String content, {
    Size size = const Size(411, 900),
    VaultConfig config = const VaultConfig(),
  }) async {
    var current = content;
    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    await tester.pumpWidget(
      ProviderScope(
        overrides: [vaultConfigProvider.overrideWith((ref) async => config)],
        child: MaterialApp(
          theme: StormTheme.dark(),
          home: Scaffold(
            body: SingleChildScrollView(
              padding: const EdgeInsets.symmetric(horizontal: 28),
              child: StatefulBuilder(
                builder: (context, setState) => NoteProperties(
                  content: current,
                  onChanged: (next) => setState(() => current = next),
                ),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    return current;
  }

  /// Reads what the panel currently holds after an interaction.
  String contentOf(WidgetTester tester) =>
      tester.widget<NoteProperties>(find.byType(NoteProperties)).content;

  group('layout', () {
    testWidgets('lays out at phone width with no frontmatter', (tester) async {
      await pump(tester, '# Heading\n\nbody\n');
      expect(find.byTooltip('Add property'), findsOneWidget);
      expect(tester.takeException(), isNull);
    });

    testWidgets('lays out a full block at phone width', (tester) async {
      await pump(
        tester,
        '---\n'
        'id: abc-123-def-456\n'
        'created: 2026-08-05T10:00:00Z\n'
        'modified: 2026-08-07T11:22:33Z\n'
        'a-rather-long-property-name: a fairly long value goes here\n'
        'tags: [homelab, storm, project, someday, maybe]\n'
        'done: true\n'
        '---\n\nbody\n',
      );
      expect(tester.takeException(), isNull);
    });

    testWidgets('lays out on a narrow phone', (tester) async {
      await pump(
        tester,
        '---\ntags: [alpha, beta, gamma]\nstatus: draft\n---\nbody\n',
        size: const Size(320, 720),
      );
      expect(tester.takeException(), isNull);
    });
  });

  group('rendering by type', () {
    testWidgets('a list renders one chip per item', (tester) async {
      await pump(tester, '---\ntags: [homelab, storm]\n---\nbody\n');
      expect(find.text('homelab'), findsOneWidget);
      expect(find.text('storm'), findsOneWidget);
      expect(find.byType(ValueChip), findsNWidgets(2));
    });

    testWidgets('a boolean renders a checkbox', (tester) async {
      await pump(tester, '---\ndone: true\n---\nbody\n');
      expect(find.byType(Checkbox), findsOneWidget);
      expect(tester.widget<Checkbox>(find.byType(Checkbox)).value, isTrue);
    });

    testWidgets('an ISO date renders formatted, not raw', (tester) async {
      await pump(tester, '---\ndue: 2026-08-07\n---\nbody\n');
      expect(find.text('7 Aug 2026'), findsOneWidget);
    });

    testWidgets('a declared type beats inference', (tester) async {
      // `2026` alone infers as a number; declaring it text must win.
      await pump(
        tester,
        '---\nyear: 2026\n---\nbody\n',
        config: const VaultConfig(types: {'year': PropertyType.select}),
      );
      expect(find.byType(DropdownButton<String>), findsOneWidget);
    });

    testWidgets('storm-owned fields are shown, read-only', (tester) async {
      await pump(
        tester,
        '---\nid: abc\nmodified: 2026-08-07T00:00:00Z\n---\nbody\n',
      );
      // Visible in the list — there is no raw mode to find them in — but
      // with no input, because the server rewrites them on every save.
      expect(find.text('id'), findsOneWidget);
      expect(find.text('modified'), findsOneWidget);
      expect(find.text('abc'), findsOneWidget);
      expect(find.byType(TextField), findsNothing);
    });

    testWidgets('a nested value is listed, read-only', (tester) async {
      await pump(tester, '---\nmeta:\n  a: 1\n  b: 2\nafter: x\n---\nbody\n');
      expect(find.text('meta'), findsOneWidget);
      expect(find.text('Nested value'), findsOneWidget);
      // And it does not swallow the property after it.
      expect(find.text('after'), findsOneWidget);
    });

    testWidgets('every frontmatter key gets a row', (tester) async {
      // The guarantee that replaced raw mode: nothing is reachable only by
      // editing text, so nothing may be missing from this list.
      const src =
          '---\n'
          'id: abc\n'
          'title: Storm\n'
          'tags: [a, b]\n'
          'meta:\n  x: 1\n'
          'text: |\n  line\n'
          '---\nbody\n';
      await pump(tester, src);
      for (final key in ['id', 'title', 'tags', 'meta', 'text']) {
        expect(find.text(key), findsOneWidget, reason: '"$key" is missing');
      }
    });

    testWidgets('there is no separator under the properties', (tester) async {
      await pump(tester, '---\ntitle: Storm\n---\nbody\n');
      expect(find.byType(Divider), findsNothing);
    });
  });

  group('editing writes frontmatter without reformatting it', () {
    testWidgets('typing a value commits on blur', (tester) async {
      await pump(
        tester,
        '---\ntitle: old\n# a hand comment\nother: keep\n---\n\nbody\n',
      );

      await tester.enterText(find.byType(TextField).first, 'new');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      final out = contentOf(tester);
      expect(out, contains('title: new'));
      expect(out, contains('# a hand comment'), reason: 'comments survive');
      expect(out, contains('other: keep'));
      expect(out, endsWith('---\n\nbody\n'), reason: 'the body is untouched');
    });

    testWidgets('a checkbox writes true and false', (tester) async {
      await pump(tester, '---\ndone: false\n---\nbody\n');
      await tester.tap(find.byType(Checkbox));
      await tester.pumpAndSettle();
      expect(contentOf(tester), contains('done: true'));
    });

    testWidgets('adding a tag keeps a block list a block list', (tester) async {
      // The case the server's writer would corrupt: it replaces the `tags:`
      // line and orphans the items beneath it.
      await pump(tester, '---\ntags:\n  - homelab\n  - storm\n---\n\nbody\n');

      await tester.enterText(find.byType(TextField).first, 'new');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      expect(
        contentOf(tester),
        '---\ntags:\n  - homelab\n  - storm\n  - new\n---\n\nbody\n',
      );
    });

    testWidgets('removing a chip removes only that item', (tester) async {
      await pump(tester, '---\ntags: [a, b, c]\nkeep: yes\n---\nbody\n');
      await tester.tap(find.byTooltip('Remove b'));
      await tester.pumpAndSettle();

      final out = contentOf(tester);
      expect(out, contains('tags: [a, c]'));
      expect(out, contains('keep: yes'));
    });

    testWidgets('deleting a property leaves its neighbours alone', (
      tester,
    ) async {
      await pump(tester, '---\na: 1\ndrop: x\nz: 26\n---\nbody\n');

      await tester.tap(find.text('drop'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Delete'));
      await tester.pumpAndSettle();

      expect(contentOf(tester), '---\na: 1\nz: 26\n---\nbody\n');
    });

    testWidgets('a value needing quotes gets them', (tester) async {
      await pump(tester, '---\ntitle: plain\n---\nbody\n');

      await tester.enterText(find.byType(TextField).first, 'My Note: a study');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      expect(contentOf(tester), contains('title: "My Note: a study"'));
    });

    testWidgets('a nested value cannot be written through the panel', (
      tester,
    ) async {
      const src = '---\nmeta:\n  a: 1\n---\nbody\n';
      await pump(tester, src);
      // There is no input for it at all, which is the guarantee.
      expect(find.byType(TextField), findsNothing);
      expect(contentOf(tester), src);
    });
  });
}

import 'package:flutter/material.dart';
import 'dart:ui' show Tristate;

import 'package:flutter/semantics.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/router.dart';
import 'package:storm/state/agent_writes.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

/// What a screen reader gets from the open note and the settings toggles.
/// Flutter web leaves a read-only text field's value out of the DOM, so text
/// must be a label, never a field's value.
void main() {
  const phone = Size(411, 900);
  const desk = Size(1280, 900);
  const primary = FakeServer.primaryVault;

  setUp(
    () => SharedPreferences.setMockInitialValues({
      SeenVersions.key: '{"$primary/":1}',
    }),
  );

  Future<ProviderContainer> at(
    WidgetTester tester,
    String location, {
    Size size = desk,
    void Function(FakeServer)? seed,
  }) async {
    final c = shellContainer();
    seed?.call(serverOf(c));
    await pumpShell(tester, c, size: size);
    c.read(routerProvider).go(location);
    await tester.pumpAndSettle();
    return c;
  }

  List<SemanticsData> nodes(WidgetTester tester) {
    final out = <SemanticsData>[];
    void walk(SemanticsNode n) {
      out.add(n.getSemanticsData());
      n.visitChildren((child) {
        walk(child);
        return true;
      });
    }

    walk(
      tester
          .binding
          .renderViews
          .first
          .owner!
          .semanticsOwner!
          .rootSemanticsNode!,
    );
    return out;
  }

  SemanticsData labelled(WidgetTester tester, String label) =>
      nodes(tester).singleWhere((d) => d.label == label);

  void seedProse(FakeServer s) => s.notes['n1'] = s.notes['n1']!.copyWith(
    content: 'Intro line.\n\n## Section\n\nSee [[Welcome]].\n',
  );

  group('open note', () {
    for (final size in [desk, phone]) {
      final side = size == desk ? 'desk' : 'phone';

      testWidgets('$side: title, prose and version line are read as text', (
        tester,
      ) async {
        final h = tester.ensureSemantics();
        final c = await at(
          tester,
          Routes.note(primary, 'n1'),
          size: size,
          seed: seedProse,
        );
        expect(
          nodes(
            tester,
          ).where((d) => d.label == '2026-08-05' && d.flagsCollection.isHeader),
          hasLength(1),
        );
        for (final text in ['Intro line.', 'Section', 'v1 · Saved']) {
          expect(
            labelled(tester, text).flagsCollection.isTextField,
            isFalse,
            reason: text,
          );
        }
        expect(
          nodes(tester).where(
            (d) =>
                d.flagsCollection.isReadOnly && d.value.contains('Intro line.'),
          ),
          isEmpty,
        );
        expect(
          nodes(
            tester,
          ).where((d) => d.label == 'Welcome' && d.flagsCollection.isLink),
          hasLength(1),
        );
        await disposeShell(tester, c);
        h.dispose();
      });

      testWidgets('$side: provenance is its own link, apart from the version', (
        tester,
      ) async {
        final h = tester.ensureSemantics();
        final c = await at(
          tester,
          Routes.note(primary, 'n1'),
          size: size,
          seed: (s) => s.agentWrote('n1', sessionId: 'ags_1', version: 1),
        );
        expect(labelled(tester, 'v1 · Saved').flagsCollection.isLink, isFalse);
        const link = 'Edited by session gateway-spec, just now';
        final data = labelled(tester, link);
        expect(data.flagsCollection.isLink, isTrue);
        expect(data.hasAction(SemanticsAction.tap), isTrue);

        tester.semantics.tap(find.semantics.byLabel(link));
        await tester.pumpAndSettle();
        final uri = c.read(routerProvider).state.uri;
        expect(uri.path, '/agents/s/ags_1');
        expect(uri.queryParameters['tab'], 'wrote');
        await disposeShell(tester, c);
        h.dispose();
      });
    }

    testWidgets('a dismissed session is named but is not a link', (
      tester,
    ) async {
      final h = tester.ensureSemantics();
      final c = await at(
        tester,
        Routes.note(primary, 'n1'),
        seed: (s) => s.agentWrote(
          'n1',
          sessionId: 'ags_gone',
          version: 1,
          dismissed: true,
        ),
      );
      final line = nodes(
        tester,
      ).singleWhere((d) => d.label.contains('Edited by session gateway-spec'));
      expect(line.flagsCollection.isLink, isFalse);
      await disposeShell(tester, c);
      h.dispose();
    });
  });

  group('settings toggles', () {
    SemanticsData toggle(WidgetTester tester, String label) {
      final d = labelled(tester, label);
      expect(
        d.flagsCollection.isToggled != Tristate.none,
        isTrue,
        reason: label,
      );
      return d;
    }

    testWidgets('AI access: each switch is named by its row, with state', (
      tester,
    ) async {
      final h = tester.ensureSemantics();
      final c = await at(
        tester,
        Routes.settingsPage('ai'),
        seed: (s) => s.agentWrites = false,
      );
      final read = toggle(tester, 'Let AI apps read your notes');
      expect(read.hint, 'Serves your vaults to apps that hold an access key.');
      expect(read.flagsCollection.isEnabled == Tristate.isTrue, isTrue);

      final writes = toggle(tester, 'Allow writes when chosen at launch');
      expect(writes.flagsCollection.isToggled, Tristate.isFalse);
      expect(writes.hasAction(SemanticsAction.tap), isTrue);
      tester.semantics.tap(
        find.semantics.byLabel('Allow writes when chosen at launch'),
      );
      await tester.pumpAndSettle();
      expect(serverOf(c).agentWrites, isTrue);
      expect(
        toggle(
              tester,
              'Allow writes when chosen at launch',
            ).flagsCollection.isToggled ==
            Tristate.isTrue,
        isTrue,
      );

      final mcpWrite = toggle(tester, 'Let them create, edit and delete');
      expect(
        mcpWrite.flagsCollection.isEnabled == Tristate.isTrue,
        serverOf(c).mcpEnabled,
      );
      await disposeShell(tester, c);
      h.dispose();
    });

    testWidgets('the row text is not read a second time', (tester) async {
      final h = tester.ensureSemantics();
      final c = await at(tester, Routes.settingsPage('ai'));
      expect(
        nodes(
          tester,
        ).where((d) => d.label.contains('Let AI apps read your notes')),
        hasLength(1),
      );
      await disposeShell(tester, c);
      h.dispose();
    });
  });
}

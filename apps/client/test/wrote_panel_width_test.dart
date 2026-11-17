import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show RenderParagraph;
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agent_widgets.dart';
import 'package:storm/agent/session_inspector.dart';
import 'package:storm/router.dart';

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'real_fonts.dart';
import 'shell_harness.dart';

/// 171-9: a note in the session panel spans the panel, whatever its width.
void main() {
  const primary = FakeServer.primaryVault;
  const pad = 16.0;
  final thoughts = File(
    "test/fixtures/Claude's Thoughts.md",
  ).readAsStringSync();

  setUp(() => SharedPreferences.setMockInitialValues({}));

  Future<ProviderContainer> openWrote(
    WidgetTester tester, {
    String title = "Claude's Thoughts",
  }) async {
    await tester.runAsync(() async {
      await loadRealFonts();
      final serif = FontLoader('Newsreader');
      for (final f in ['Newsreader.ttf', 'Newsreader-Italic.ttf']) {
        serif.addFont(
          Future.value(
            ByteData.sublistView(File('assets/fonts/$f').readAsBytesSync()),
          ),
        );
      }
      await serif.load();
    });
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [agentSession('ags_1', writeVaultId: primary, wroteCount: 1)],
    );
    agents.writes['ags_1'] = [
      {
        'vault_id': primary,
        'note_id': 'thoughts',
        'title': title,
        'path': '$title.md',
        'kind': 'created',
        'version': 1,
        'at': '2026-11-17T05:46:34Z',
      },
    ];
    final c = shellContainer(agentClient: agents.client);
    serverOf(c).notes['thoughts'] = ServerNote(
      id: 'thoughts',
      path: '$title.md',
      content: thoughts,
      version: 1,
      title: title,
    );
    await pumpShell(tester, c, size: const Size(1850, 1000));
    c.read(routerProvider).go(Routes.agentSession('ags_1', tab: 'wrote'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('wrote-thoughts')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('back-to-wrote')), findsOneWidget);
    return c;
  }

  void expectSpansPanel(WidgetTester tester, double width) {
    final panel = tester.getRect(find.byKey(const Key('inspector-panel')));
    expect(panel.width, width);
    final right = panel.right - pad;
    expect(
      tester.getTopRight(find.byKey(const Key('open-in-notes'))).dx,
      moreOrLessEquals(right, epsilon: 0.5),
      reason: 'Open in Notes sits at the content edge, at $width',
    );
    final body = tester.getRect(find.byKey(const Key('storm-markdown-body')));
    expect(body.left, moreOrLessEquals(panel.left + pad, epsilon: 0.5));
    expect(body.right, moreOrLessEquals(right, epsilon: 0.5));
    final paragraphs = tester.renderObjectList<RenderParagraph>(
      find.descendant(
        of: find.byKey(const Key('storm-markdown-body')),
        matching: find.byType(RichText),
      ),
    );
    expect(paragraphs, isNotEmpty);
    for (final p in paragraphs) {
      final r = p.localToGlobal(Offset(p.size.width, 0)).dx;
      expect(
        r,
        lessThanOrEqualTo(right + 0.5),
        reason: '"${p.text.toPlainText()}" at $width',
      );
    }
  }

  testWidgets('a Wrote note spans the panel at 500 and 360, and resized', (
    tester,
  ) async {
    final c = await openWrote(tester);
    for (final w in [500.0, 360.0, 500.0]) {
      c.read(sessionInspectorWidthProvider.notifier).state = w;
      await tester.pumpAndSettle();
      expectSpansPanel(tester, w);
    }
    // The window narrows: the panel gives way and the content follows it.
    tester.view.physicalSize = const Size(1200, 1000);
    await tester.pumpAndSettle();
    final w = tester.getSize(find.byKey(const Key('inspector-panel'))).width;
    expect(w, lessThan(500));
    expectSpansPanel(tester, w);
    await disposeShell(tester, c);
  });

  testWidgets('the panel title is not repeated by the body\'s leading H1', (
    tester,
  ) async {
    final c = await openWrote(tester);
    expect(
      tester.widget<Text>(find.byKey(const Key('panel-note-title'))).data,
      "Claude's Thoughts",
    );
    expect(
      find.descendant(
        of: find.byKey(const Key('storm-markdown-body')),
        matching: find.text("Claude's Thoughts", findRichText: true),
      ),
      findsNothing,
    );
    expect(
      find.descendant(
        of: find.byKey(const Key('storm-markdown-body')),
        matching: find.textContaining('On being handed', findRichText: true),
      ),
      findsOneWidget,
    );
    await disposeShell(tester, c);
  });

  test('only a leading H1 equal to the title is dropped', () {
    expect(withoutLeadingTitle('\n# Plan\n\nBody', ' plan '), '\nBody');
    expect(withoutLeadingTitle('# Plan #\nBody', 'Plan'), 'Body');
    expect(withoutLeadingTitle('# Other\nBody', 'Plan'), '# Other\nBody');
    expect(withoutLeadingTitle('## Plan\nBody', 'Plan'), '## Plan\nBody');
    expect(withoutLeadingTitle('Intro\n# Plan', 'Plan'), 'Intro\n# Plan');
    expect(withoutLeadingTitle('# Plan', ''), '# Plan');
  });

  testWidgets('a long title wraps inside the panel at every desk width', (
    tester,
  ) async {
    const long = 'Issue - Wrote panel note overflows its width';
    final c = await openWrote(tester, title: long);
    for (final win in [1280.0, 1440.0, 1850.0]) {
      for (final w in [null, 360.0, 500.0]) {
        c.read(sessionInspectorWidthProvider.notifier).state = w;
        tester.view.physicalSize = Size(win, 1000);
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull);
        final panel = tester.getRect(find.byKey(const Key('inspector-panel')));
        expect(panel.right, lessThanOrEqualTo(win), reason: 'at $win');
        final title = find.byKey(const Key('panel-note-title'));
        expect(
          tester.getRect(title).right,
          lessThanOrEqualTo(panel.right - pad),
        );
        final p = tester.renderObject<RenderParagraph>(
          find.descendant(of: title, matching: find.byType(RichText)),
        );
        expect(
          p.getMaxIntrinsicWidth(double.infinity),
          greaterThan(p.size.width),
        );
        expect(
          tester.getSize(title).height,
          greaterThan(p.getMinIntrinsicHeight(double.infinity) * 1.5),
          reason: 'the title wraps at $win, panel ${panel.width}',
        );
        expectSpansPanel(tester, panel.width);
      }
    }
    await disposeShell(tester, c);
  });
}

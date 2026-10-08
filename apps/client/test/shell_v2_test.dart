import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/router.dart';
import 'package:storm/state/health.dart';
import 'package:storm/state/nav_memory.dart';
import 'package:storm/ui/shell/activity_rail.dart';
import 'package:storm/ui/shell/app_shell.dart';
import 'package:storm/ui/tokens.dart';

import 'fake_server.dart';
import 'shell_harness.dart';

void main() {
  const v = FakeServer.primaryVault;
  const phone = Size(411, 900);
  const wide = Size(1280, 900);

  String location(ProviderContainer c) => c.read(routerProvider).state.uri.path;

  group('NavMemory', () {
    test('remembers the last location per activity', () {
      final m = const NavMemory()
          .visit(Routes.note(v, 'n1'))
          .visit(Routes.agents)
          .visit(Routes.settingsPage('ai'));
      expect(m.activity, Activity.settings);
      expect(m.notes, Routes.note(v, 'n1'));
      expect(m.agents, Routes.agents);
      expect(m.launch, Routes.settingsPage('ai'));
    });

    test('an activity with no history opens at its entry', () {
      const m = NavMemory();
      expect(m.launch, Routes.notes);
      expect(m.entryOf(Activity.agents), Routes.agents);
      expect(m.entryOf(Activity.settings), Routes.settings);
    });

    test('the Notes entry and auth screens are not remembered', () {
      final m = const NavMemory()
          .visit(Routes.browse(v))
          .visit(Routes.notes)
          .visit(Routes.login);
      expect(m.notes, Routes.browse(v));
      expect(m.activity, Activity.notes);
    });
  });

  group('logicalParent (Q2)', () {
    final memory = NavMemory(notes: Routes.folder(v, 'Daily'));
    String? parent(String l, {bool wide = false}) => logicalParent(
      l,
      wide: wide,
      memory: memory,
      folderOfNote: (id) => id == 'n3' ? 'Projects/Storm' : null,
    );

    test('notes and folders walk up; the vault root exits', () {
      expect(parent(Routes.note(v, 'n3')), Routes.folder(v, 'Projects/Storm'));
      expect(parent(Routes.note(v, 'gone')), Routes.browse(v));
      expect(
        parent(Routes.folder(v, 'Projects/Storm')),
        Routes.folder(v, 'Projects'),
      );
      expect(parent(Routes.folder(v, 'Projects')), Routes.browse(v));
      expect(parent(Routes.browse(v)), isNull);
      expect(parent(Routes.search(v)), Routes.browse(v));
    });

    test('Agents and Settings return to the last Notes location', () {
      expect(parent(Routes.agents), Routes.folder(v, 'Daily'));
      expect(parent('/agents/s/ags_1'), Routes.agents);
      expect(parent(Routes.settings), Routes.folder(v, 'Daily'));
      expect(parent(Routes.settingsPage('ai')), Routes.settings);
      expect(
        parent(Routes.settingsPage('ai'), wide: true),
        Routes.folder(v, 'Daily'),
      );
    });
  });

  group('the rail', () {
    testWidgets('only at desk width, with the current activity lit', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      expect(find.byType(ActivityRail), findsNothing);
      await disposeShell(tester, c);

      final w = shellContainer();
      await pumpShell(tester, w, size: wide);
      expect(find.byType(ActivityRail), findsOneWidget);
      final t = StormTokens.from(StormPreset.stormDark);
      Color fill(String key) => tester
          .widget<Material>(
            find
                .descendant(
                  of: find.byKey(Key(key)),
                  matching: find.byType(Material),
                )
                .first,
          )
          .color!;
      expect(fill('rail-notes'), t.accentSoft);
      expect(fill('rail-agents'), Colors.transparent);

      await tester.tap(find.byKey(const Key('rail-agents')));
      await tester.pumpAndSettle();
      expect(fill('rail-agents'), t.accentSoft);
      expect(fill('rail-notes'), Colors.transparent);
      await disposeShell(tester, w);
    });

    testWidgets('each activity reopens where it was left', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: wide);
      c.read(routerProvider).go(Routes.folder(v, 'Daily'));
      await tester.pumpAndSettle();
      c.read(routerProvider).go(Routes.settingsPage('ai'));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const Key('rail-notes')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.folder(v, 'Daily'));
      await tester.tap(find.byKey(const Key('rail-settings')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.settingsPage('ai'));
      await disposeShell(tester, c);
    });
  });

  group('health', () {
    MockClient integrations(String status) => MockClient(
      (req) async => http.Response(
        jsonEncode([
          {
            'id': 'int_1',
            'slug': 'github',
            'display_name': 'GitHub',
            'url': 'https://example.com/mcp',
            'auth_kind': 'oauth',
            'status': status,
          },
        ]),
        200,
      ),
    );

    Color dotColor(WidgetTester tester) =>
        (tester
                    .widget<Container>(
                      find
                          .descendant(
                            of: find.byKey(const Key('rail-status')),
                            matching: find.byType(Container),
                          )
                          .last,
                    )
                    .decoration!
                as BoxDecoration)
            .color!;

    testWidgets('green when nothing needs attention', (tester) async {
      final c = shellContainer(integrationsClient: integrations('connected'));
      await pumpShell(tester, c, size: wide);
      expect(c.read(healthToneProvider), HealthTone.good);
      expect(dotColor(tester), StormTokens.from(StormPreset.stormDark).green);
      await disposeShell(tester, c);
    });

    testWidgets('danger, with the row, when an integration needs sign-in', (
      tester,
    ) async {
      final c = shellContainer(
        integrationsClient: integrations('needs_reauth'),
      );
      await pumpShell(tester, c, size: wide);
      expect(dotColor(tester), StormTokens.from(StormPreset.stormDark).danger);

      await tester.tap(find.byKey(const Key('rail-status')));
      await tester.pumpAndSettle();
      expect(find.text('GitHub needs you to sign in again'), findsOneWidget);
      final sync = c.read(healthRowsProvider).first;
      expect(find.text(sync.text), findsOneWidget, reason: 'the sync row');

      await tester.tap(find.byKey(const Key('health-about')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.settingsPage('health'));
      expect(find.text('GitHub needs you to sign in again'), findsOneWidget);
      await disposeShell(tester, c);
    });
  });

  group('phone bubbles', () {
    testWidgets('the gear opens Settings and is lit there', (tester) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      await tester.tap(find.byKey(const Key('settings-bubble')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.settings);
      expect(find.byKey(const Key('settings-row-health')), findsOneWidget);
      await disposeShell(tester, c);
    });

    testWidgets('the place picker moves between a vault and Agents', (
      tester,
    ) async {
      final c = shellContainer();
      await pumpShell(tester, c, size: phone);
      await tester.tap(find.byKey(const Key('places-bubble')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('place-agents')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.agents);

      await tester.tap(find.byKey(const Key('places-bubble')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('place-vault-$v')));
      await tester.pumpAndSettle();
      expect(location(c), Routes.browse(v));
      await disposeShell(tester, c);
    });
  });

  testWidgets('the rail and the sidebar stay in the semantics tree', (
    tester,
  ) async {
    final handle = tester.ensureSemantics();
    final c = shellContainer();
    await pumpShell(tester, c, size: wide);
    for (final label in ['Notes', 'Agents', 'Settings', 'Status']) {
      expect(find.bySemanticsLabel(label), findsOneWidget, reason: label);
    }
    expect(find.bySemanticsLabel(RegExp('Primary')), findsWidgets);
    await disposeShell(tester, c);
    handle.dispose();
  });
}

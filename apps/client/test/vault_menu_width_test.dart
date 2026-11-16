import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/router.dart';
import 'package:storm/ui/surfaces.dart';
import 'package:storm/ui/widgets.dart';

import 'agent_fakes.dart';
import 'shell_harness.dart';

/// Slice 171-4: the write-vault menu fits its names, not its compact button.
void main() {
  const wide = Size(1400, 900);
  const long = 'a vault with a name long enough to never fit any menu at all';
  const short = ['Primary', 'kit', 'personal', 'shit', 'workk'];

  setUp(() => SharedPreferences.setMockInitialValues({}));

  Map<String, dynamic> host() => agentHost(
    providers: ['claude-code'],
    workspaces: ['storm'],
    identity: 'verified',
    isolation: 'verified',
    execution: {'account': 'dewansh', 'isolation': 'per-session'},
  );

  Future<ProviderContainer> open(WidgetTester t) async {
    final agents = FakeAgentServer(hosts: [host()]);
    final c = shellContainer(agentClient: agents.client);
    final server = serverOf(c);
    for (final (i, name) in [...short.skip(1), long].indexed) {
      server.addVault('v-$i', name);
    }
    await pumpShell(t, c, size: wide);
    c.read(routerProvider).go(Routes.agents);
    await t.pumpAndSettle();
    await t.tap(find.byKey(const Key('new-session')));
    await t.pumpAndSettle();
    await t.ensureVisible(find.byKey(const Key('launcher-more')));
    await t.tap(find.byKey(const Key('launcher-more')));
    await t.pumpAndSettle();
    return c;
  }

  Future<void> openMenu(WidgetTester t) async {
    await t.ensureVisible(find.byKey(const Key('launcher-write-vault')));
    await t.pumpAndSettle();
    await t.tap(find.byKey(const Key('launcher-write-vault')));
    await t.pumpAndSettle();
  }

  Finder item(String label) => find.descendant(
    of: find.byType(StormPopover),
    matching: find.widgetWithText(PopoverItem, label),
  );

  RenderParagraph line(WidgetTester t, String label) => t.renderObject(
    find.descendant(of: item(label), matching: find.text(label)),
  );

  void expectFits(WidgetTester t) {
    final width = t.getSize(find.byType(StormPopover)).width;
    expect(width, inInclusiveRange(180, 320));
    for (final name in short) {
      expect(line(t, name).didExceedMaxLines, isFalse, reason: name);
      expect(
        find.descendant(of: item(name), matching: find.byType(Tooltip)),
        findsNothing,
        reason: name,
      );
    }
    expect(line(t, long).didExceedMaxLines, isTrue);
    final tip = t.widget<Tooltip>(
      find.descendant(of: item(long), matching: find.byType(Tooltip)),
    );
    expect(tip.message, long);
  }

  testWidgets('the menu fits its names between 180 and 320, and elides '
      'only past that with the whole name on hover', (t) async {
    final c = await open(t);
    await openMenu(t);
    expectFits(t);
    await disposeShell(t, c);
  });

  testWidgets('a narrow anchor still opens a menu wide enough', (t) async {
    final c = await open(t);
    await openMenu(t);
    await t.tap(item('kit'));
    await t.pumpAndSettle();

    final anchor = t.getSize(find.byKey(const Key('launcher-write-vault')));
    expect(anchor.width, lessThan(180));
    await openMenu(t);
    expect(
      t.getSize(find.byType(StormPopover)).width,
      greaterThan(anchor.width),
    );
    expectFits(t);
    await disposeShell(t, c);
  });

  testWidgets('a wide anchor caps the menu at 320', (t) async {
    final c = await open(t);
    await openMenu(t);
    await t.tap(item(long));
    await t.pumpAndSettle();

    await openMenu(t);
    expectFits(t);
    await disposeShell(t, c);
  });
}

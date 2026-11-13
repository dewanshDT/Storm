import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/agent/agents_shell.dart' show AgentsSidebar;
import 'package:storm/agent/terminal_surface.dart' show StormTerminalView;
import 'package:storm/keyboard/storm_activators.dart';
import 'package:storm/keyboard/storm_app_shortcuts.dart';
import 'package:storm/keyboard/storm_commands.dart';
import 'package:storm/router.dart';
import 'package:storm/state/app_state.dart';
import 'package:storm/ui/settings/settings_shell.dart' show SettingsNav;
import 'package:storm/ui/shell/vault_sidebar.dart';

import 'agent_fakes.dart';
import 'fake_server.dart';
import 'shell_harness.dart';

/// The app-wide shortcut layer (decision 156d–f), driven with real key
/// events on every screen, both sides of the breakpoint, with Meta on
/// macOS and Ctrl on Linux.
void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));
  tearDown(() => debugDefaultTargetPlatformOverride = null);

  const at899 = Size(899, 900);
  const at900 = Size(900, 900);
  const v = FakeServer.primaryVault;

  Future<void> frames(WidgetTester t) async {
    for (var i = 0; i < 20; i++) {
      await t.pump(const Duration(milliseconds: 50));
    }
  }

  /// [key] with the platform's modifier (⌘ or Ctrl), and Shift or Alt.
  Future<void> press(
    WidgetTester t,
    LogicalKeyboardKey key, {
    bool mod = true,
    bool shift = false,
    bool alt = false,
  }) async {
    final modifier = stormUsesMetaModifier
        ? LogicalKeyboardKey.metaLeft
        : LogicalKeyboardKey.controlLeft;
    if (mod) await t.sendKeyDownEvent(modifier);
    if (shift) await t.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    if (alt) await t.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await t.sendKeyEvent(key);
    if (alt) await t.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    if (shift) await t.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    if (mod) await t.sendKeyUpEvent(modifier);
    await frames(t);
  }

  Future<void> back(WidgetTester t) => stormUsesMetaModifier
      ? press(t, LogicalKeyboardKey.bracketLeft)
      : press(t, LogicalKeyboardKey.arrowLeft, mod: false, alt: true);

  Future<void> forward(WidgetTester t) => stormUsesMetaModifier
      ? press(t, LogicalKeyboardKey.bracketRight)
      : press(t, LogicalKeyboardKey.arrowRight, mod: false, alt: true);

  String location(ProviderContainer c) =>
      c.read(routerProvider).state.uri.toString();

  final newNoteDialog = find.widgetWithText(AlertDialog, 'New note');
  final newFolderDialog = find.text('New folder');
  final launcher = find.byKey(const Key('launcher-chips'));

  Future<void> closeRoute(WidgetTester t) async {
    t.state<NavigatorState>(find.byType(Navigator).first).pop();
    await frames(t);
  }

  Future<ProviderContainer> open(
    WidgetTester t,
    Size size,
    String route, {
    bool noVaults = false,
  }) async {
    final agents = FakeAgentServer(
      hosts: [agentHost()],
      sessions: [agentSession('ags_1', name: 'gateway-impl')],
    );
    final c = shellContainer(agentClient: agents.client);
    if (noVaults) serverOf(c).vaults.clear();
    await pumpShellFrames(t, c, size: size);
    for (final step in [Routes.agents, '/settings/ai', route]) {
      c.read(routerProvider).go(step);
      await frames(t);
    }
    return c;
  }

  Future<void> close(WidgetTester t, ProviderContainer c) async {
    await t.pumpWidget(const SizedBox());
    c.dispose();
    debugDefaultTargetPlatformOverride = null;
  }

  /// A session autofocuses its terminal at the desk, where the app's
  /// chords stop; leaving it is how they come back.
  void leaveTerminal(WidgetTester t) {
    for (final view in t.widgetList<StormTerminalView>(
      find.byType(StormTerminalView),
    )) {
      view.focusNode?.unfocus();
    }
  }

  final screens = <(String, String, Finder?, bool)>[
    ('Notes', Routes.browse(v), find.byType(VaultSidebar), false),
    ('NotesHome', Routes.notes, null, true),
    ('Agents', Routes.agents, find.byType(AgentsSidebar), false),
    (
      'a session',
      Routes.agentSession('ags_1'),
      find.byType(AgentsSidebar),
      false,
    ),
    (
      'Settings',
      Routes.settingsPage('device'),
      find.byType(SettingsNav),
      false,
    ),
  ];

  for (final platform in [TargetPlatform.macOS, TargetPlatform.linux]) {
    for (final size in [at899, at900]) {
      final wide = size == at900;
      for (final (name, route, sidebar, noVaults) in screens) {
        final agentsActivity = route.startsWith('/agents');
        testWidgets(
          '${platform.name} at ${size.width.toInt()}: the chords work in $name',
          (t) async {
            debugDefaultTargetPlatformOverride = platform;
            final c = await open(t, size, route, noVaults: noVaults);
            expect(location(c), route);
            leaveTerminal(t);
            await frames(t);

            await press(t, LogicalKeyboardKey.keyN);
            expect(newNoteDialog, findsOneWidget, reason: '⌘N');
            await closeRoute(t);

            await press(t, LogicalKeyboardKey.keyN, shift: true);
            if (agentsActivity) {
              expect(launcher, findsOneWidget, reason: '⇧⌘N is New session');
              await closeRoute(t);
            } else if (route.startsWith('/settings')) {
              expect(newFolderDialog, findsNothing);
              expect(launcher, findsNothing);
            } else {
              expect(newFolderDialog, findsOneWidget, reason: '⇧⌘N');
              await closeRoute(t);
            }

            await press(t, LogicalKeyboardKey.backslash);
            expect(c.read(sidebarCollapsedProvider), wide, reason: '⌘\\');
            if (wide && sidebar != null) expect(sidebar, findsNothing);
            await press(t, LogicalKeyboardKey.backslash);
            expect(c.read(sidebarCollapsedProvider), isFalse);
            if (wide && sidebar != null) expect(sidebar, findsOneWidget);

            // `/notes` only forwards to a vault, so history skips it.
            final notesHome = route == Routes.notes;
            await back(t);
            expect(
              location(c),
              notesHome ? Routes.agents : '/settings/ai',
              reason: 'back',
            );
            await forward(t);
            expect(
              location(c),
              notesHome ? '/settings/ai' : route,
              reason: 'forward',
            );

            await close(t, c);
          },
        );
      }
    }
  }

  group('focus that went nowhere', () {
    testWidgets('focus that fell above the layer is taken back', (t) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      final c = await open(t, at900, Routes.agents);
      final layer = t.element(find.byType(StormAppShortcuts));
      bool inLayer(FocusNode? node) =>
          node?.context != null &&
          (node!.context == layer ||
              node.context!
                      .findAncestorWidgetOfExactType<StormAppShortcuts>() !=
                  null);

      // What a detached page does to focus: it walks up scope by scope,
      // out of the shell.
      final manager = FocusManager.instance;
      for (var i = 0; i < 20 && inLayer(manager.primaryFocus); i++) {
        manager.primaryFocus!.unfocus();
        manager.applyFocusChangesIfNeeded();
      }
      expect(inLayer(manager.primaryFocus), isFalse);
      await frames(t);
      expect(inLayer(manager.primaryFocus), isTrue);

      await press(t, LogicalKeyboardKey.keyN, shift: true);
      expect(launcher, findsOneWidget);
      await closeRoute(t);
      await close(t, c);
    });

    testWidgets('a removed focused terminal leaves the chords working', (
      t,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      final c = await open(t, at900, Routes.agentSession('ags_1'));
      final view = t.widget<StormTerminalView>(find.byType(StormTerminalView));
      expect(view.focusNode!.hasFocus, isTrue);

      c.read(routerProvider).go(Routes.settingsPage('device'));
      await frames(t);
      expect(find.byType(StormTerminalView), findsNothing);

      await press(t, LogicalKeyboardKey.keyN);
      expect(newNoteDialog, findsOneWidget);
      await closeRoute(t);
      await close(t, c);
    });

    testWidgets('a removed focused editor leaves the chords working', (
      t,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
      final c = await open(t, at900, Routes.browse(v));
      c.read(routerProvider).go(Routes.note(v, serverOf(c).notes.keys.first));
      await frames(t);
      await enterEditMode(t);
      await t.tap(find.byKey(const Key('note-body')));
      await frames(t);
      expect(
        FocusManager.instance.primaryFocus?.context
            ?.findAncestorWidgetOfExactType<EditableText>(),
        isNotNull,
      );

      c.read(routerProvider).go(Routes.agents);
      await frames(t);

      await press(t, LogicalKeyboardKey.keyN, shift: true);
      expect(launcher, findsOneWidget);
      await closeRoute(t);
      await close(t, c);
    });

    testWidgets('a field let go by a tap outside leaves the chords working', (
      t,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      final c = await open(t, at900, Routes.browse(v));
      c.read(routerProvider).go(Routes.note(v, serverOf(c).notes.keys.first));
      await frames(t);
      await enterEditMode(t);
      await t.tap(find.byKey(const Key('note-body')));
      await frames(t);

      // The app bar's blank middle: no control, so only the tap-outside.
      await t.tapAt(const Offset(450, 20));
      await frames(t);
      expect(
        FocusManager.instance.primaryFocus?.context
            ?.findAncestorWidgetOfExactType<EditableText>(),
        isNull,
      );

      await press(t, LogicalKeyboardKey.keyN);
      expect(newNoteDialog, findsOneWidget);
      await closeRoute(t);
      await press(t, LogicalKeyboardKey.backslash);
      expect(c.read(sidebarCollapsedProvider), isTrue);
      await close(t, c);
    });

    testWidgets('the keeper never takes focus from a focused field', (t) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      final c = await open(t, at900, Routes.browse(v));
      c.read(routerProvider).go(Routes.note(v, serverOf(c).notes.keys.first));
      await frames(t);
      await enterEditMode(t);
      await t.tap(find.byKey(const Key('note-body')));
      await frames(t);
      await t.enterText(find.byKey(const Key('note-body')), 'still here');
      await frames(t);

      final focused = FocusManager.instance.primaryFocus;
      expect(
        focused?.context?.findAncestorWidgetOfExactType<EditableText>(),
        isNotNull,
      );
      expect(FocusManager.instance.primaryFocus, same(focused));
      await close(t, c);
    });

    testWidgets('a dialog keeps its focus', (t) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      final c = await open(t, at900, Routes.browse(v));
      await press(t, LogicalKeyboardKey.keyN);
      expect(newNoteDialog, findsOneWidget);
      final inDialog = FocusManager.instance.primaryFocus;
      inDialog?.unfocus();
      await frames(t);
      final at = FocusManager.instance.primaryFocus?.context;
      expect(
        at == null ? null : ModalRoute.of(at),
        isA<PopupRoute<Object?>>(),
        reason: 'focus stays in the dialog, never back under it',
      );
      await closeRoute(t);
      await close(t, c);
    });
  });

  group('the terminal', () {
    Future<ProviderContainer> focusedTerminal(
      WidgetTester t,
      TargetPlatform platform,
    ) async {
      debugDefaultTargetPlatformOverride = platform;
      final c = await open(t, at900, Routes.agentSession('ags_1'));
      final view = t.widget<StormTerminalView>(find.byType(StormTerminalView));
      expect(view.focusNode!.hasFocus, isTrue);
      return c;
    }

    testWidgets('Linux: the app chords stop at it', (t) async {
      final c = await focusedTerminal(t, TargetPlatform.linux);
      await press(t, LogicalKeyboardKey.keyN);
      expect(newNoteDialog, findsNothing);
      await press(t, LogicalKeyboardKey.keyN, shift: true);
      expect(launcher, findsNothing);
      await press(t, LogicalKeyboardKey.backslash);
      expect(c.read(sidebarCollapsedProvider), isFalse);
      await press(t, LogicalKeyboardKey.keyK);
      await press(t, LogicalKeyboardKey.keyP);
      expect(location(c), Routes.agentSession('ags_1'), reason: 'readline');
      await close(t, c);
    });

    testWidgets('Linux: Ctrl+Shift+K gets through', (t) async {
      final c = await focusedTerminal(t, TargetPlatform.linux);
      await press(t, LogicalKeyboardKey.keyK, shift: true);
      expect(location(c), Routes.search(v));
      await close(t, c);
    });

    testWidgets('Linux: Ctrl+Shift+P gets through', (t) async {
      final c = await focusedTerminal(t, TargetPlatform.linux);
      await press(t, LogicalKeyboardKey.keyP, shift: true);
      expect(location(c), Routes.search(v));
      await close(t, c);
    });

    testWidgets('macOS: ⌘N, ⇧⌘N and ⌘\\ stop at it', (t) async {
      final c = await focusedTerminal(t, TargetPlatform.macOS);
      await press(t, LogicalKeyboardKey.keyN);
      expect(newNoteDialog, findsNothing);
      await press(t, LogicalKeyboardKey.keyN, shift: true);
      expect(launcher, findsNothing);
      await press(t, LogicalKeyboardKey.backslash);
      expect(c.read(sidebarCollapsedProvider), isFalse);
      expect(location(c), Routes.agentSession('ags_1'));
      await close(t, c);
    });

    for (final (label, key, shift) in [
      ('⌘K', LogicalKeyboardKey.keyK, false),
      ('⌘P', LogicalKeyboardKey.keyP, false),
      ('⇧⌘P', LogicalKeyboardKey.keyP, true),
    ]) {
      testWidgets('macOS: $label gets through', (t) async {
        final c = await focusedTerminal(t, TargetPlatform.macOS);
        await press(t, key, shift: shift);
        expect(location(c), Routes.search(v));
        await close(t, c);
      });
    }
  });

  group('the registry', () {
    test('Linux blocks Ctrl+K and Ctrl+P in the terminal, not Ctrl+Shift', () {
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      final blocked = stormTerminalBlockList().keys
          .cast<SingleActivator>()
          .map((a) => (a.trigger, a.control, a.shift))
          .toSet();
      expect(blocked, contains((LogicalKeyboardKey.keyK, true, false)));
      expect(blocked, contains((LogicalKeyboardKey.keyP, true, false)));
      expect(blocked, contains((LogicalKeyboardKey.keyN, true, true)));
      expect(blocked, isNot(contains((LogicalKeyboardKey.keyK, true, true))));
      expect(blocked, isNot(contains((LogicalKeyboardKey.keyP, true, true))));
    });

    test('macOS lets ⌘K, ⌘P and ⇧⌘P through', () {
      debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
      final blocked = stormTerminalBlockList().keys
          .cast<SingleActivator>()
          .map((a) => (a.trigger, a.meta, a.shift))
          .toSet();
      expect(blocked, isNot(contains((LogicalKeyboardKey.keyK, true, false))));
      expect(blocked, isNot(contains((LogicalKeyboardKey.keyP, true, false))));
      expect(blocked, isNot(contains((LogicalKeyboardKey.keyP, true, true))));
      expect(blocked, contains((LogicalKeyboardKey.keyN, true, false)));
    });

    test('ids are unique and chords are labelled per platform', () {
      debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
      final mac = stormCommands();
      expect(mac.map((c) => c.id).toSet().length, mac.length);
      expect(mac.firstWhere((c) => c.id == 'new-session').hint, '⇧⌘N');
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      final linux = stormCommands();
      expect(
        linux.firstWhere((c) => c.id == 'new-session').hint,
        'Ctrl+Shift+N',
      );
      expect(linux.firstWhere((c) => c.id == 'back').hint, 'Alt+←');
    });

    test('there are no destructive commands', () {
      for (final c in stormCommands()) {
        expect(c.id, isNot(matches(RegExp('delete|end|empty|trash-empty'))));
      }
    });
  });

  group('a phone', () {
    for (final size in [const Size(411, 900), at900]) {
      testWidgets('arms nothing at ${size.width.toInt()}', (t) async {
        debugDefaultTargetPlatformOverride = TargetPlatform.android;
        final c = await open(t, size, Routes.browse(v));
        await press(t, LogicalKeyboardKey.keyN);
        expect(newNoteDialog, findsNothing);
        await press(t, LogicalKeyboardKey.keyN, shift: true);
        expect(newFolderDialog, findsNothing);
        await press(t, LogicalKeyboardKey.keyK);
        expect(location(c), Routes.browse(v));
        await press(t, LogicalKeyboardKey.arrowLeft, mod: false, alt: true);
        expect(location(c), Routes.browse(v));
        await close(t, c);
      });
    }
  });
}

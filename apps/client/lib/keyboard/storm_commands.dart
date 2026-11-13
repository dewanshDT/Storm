import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../agent/agent_state.dart' show agentOverviewProvider;
import '../agent/agents_screen.dart' show launchAgentSession;
import '../agent/agents_shell.dart' show newSessionBlocker;
import '../router.dart';
import '../state/app_state.dart'
    show activeVaultProvider, openNoteIdProvider, sidebarCollapsedProvider;
import '../state/nav_memory.dart';
import '../ui/breakpoints.dart';
import '../ui/shell/app_bar.dart' show goBack, goForward;
import '../ui/palette/palette.dart' show openPalette;
import '../ui/shell/create_here.dart';
import 'storm_activators.dart';
import 'storm_intents.dart';

/// Where a command sits in a list of them.
enum StormCommandGroup { create, navigate, view, note }

/// One key combination for a command.
@immutable
class StormChord {
  const StormChord(this.activator, {this.terminalSafe = false});

  final SingleActivator activator;

  /// Taken by the app even while the terminal has focus; every other chord
  /// stops at the terminal (AM111). Per chord, not per command: Ctrl+K is
  /// the agent's on Linux while Ctrl+Shift+K, for the same command, is not.
  final bool terminalSafe;

  /// What identifies the key combination, so two commands can share one.
  String get id {
    final a = activator;
    return [
      if (a.control) 'ctrl',
      if (a.meta) 'meta',
      if (a.alt) 'alt',
      if (a.shift) 'shift',
      a.trigger.keyId,
    ].join('+');
  }

  /// How the chord is written on this platform: ⇧⌘N on macOS, Ctrl+Shift+N
  /// elsewhere.
  String get label {
    final a = activator;
    final key = _keyLabel(a.trigger);
    if (stormUsesMetaModifier) {
      return [
        if (a.control) '⌃',
        if (a.alt) '⌥',
        if (a.shift) '⇧',
        if (a.meta) '⌘',
        key,
      ].join();
    }
    return [
      if (a.control) 'Ctrl',
      if (a.alt) 'Alt',
      if (a.shift) 'Shift',
      if (a.meta) 'Super',
      key,
    ].join('+');
  }
}

String _keyLabel(LogicalKeyboardKey key) => switch (key) {
  LogicalKeyboardKey.arrowLeft => '←',
  LogicalKeyboardKey.arrowRight => '→',
  _ => key.keyLabel.toUpperCase(),
};

/// An app command: what the shortcut layer arms, the terminal stops and
/// the palette lists.
@immutable
class StormCommand {
  const StormCommand({
    required this.id,
    required this.label,
    required this.group,
    this.synonyms = const [],
    this.chords = const [],
    this.app = true,
    this.when,
    required this.run,
  });

  final String id;
  final String label;
  final List<String> synonyms;
  final StormCommandGroup group;
  final List<StormChord> chords;

  /// Whether [StormAppShortcuts] arms [chords]. A note's own chords are
  /// armed by the note screen, which owns what they act on.
  final bool app;

  /// Whether the command can run here; a chord whose commands all say no
  /// passes on to whatever else wants the key.
  final bool Function(BuildContext context, WidgetRef ref)? when;

  final void Function(BuildContext context, WidgetRef ref) run;

  bool enabled(BuildContext context, WidgetRef ref) =>
      when?.call(context, ref) ?? true;

  bool get terminalSafe =>
      chords.isNotEmpty && chords.every((c) => c.terminalSafe);

  /// The first chord's label, for a hint.
  String? get hint => chords.isEmpty ? null : chords.first.label;
}

Uri _location(BuildContext context) => GoRouter.of(context).state.uri;

Activity? _activity(BuildContext context) =>
    activityOf(_location(context).path);

bool _hasVault(WidgetRef ref) => ref.read(activeVaultProvider).isNotEmpty;

/// Whether the primary focus is in a text field, which keeps Alt+← and
/// Alt+→ for the caret off macOS.
bool _inTextField() =>
    primaryFocus?.context?.findAncestorWidgetOfExactType<EditableText>() !=
    null;

/// Back and forward (153c): ⌘[ and ⌘] on macOS, Alt+← and Alt+→ elsewhere,
/// none on the web, where the browser's own keys move its history.
List<StormChord> _navChords({required bool back}) {
  if (kIsWeb) return const [];
  if (stormUsesMetaModifier) {
    // The terminal never uses Meta, so it never needs these.
    return [
      StormChord(
        SingleActivator(
          back
              ? LogicalKeyboardKey.bracketLeft
              : LogicalKeyboardKey.bracketRight,
          meta: true,
        ),
        terminalSafe: true,
      ),
    ];
  }
  return [
    StormChord(
      SingleActivator(
        back ? LogicalKeyboardKey.arrowLeft : LogicalKeyboardKey.arrowRight,
        alt: true,
      ),
    ),
  ];
}

/// ⌘K, ⌘P and ⇧⌘P reach the app from a focused terminal on macOS; on Linux
/// and Windows Ctrl+K and Ctrl+P stay the agent's (readline), and
/// Ctrl+Shift+K and Ctrl+Shift+P are the way in (156c).
List<StormChord> _searchChords() {
  final mac = stormUsesMetaModifier;
  return [
    StormChord(stormActivator(LogicalKeyboardKey.keyK), terminalSafe: mac),
    StormChord(stormActivator(LogicalKeyboardKey.keyP), terminalSafe: mac),
    if (!mac)
      StormChord(
        stormActivator(LogicalKeyboardKey.keyK, shift: true),
        terminalSafe: true,
      ),
  ];
}

List<StormChord> _commandModeChords() => [
  StormChord(
    stormActivator(LogicalKeyboardKey.keyP, shift: true),
    terminalSafe: true,
  ),
];

/// The v1 commands (156f), with the chords they have on this platform.
/// Destructive acts are never commands; they stay on their surfaces.
List<StormCommand> stormCommands() => [
  StormCommand(
    id: 'search',
    label: 'Search',
    synonyms: const ['find', 'go to', 'open'],
    group: StormCommandGroup.navigate,
    chords: _searchChords(),
    run: (context, ref) => openPalette(context, ref),
  ),
  StormCommand(
    id: 'commands',
    label: 'Show commands',
    synonyms: const ['command palette', 'actions'],
    group: StormCommandGroup.navigate,
    chords: _commandModeChords(),
    run: (context, ref) => openPalette(context, ref, initial: '>'),
  ),
  StormCommand(
    id: 'new-note',
    label: 'New note',
    synonyms: const ['create note', 'add note'],
    group: StormCommandGroup.create,
    chords: [StormChord(stormActivator(LogicalKeyboardKey.keyN))],
    when: (_, ref) => _hasVault(ref),
    run: (context, ref) => createNote(context, ref),
  ),
  StormCommand(
    id: 'new-folder',
    label: 'New folder',
    synonyms: const ['create folder', 'add folder'],
    group: StormCommandGroup.create,
    chords: [StormChord(stormActivator(LogicalKeyboardKey.keyN, shift: true))],
    when: (context, ref) =>
        _hasVault(ref) && _activity(context) == Activity.notes,
    run: (context, ref) => createFolderInVault(context, ref),
  ),
  StormCommand(
    id: 'new-session',
    label: 'New session',
    synonyms: const ['start session', 'launch agent', 'new agent'],
    group: StormCommandGroup.create,
    chords: [StormChord(stormActivator(LogicalKeyboardKey.keyN, shift: true))],
    when: (context, _) => _activity(context) == Activity.agents,
    run: (context, ref) {
      if (newSessionBlocker(ref.read(agentOverviewProvider)) == null) {
        launchAgentSession(context, ref);
      }
    },
  ),
  StormCommand(
    id: 'toggle-sidebar',
    label: 'Toggle sidebar',
    synonyms: const ['hide sidebar', 'show sidebar'],
    group: StormCommandGroup.view,
    chords: [StormChord(stormActivator(LogicalKeyboardKey.backslash))],
    // There is no sidebar below the breakpoint to toggle.
    when: (context, _) => context.isExpanded,
    run: (_, ref) {
      final collapsed = ref.read(sidebarCollapsedProvider);
      ref.read(sidebarCollapsedProvider.notifier).state = !collapsed;
    },
  ),
  StormCommand(
    id: 'back',
    label: 'Back',
    group: StormCommandGroup.navigate,
    chords: _navChords(back: true),
    when: (_, _) => stormUsesMetaModifier || !_inTextField(),
    run: goBack,
  ),
  StormCommand(
    id: 'forward',
    label: 'Forward',
    group: StormCommandGroup.navigate,
    chords: _navChords(back: false),
    when: (_, _) => stormUsesMetaModifier || !_inTextField(),
    run: goForward,
  ),
  StormCommand(
    id: 'read-edit',
    label: 'Read / Edit',
    synonyms: const ['toggle edit', 'read mode', 'edit mode'],
    group: StormCommandGroup.note,
    chords: [StormChord(stormActivator(LogicalKeyboardKey.keyE))],
    app: false,
    when: (_, ref) => ref.read(openNoteIdProvider) != null,
    run: (_, _) => _invokeOnFocus(const StormToggleReadEditIntent()),
  ),
  StormCommand(
    id: 'find-in-note',
    label: 'Find in note',
    synonyms: const ['search note'],
    group: StormCommandGroup.note,
    chords: [StormChord(stormActivator(LogicalKeyboardKey.keyF))],
    app: false,
    when: (_, ref) => ref.read(openNoteIdProvider) != null,
    run: (_, _) => _invokeOnFocus(const StormFindInNoteIntent()),
  ),
  StormCommand(
    id: 'switch-vault',
    label: 'Switch vault…',
    synonyms: const ['change vault', 'vaults'],
    group: StormCommandGroup.navigate,
    run: (context, _) => context.go(Routes.settingsPage('vaults')),
  ),
  StormCommand(
    id: 'open-trash',
    label: 'Open Trash',
    synonyms: const ['deleted', 'bin'],
    group: StormCommandGroup.navigate,
    when: (_, ref) => _hasVault(ref),
    run: (context, ref) =>
        context.go(Routes.trash(ref.read(activeVaultProvider))),
  ),
  StormCommand(
    id: 'open-agents',
    label: 'Open Agents',
    synonyms: const ['sessions'],
    group: StormCommandGroup.navigate,
    run: (context, _) => context.go(Routes.agents),
  ),
  StormCommand(
    id: 'open-notes',
    label: 'Open Notes',
    group: StormCommandGroup.navigate,
    run: (context, _) => context.go(Routes.notes),
  ),
  StormCommand(
    id: 'open-settings',
    label: 'Open Settings',
    synonyms: const ['preferences'],
    group: StormCommandGroup.navigate,
    run: (context, _) => context.go(Routes.settings),
  ),
  StormCommand(
    id: 'theme',
    label: 'Theme',
    synonyms: const ['dark mode', 'light mode', 'appearance'],
    group: StormCommandGroup.view,
    run: (context, _) => context.go(Routes.settingsPage('device')),
  ),
];

void _invokeOnFocus(Intent intent) {
  final context = primaryFocus?.context;
  if (context != null) Actions.maybeInvoke(context, intent);
}

/// The app layer's chords, each to the intent [StormAppShortcuts] resolves
/// to the first of its commands that can run.
Map<ShortcutActivator, Intent> stormAppShortcuts(List<StormCommand> commands) {
  final byChord = <String, StormChord>{};
  for (final command in commands.where((c) => c.app)) {
    for (final chord in command.chords) {
      byChord.putIfAbsent(chord.id, () => chord);
    }
  }
  return {
    for (final chord in byChord.values)
      chord.activator: StormChordIntent(chord.id),
  };
}

/// The commands a chord can mean, in registry order.
List<StormCommand> commandsFor(List<StormCommand> commands, String chord) => [
  for (final c in commands)
    if (c.app && c.chords.any((k) => k.id == chord)) c,
];

/// What the terminal stops while it has focus: every app chord that is not
/// terminal safe, so Ctrl+K reaches the agent or nobody, never Storm.
Map<ShortcutActivator, Intent> stormTerminalBlockList() => {
  for (final command in stormCommands().where((c) => c.app))
    for (final chord in command.chords)
      if (!chord.terminalSafe)
        chord.activator: const DoNothingAndStopPropagationIntent(),
};

/// Sends a terminal-safe chord to the app before the terminal can encode
/// it: xterm2 would otherwise turn Ctrl+Shift+K into the agent's ^K.
/// Returns null for any other key, which the terminal then handles.
KeyEventResult? stormTerminalSafeKey(BuildContext context, KeyEvent event) {
  if (event is KeyUpEvent) return null;
  for (final command in stormCommands().where((c) => c.app)) {
    for (final chord in command.chords) {
      if (!chord.terminalSafe ||
          !chord.activator.accepts(event, HardwareKeyboard.instance)) {
        continue;
      }
      final intent = StormChordIntent(chord.id);
      final action = Actions.maybeFind<StormChordIntent>(
        context,
        intent: intent,
      );
      if (action == null || !action.isEnabled(intent)) return null;
      Actions.invoke(context, intent);
      return KeyEventResult.handled;
    }
  }
  return null;
}

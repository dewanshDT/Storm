import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'storm_activators.dart' show stormHasKeyboard;
import 'storm_commands.dart';
import 'storm_intents.dart';

/// The app's chords on every signed-in screen, at every width, wherever a
/// physical keyboard exists (156d); a phone arms nothing.
///
/// The chords and what they mean come from [stormCommands]. Note and editor
/// chords stay with the note, below this.
///
/// It also keeps focus inside itself: when primary focus is lost (a focused
/// page detached, a field let go to a scope above), no chord would fire, so
/// the layer takes focus back unless a dialog or popover is up.
class StormAppShortcuts extends ConsumerStatefulWidget {
  const StormAppShortcuts({super.key, required this.child});

  final Widget child;

  @override
  ConsumerState<StormAppShortcuts> createState() => _StormAppShortcutsState();
}

class _StormAppShortcutsState extends ConsumerState<StormAppShortcuts> {
  final _node = FocusNode(
    debugLabel: 'storm-app-shortcuts',
    skipTraversal: true,
  );
  ModalRoute<Object?>? _route;
  bool _scheduled = false;

  @override
  void initState() {
    super.initState();
    if (!stormHasKeyboard) return;
    FocusManager.instance.addListener(_onFocusChange);
    _onFocusChange();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _route = ModalRoute.of(context);
  }

  @override
  void dispose() {
    FocusManager.instance.removeListener(_onFocusChange);
    _node.dispose();
    super.dispose();
  }

  void _onFocusChange() {
    if (_scheduled) return;
    _scheduled = true;
    // After the frame: focus moves while routes and pages are still being
    // torn down, and the answer is only settled once they are.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _scheduled = false;
      if (mounted && _lost()) _node.requestFocus();
    });
    WidgetsBinding.instance.ensureVisualUpdate();
  }

  /// Focus is nowhere the layer hears keys from, and nothing above the
  /// shell is holding it on purpose.
  bool _lost() {
    if (_node.hasFocus) return false;
    // A dialog or popover on the root navigator: the shell's route is not
    // the top one. One on a pane's own navigator holds focus inside the
    // layer, so it never gets here.
    return _route?.isCurrent ?? true;
  }

  @override
  Widget build(BuildContext context) {
    if (!stormHasKeyboard) return widget.child;
    final commands = stormCommands();
    return Shortcuts(
      shortcuts: stormAppShortcuts(commands),
      child: Actions(
        actions: {
          StormChordIntent: _ChordAction(
            commands: commands,
            context: context,
            ref: ref,
          ),
        },
        child: Focus(focusNode: _node, child: widget.child),
      ),
    );
  }
}

class _ChordAction extends Action<StormChordIntent> {
  _ChordAction({
    required this.commands,
    required this.context,
    required this.ref,
  });

  final List<StormCommand> commands;
  final BuildContext context;
  final WidgetRef ref;

  StormCommand? _target(StormChordIntent intent) {
    if (!context.mounted) return null;
    for (final command in commandsFor(commands, intent.chord)) {
      if (command.enabled(context, ref)) return command;
    }
    return null;
  }

  @override
  bool isEnabled(StormChordIntent intent) => _target(intent) != null;

  @override
  Object? invoke(StormChordIntent intent) {
    _target(intent)?.run(context, ref);
    return null;
  }
}

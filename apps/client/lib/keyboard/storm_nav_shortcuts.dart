import 'package:flutter/widgets.dart';

import 'storm_activators.dart' show stormUsesMetaModifier;
import 'storm_intents.dart';
import 'storm_shortcut_maps.dart';

/// Back and forward from the keyboard (153c), around the whole shell.
///
/// Alt+← and Alt+→ move the caret to a line's ends in a text field on
/// Windows and Linux, so there the field keeps them; the terminal takes its
/// own keys before they reach here.
class StormNavShortcuts extends StatelessWidget {
  const StormNavShortcuts({
    super.key,
    required this.onBack,
    required this.onForward,
    required this.child,
  });

  final VoidCallback onBack;
  final VoidCallback onForward;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Shortcuts(
      shortcuts: stormNavShortcuts(),
      child: Actions(
        actions: {
          StormBackIntent: _NavAction<StormBackIntent>(onBack),
          StormForwardIntent: _NavAction<StormForwardIntent>(onForward),
        },
        child: child,
      ),
    );
  }
}

class _NavAction<T extends Intent> extends Action<T> {
  _NavAction(this.run);

  final VoidCallback run;

  @override
  bool isEnabled(T intent) =>
      stormUsesMetaModifier ||
      primaryFocus?.context?.findAncestorWidgetOfExactType<EditableText>() ==
          null;

  @override
  Object? invoke(T intent) {
    run();
    return null;
  }
}

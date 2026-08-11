import 'package:flutter/material.dart';

import '../editor/storm_markdown_controller.dart';
import 'storm_intents.dart';
import 'storm_shortcut_maps.dart';

/// Editor-level chords: bold / italic.
///
/// Wrap only the body [TextField] so search fields, dialogs and property
/// inputs keep OS/browser shortcuts. Undo/redo stay with Flutter defaults.
class StormEditorShortcuts extends StatelessWidget {
  const StormEditorShortcuts({
    super.key,
    required this.controller,
    required this.child,
  });

  final StormMarkdownController controller;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Shortcuts(
      shortcuts: stormEditorShortcuts(),
      child: Actions(
        actions: {
          StormBoldIntent: CallbackAction<StormBoldIntent>(
            onInvoke: (_) {
              controller.toggleInline('**');
              return null;
            },
          ),
          StormItalicIntent: CallbackAction<StormItalicIntent>(
            onInvoke: (_) {
              controller.toggleInline('*');
              return null;
            },
          ),
        },
        child: child,
      ),
    );
  }
}

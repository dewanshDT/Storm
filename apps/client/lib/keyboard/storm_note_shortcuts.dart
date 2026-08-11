import 'package:flutter/material.dart';

import 'storm_intents.dart';
import 'storm_shortcut_maps.dart';

/// Note-level chords: read/edit, save, find, escape.
///
/// Callbacks are injected by the note editor so this stays a thin
/// Shortcuts/Actions shell with no Riverpod dependency.
class StormNoteShortcuts extends StatelessWidget {
  const StormNoteShortcuts({
    super.key,
    required this.child,
    required this.onToggleReadEdit,
    required this.onSave,
    required this.onFind,
    required this.onDismiss,
  });

  final Widget child;
  final VoidCallback onToggleReadEdit;
  final VoidCallback onSave;
  final VoidCallback onFind;
  final VoidCallback onDismiss;

  @override
  Widget build(BuildContext context) {
    return Shortcuts(
      shortcuts: stormNoteShortcuts(),
      child: Actions(
        actions: {
          StormToggleReadEditIntent: CallbackAction<StormToggleReadEditIntent>(
            onInvoke: (_) {
              onToggleReadEdit();
              return null;
            },
          ),
          StormSaveNoteIntent: CallbackAction<StormSaveNoteIntent>(
            onInvoke: (_) {
              onSave();
              return null;
            },
          ),
          StormFindInNoteIntent: CallbackAction<StormFindInNoteIntent>(
            onInvoke: (_) {
              onFind();
              return null;
            },
          ),
          StormDismissIntent: CallbackAction<StormDismissIntent>(
            onInvoke: (_) {
              onDismiss();
              return null;
            },
          ),
        },
        child: child,
      ),
    );
  }
}

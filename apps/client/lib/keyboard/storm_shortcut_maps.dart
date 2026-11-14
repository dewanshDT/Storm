import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'storm_activators.dart';
import 'storm_intents.dart';

/// The note's and the editor's chord maps; the app layer's come from the
/// command registry (`storm_commands.dart`).
///
/// Undo/redo and select-all stay with Flutter's
/// [DefaultTextEditingShortcuts] when a [TextField] has focus — do not add
/// them here or they fight the editor.

/// How ⇧⌘N is written on this platform, for the ＋'s tooltip.
String get stormNewSessionChord =>
    stormUsesMetaModifier ? '⇧⌘N' : 'Ctrl+Shift+N';

Map<ShortcutActivator, Intent> stormNoteShortcuts() => {
  stormActivator(LogicalKeyboardKey.keyE): const StormToggleReadEditIntent(),
  stormActivator(LogicalKeyboardKey.keyS): const StormSaveNoteIntent(),
  stormActivator(LogicalKeyboardKey.keyF): const StormFindInNoteIntent(),
  const SingleActivator(LogicalKeyboardKey.escape): const StormDismissIntent(),
};

Map<ShortcutActivator, Intent> stormEditorShortcuts() => {
  stormActivator(LogicalKeyboardKey.keyB): const StormBoldIntent(),
  stormActivator(LogicalKeyboardKey.keyI): const StormItalicIntent(),
};

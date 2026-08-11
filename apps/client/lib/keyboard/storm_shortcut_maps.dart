import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'storm_activators.dart';
import 'storm_intents.dart';

/// Shortcut maps for the three nesting levels in M18.
///
/// Undo/redo and select-all stay with Flutter's
/// [DefaultTextEditingShortcuts] when a [TextField] has focus — do not add
/// them here or they fight the editor.

Map<ShortcutActivator, Intent> stormGlobalShortcuts() => {
  stormActivator(LogicalKeyboardKey.keyK): const StormSearchIntent(),
  stormActivator(LogicalKeyboardKey.keyN): const StormNewNoteIntent(),
  stormActivator(LogicalKeyboardKey.keyN, shift: true):
      const StormNewFolderIntent(),
  stormActivator(LogicalKeyboardKey.backslash):
      const StormToggleSidebarIntent(),
};

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

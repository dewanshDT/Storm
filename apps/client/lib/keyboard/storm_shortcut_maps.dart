import 'package:flutter/foundation.dart';
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

/// The app bar's back and forward (153c): ⌘[ and ⌘] on macOS, Alt+← and
/// Alt+→ elsewhere. None on the web, where the browser's own keys already
/// move its history and the bar follows it.
Map<ShortcutActivator, Intent> stormNavShortcuts() => kIsWeb
    ? const {}
    : stormUsesMetaModifier
    ? {
        const SingleActivator(LogicalKeyboardKey.bracketLeft, meta: true):
            const StormBackIntent(),
        const SingleActivator(LogicalKeyboardKey.bracketRight, meta: true):
            const StormForwardIntent(),
      }
    : {
        const SingleActivator(LogicalKeyboardKey.arrowLeft, alt: true):
            const StormBackIntent(),
        const SingleActivator(LogicalKeyboardKey.arrowRight, alt: true):
            const StormForwardIntent(),
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

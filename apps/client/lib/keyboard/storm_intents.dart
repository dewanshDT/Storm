import 'package:flutter/widgets.dart';

/// App / note / editor intents for Storm's desktop keyboard layer.
///
/// Actions live next to the widgets that can fulfill them; this file only
/// names the intents so [Shortcuts] maps stay typed and shared.

// --- App-wide ------------------------------------------------------------

/// A chord of the app layer, by its `StormChord.id`. One chord can mean several
/// commands (⇧⌘N is New folder in Notes, New session in Agents); the layer
/// runs the first that can run where you are.
class StormChordIntent extends Intent {
  const StormChordIntent(this.chord);

  final String chord;
}

// --- Note open ------------------------------------------------------------

class StormToggleReadEditIntent extends Intent {
  const StormToggleReadEditIntent();
}

class StormSaveNoteIntent extends Intent {
  const StormSaveNoteIntent();
}

class StormFindInNoteIntent extends Intent {
  const StormFindInNoteIntent();
}

class StormDismissIntent extends Intent {
  const StormDismissIntent();
}

// --- Editor focus only ----------------------------------------------------

class StormBoldIntent extends Intent {
  const StormBoldIntent();
}

class StormItalicIntent extends Intent {
  const StormItalicIntent();
}

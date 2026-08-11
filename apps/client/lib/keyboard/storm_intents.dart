import 'package:flutter/widgets.dart';

/// Global / note / editor intents for Storm's desktop keyboard layer.
///
/// Actions live next to the widgets that can fulfill them; this file only
/// names the intents so [Shortcuts] maps stay typed and shared.

// --- Global (vault shell) -------------------------------------------------

class StormSearchIntent extends Intent {
  const StormSearchIntent();
}

class StormNewNoteIntent extends Intent {
  const StormNewNoteIntent();
}

class StormNewFolderIntent extends Intent {
  const StormNewFolderIntent();
}

class StormToggleSidebarIntent extends Intent {
  const StormToggleSidebarIntent();
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

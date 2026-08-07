import 'package:flutter/material.dart';

import 'search_panel.dart';
import 'note_screen.dart';
import 'shell/storm_scaffold.dart';

/// Search, full screen.
///
/// Wraps the existing [SearchPanel] rather than reimplementing it — the panel
/// already owns debouncing and snippet rendering.
class SearchScreen extends StatelessWidget {
  const SearchScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return StormScaffold(
      title: const Text('Search'),
      child: Padding(
        padding: const EdgeInsets.only(bottom: 96),
        child: SearchPanel(onOpen: (note) => openNote(context, note.id)),
      ),
    );
  }
}

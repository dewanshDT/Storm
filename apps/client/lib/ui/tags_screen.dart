import 'package:flutter/material.dart';

import 'note_screen.dart';
import 'shell/storm_scaffold.dart';
import 'tags_panel.dart';

/// The tag browser, full screen. Reuses the existing [TagsPanel].
class TagsScreen extends StatelessWidget {
  const TagsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return StormScaffold(
      title: const Text('Tags'),
      child: Padding(
        padding: const EdgeInsets.only(bottom: 96),
        child: TagsPanel(onOpen: (note) => openNote(context, note.id)),
      ),
    );
  }
}

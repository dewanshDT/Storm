import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import 'browse_screen.dart';
import 'note_screen.dart';
import 'shell/vault_gate.dart';
import 'surfaces.dart';
import 'tags_panel.dart';

/// The tag browser: a sheet over the directory, as the design draws it.
///
/// Still a route, so `/v/<vault>/tags` keeps resolving for a deep link.
class TagsScreen extends StatelessWidget {
  const TagsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final vaultId = VaultGate.of(context);
    return SheetHost(
      title: 'Tags',
      backdrop: const BrowseScreen(folder: ''),
      // Back out the way the route came in, so closing the sheet does not
      // leave a destination on the stack the nav bubble never pushed.
      onDismiss: () =>
          context.canPop() ? context.pop() : context.go(Routes.browse(vaultId)),
      builder: (sheetContext) => TagsPanel(
        onOpen: (note) {
          Navigator.of(sheetContext).pop();
          openNote(context, note.id);
        },
      ),
    );
  }
}

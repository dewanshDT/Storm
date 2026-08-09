import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import 'note_screen.dart';
import 'search_panel.dart';
import 'shell/storm_scaffold.dart';
import 'shell/vault_gate.dart';

/// Search, full-bleed over the vault.
///
/// Wraps [SearchPanel] rather than reimplementing it — the panel already owns
/// debouncing and snippet rendering.
class SearchScreen extends StatelessWidget {
  const SearchScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final vaultId = VaultGate.of(context);
    void close() =>
        context.canPop() ? context.pop() : context.go(Routes.browse(vaultId));

    return StormScaffold(
      child: SearchPanel(
        onOpen: (note) => openNote(context, note.id),
        onClose: close,
      ),
    );
  }
}

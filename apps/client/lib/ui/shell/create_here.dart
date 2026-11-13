import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart' show NoteMeta;
import '../../router.dart';
import '../../state/app_state.dart';
import '../../state/vault_config.dart' show kColorKey;
import '../browse_screen.dart' show createFolder;
import '../new_note_dialog.dart';
import 'vault_sidebar.dart' show openNoteIdOf;

/// When the page on screen was arrived at. A folder picked in the sidebar is
/// New note's "here" only while that pick is newer than the page.
class PageArrival {
  Uri? _page;
  DateTime _at = DateTime.now();

  DateTime get at => _at;

  void visit(Uri page) {
    if (page == _page) return;
    _page = page;
    _at = DateTime.now();
  }
}

/// Written by the shell as it builds, read by the create commands; it never
/// notifies, so nothing rebuilds on it.
final pageArrivalProvider = Provider<PageArrival>((ref) => PageArrival());

/// Where a new note or folder goes in [vaultId]: whichever was picked last,
/// a folder row clicked in the sidebar or the page itself (an opened folder,
/// or an open note's folder); otherwise the top of the vault.
String newItemFolder(WidgetRef ref, Uri uri, String vaultId) {
  final picked = ref.read(pickedFolderProvider);
  if (picked != null &&
      picked.vault == vaultId &&
      picked.at.isAfter(ref.read(pageArrivalProvider).at)) {
    return picked.folder;
  }
  if (Routes.vaultOf(uri) != vaultId) return '';
  final noteId = openNoteIdOf(uri);
  if (noteId != null) {
    final notes = ref.read(treeProvider).value ?? const <NoteMeta>[];
    final note = notes.where((n) => n.id == noteId).firstOrNull;
    if (note != null) return note.folder;
  }
  return Routes.folderOf(uri);
}

/// New note in the active vault, from any screen.
Future<void> createNote(BuildContext context, WidgetRef ref) async {
  final vaultId = ref.read(activeVaultProvider);
  if (vaultId.isEmpty) return;
  final router = GoRouter.of(context);
  final uri = router.state.uri;
  final here = newItemFolder(ref, uri, vaultId);
  NoteMeta? made;
  final wanted = await promptForNewNote(
    context,
    folder: here,
    create: (note) async {
      final path = noteFileName(note.name, folder: here);
      final notes = ref.read(treeProvider).value ?? const <NoteMeta>[];
      if (notes.any((n) => n.path.toLowerCase() == path.toLowerCase())) {
        return 'A note with that name is already '
            '${newNoteDestination(path)}.';
      }
      final created = await ref
          .read(syncEngineProvider)
          .create(
            path: path,
            // A colour chosen up front is just the note's first property.
            content: note.accent.isNone
                ? ''
                : '---\n$kColorKey: ${note.accent.name}\n---\n\n',
          );
      made = created.meta;
      if (made != null) return null;
      final why = (created.error ?? 'the server did not make the note')
          .replaceAll(RegExp(r'\.+$'), '');
      return 'Nothing was created: $why.';
    },
  );
  final note = made;
  if (wanted == null || note == null || !context.mounted) return;
  ref.invalidate(treeProvider);
  final to = Routes.note(vaultId, note.id);
  // Pushed inside the vault so back returns to the folder; from another
  // activity a push would stack a second shell.
  if (Routes.vaultOf(router.state.uri) == vaultId) {
    router.push(to);
  } else {
    router.go(to);
  }
}

/// New folder in the active vault, beside wherever New note would go.
Future<void> createFolderInVault(BuildContext context, WidgetRef ref) async {
  final vaultId = ref.read(activeVaultProvider);
  if (vaultId.isEmpty) return;
  final here = newItemFolder(ref, GoRouter.of(context).state.uri, vaultId);
  await createFolder(context, ref, vaultId, here);
}

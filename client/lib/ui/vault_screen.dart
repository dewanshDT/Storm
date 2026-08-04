import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../state/app_state.dart';
import 'note_editor.dart';
import 'search_panel.dart';
import 'vault_tree.dart';

/// The main shell: vault tree on the left, editor on the right.
///
/// On narrow screens the tree becomes a drawer, since M2 already has to run on
/// web at phone widths.
class VaultScreen extends ConsumerStatefulWidget {
  const VaultScreen({super.key});

  @override
  ConsumerState<VaultScreen> createState() => _VaultScreenState();
}

class _VaultScreenState extends ConsumerState<VaultScreen> {
  final _scaffold = GlobalKey<ScaffoldState>();
  bool _searching = false;

  Future<void> _open(NoteMeta note) async {
    // Flush any pending edit before switching away, or the debounce timer
    // would fire against a note that is no longer open.
    final session = ref.read(noteSessionProvider);
    if (session.isDirty) await session.save();

    ref.read(openNoteIdProvider.notifier).state = note.id;
    await ref.read(noteSessionProvider).open(note.id);
    if (mounted && _scaffold.currentState?.isDrawerOpen == true) {
      Navigator.of(context).pop();
    }
  }

  Future<void> _createNote() async {
    final path = await _promptForPath(
      context,
      title: 'New note',
      initial: 'Untitled.md',
    );
    if (path == null) return;

    final api = ref.read(apiProvider);
    if (api == null) return;
    try {
      final result = await api.createNote(path: path, content: '');
      ref.invalidate(treeProvider);
      await _open(result.meta);
    } on StormApiException catch (e) {
      _toast(e.message);
    }
  }

  Future<void> _renameNote() async {
    final session = ref.read(noteSessionProvider);
    final note = session.note;
    if (note == null) return;

    final path = await _promptForPath(
      context,
      title: 'Rename or move',
      initial: note.meta.path,
    );
    if (path == null || path == note.meta.path) return;

    final api = ref.read(apiProvider);
    if (api == null) return;
    try {
      if (session.isDirty) await session.save();
      final result = await api.moveNote(id: note.meta.id, newPath: path);
      ref.invalidate(treeProvider);
      await ref.read(noteSessionProvider).open(result.meta.id);
    } on StormApiException catch (e) {
      _toast(e.message);
    }
  }

  Future<void> _deleteNote() async {
    final note = ref.read(noteSessionProvider).note;
    if (note == null) return;

    final confirmed = await showDialog<bool>(
      context: context,
      builder: (c) => AlertDialog(
        title: const Text('Delete note?'),
        content: Text(
          '“${note.meta.path}” will be removed from the vault on the server.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(c, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(c, true),
            style: FilledButton.styleFrom(
              backgroundColor: Theme.of(context).colorScheme.error,
            ),
            child: const Text('Delete'),
          ),
        ],
      ),
    );
    if (confirmed != true) return;

    final api = ref.read(apiProvider);
    if (api == null) return;
    try {
      await api.deleteNote(note.meta.id);
      ref.read(noteSessionProvider).close();
      ref.read(openNoteIdProvider.notifier).state = null;
      ref.invalidate(treeProvider);
    } on StormApiException catch (e) {
      _toast(e.message);
    }
  }

  void _toast(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(context)
        .showSnackBar(SnackBar(content: Text(message)));
  }

  @override
  Widget build(BuildContext context) {
    final wide = MediaQuery.sizeOf(context).width >= 820;
    final hasNote = ref.watch(noteSessionProvider).isOpen;

    final sidebar = Column(
      children: [
        Expanded(
          child: _searching
              ? SearchPanel(onOpen: _open)
              : VaultTreePanel(onOpen: _open),
        ),
      ],
    );

    return Scaffold(
      key: _scaffold,
      appBar: AppBar(
        title: const Text('Storm'),
        leading: wide
            ? null
            : IconButton(
                icon: const Icon(Icons.menu),
                onPressed: () => _scaffold.currentState?.openDrawer(),
              ),
        actions: [
          IconButton(
            tooltip: _searching ? 'Browse vault' : 'Search',
            icon: Icon(_searching ? Icons.folder_outlined : Icons.search),
            onPressed: () => setState(() => _searching = !_searching),
          ),
          IconButton(
            tooltip: 'New note',
            icon: const Icon(Icons.note_add_outlined),
            onPressed: _createNote,
          ),
          if (hasNote) ...[
            IconButton(
              tooltip: 'Rename or move',
              icon: const Icon(Icons.drive_file_rename_outline),
              onPressed: _renameNote,
            ),
            IconButton(
              tooltip: 'Delete',
              icon: const Icon(Icons.delete_outline),
              onPressed: _deleteNote,
            ),
          ],
          IconButton(
            tooltip: 'Refresh',
            icon: const Icon(Icons.refresh),
            onPressed: () => ref.invalidate(treeProvider),
          ),
          const _SettingsButton(),
        ],
      ),
      drawer: wide ? null : Drawer(child: SafeArea(child: sidebar)),
      body: Row(
        children: [
          if (wide) ...[
            SizedBox(
              width: 280,
              child: Container(
                decoration: BoxDecoration(
                  border: Border(
                    right: BorderSide(color: Theme.of(context).dividerColor),
                  ),
                ),
                child: sidebar,
              ),
            ),
          ],
          const Expanded(child: NoteEditor()),
        ],
      ),
    );
  }
}

/// Prompts for a vault-relative path, validating it client-side.
///
/// The server rejects bad paths too, but catching it here avoids a round trip
/// and gives a clearer message.
Future<String?> _promptForPath(
  BuildContext context, {
  required String title,
  required String initial,
}) async {
  final controller = TextEditingController(text: initial);
  String? error;

  final result = await showDialog<String>(
    context: context,
    builder: (c) => StatefulBuilder(
      builder: (c, setState) {
        void submit() {
          final value = controller.text.trim();
          final problem = validatePath(value);
          if (problem != null) {
            setState(() => error = problem);
            return;
          }
          Navigator.pop(c, value.endsWith('.md') ? value : '$value.md');
        }

        return AlertDialog(
          title: Text(title),
          content: TextField(
            controller: controller,
            autofocus: true,
            decoration: InputDecoration(
              labelText: 'Path in vault',
              hintText: 'Folder/Note.md',
              errorText: error,
              border: const OutlineInputBorder(),
            ),
            onSubmitted: (_) => submit(),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(c),
              child: const Text('Cancel'),
            ),
            FilledButton(onPressed: submit, child: const Text('OK')),
          ],
        );
      },
    ),
  );
  controller.dispose();
  return result;
}

/// Returns a human-readable problem with [path], or `null` if it is fine.
///
/// Mirrors the server's rules so the two can't drift into disagreeing about
/// what a legal path is.
String? validatePath(String path) {
  if (path.isEmpty) return 'Enter a path';
  if (path.startsWith('/')) return 'Use a path relative to the vault';
  final segments = path.split('/');
  if (segments.any((s) => s == '..')) return "Paths can't contain “..”";
  if (segments.any((s) => s.isEmpty)) return 'Empty folder name';
  if (segments.any((s) => s.startsWith('.'))) {
    return "Names can't start with a dot";
  }
  return null;
}

class _SettingsButton extends ConsumerWidget {
  const _SettingsButton();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final settings = ref.watch(settingsProvider).value ?? const Settings();
    return PopupMenuButton<String>(
      icon: const Icon(Icons.settings_outlined),
      onSelected: (choice) async {
        final notifier = ref.read(settingsProvider.notifier);
        switch (choice) {
          case 'theme':
            await notifier.save(settings.copyWith(darkMode: !settings.darkMode));
          case 'bigger':
            await notifier
                .save(settings.copyWith(fontSize: (settings.fontSize + 1).clamp(11, 26)));
          case 'smaller':
            await notifier
                .save(settings.copyWith(fontSize: (settings.fontSize - 1).clamp(11, 26)));
          case 'disconnect':
            await notifier.save(const Settings());
        }
      },
      itemBuilder: (c) => [
        PopupMenuItem(
          value: 'theme',
          child: Text(settings.darkMode ? 'Light theme' : 'Dark theme'),
        ),
        const PopupMenuItem(value: 'bigger', child: Text('Larger text')),
        const PopupMenuItem(value: 'smaller', child: Text('Smaller text')),
        const PopupMenuDivider(),
        const PopupMenuItem(value: 'disconnect', child: Text('Disconnect')),
      ],
    );
  }
}

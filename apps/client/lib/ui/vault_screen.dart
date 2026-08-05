import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../state/app_state.dart';
import '../sync/sync_engine.dart';
import 'backlinks_panel.dart';
import 'note_editor.dart';
import 'search_panel.dart';
import 'tags_panel.dart';
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

/// Which view the sidebar is showing.
enum _Sidebar { files, search, tags }

class _VaultScreenState extends ConsumerState<VaultScreen> {
  final _scaffold = GlobalKey<ScaffoldState>();
  _Sidebar _sidebar = _Sidebar.files;

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
    final note = session.meta;
    if (note == null) return;

    final path = await _promptForPath(
      context,
      title: 'Rename or move',
      initial: note.path,
    );
    if (path == null || path == note.path) return;

    if (session.isDirty) await session.save();
    final outcome = await ref
        .read(syncEngineProvider)
        .move(id: note.id, newPath: path);
    if (outcome.status == SaveStatus.failed) {
      _toast(outcome.error ?? 'Could not move the note');
      return;
    }
    if (outcome.status == SaveStatus.queued) {
      _toast('Offline — the move will sync when the server is back');
    }
    ref.invalidate(treeProvider);
    await ref.read(noteSessionProvider).open(note.id);
  }

  Future<void> _deleteNote() async {
    final note = ref.read(noteSessionProvider).meta;
    if (note == null) return;

    final confirmed = await showDialog<bool>(
      context: context,
      builder: (c) => AlertDialog(
        title: const Text('Delete note?'),
        content: Text(
          '“${note.path}” will be removed from the vault on the server.',
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
      await api.deleteNote(note.id);
      ref.read(noteSessionProvider).close();
      ref.read(openNoteIdProvider.notifier).state = null;
      ref.invalidate(treeProvider);
    } on StormApiException catch (e) {
      _toast(e.message);
    }
  }

  void _toast(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text(message)));
  }

  @override
  Widget build(BuildContext context) {
    // Providers are lazy: without this nothing subscribes to the engine's
    // change stream, so remote edits would never reach the open editor and the
    // tree would never refresh after a sync.
    ref.watch(syncListenerProvider);

    final wide = MediaQuery.sizeOf(context).width >= 820;
    final hasNote = ref.watch(noteSessionProvider).isOpen;

    final openId = ref.watch(openNoteIdProvider);

    final sidebar = Column(
      children: [
        _SidebarTabs(
          current: _sidebar,
          onChanged: (mode) => setState(() => _sidebar = mode),
        ),
        const Divider(height: 1),
        Expanded(
          child: switch (_sidebar) {
            _Sidebar.files => VaultTreePanel(onOpen: _open),
            _Sidebar.search => SearchPanel(onOpen: _open),
            _Sidebar.tags => TagsPanel(onOpen: _open),
          },
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
          const _SyncStatus(),
          IconButton(
            tooltip: 'Sync now',
            icon: const Icon(Icons.refresh),
            onPressed: () async {
              await ref.read(syncEngineProvider).sync();
              ref.invalidate(treeProvider);
            },
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
          Expanded(
            child: Column(
              children: [
                const Expanded(child: NoteEditor()),
                if (openId != null)
                  BacklinksPanel(noteId: openId, onOpen: _open),
              ],
            ),
          ),
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

/// Files / Search / Tags switcher for the sidebar.
class _SidebarTabs extends StatelessWidget {
  const _SidebarTabs({required this.current, required this.onChanged});

  final _Sidebar current;
  final void Function(_Sidebar) onChanged;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    Widget tab(_Sidebar mode, IconData icon, String tooltip) {
      final selected = mode == current;
      return Expanded(
        child: Tooltip(
          message: tooltip,
          child: InkWell(
            onTap: () => onChanged(mode),
            child: Container(
              padding: const EdgeInsets.symmetric(vertical: 10),
              decoration: BoxDecoration(
                border: Border(
                  bottom: BorderSide(
                    width: 2,
                    color: selected
                        ? theme.colorScheme.primary
                        : Colors.transparent,
                  ),
                ),
              ),
              child: Icon(
                icon,
                size: 18,
                color: selected
                    ? theme.colorScheme.primary
                    : theme.colorScheme.onSurfaceVariant,
              ),
            ),
          ),
        ),
      );
    }

    return Row(
      children: [
        tab(_Sidebar.files, Icons.folder_outlined, 'Files'),
        tab(_Sidebar.search, Icons.search, 'Search'),
        tab(_Sidebar.tags, Icons.label_outline, 'Tags'),
      ],
    );
  }
}

/// Connection and outbox state.
///
/// Offline is never silent: an edit that only exists in the outbox is one the
/// server has not seen, and the user needs to know that before closing the app.
class _SyncStatus extends ConsumerWidget {
  const _SyncStatus();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final engine = ref.watch(syncEngineProvider);
    final scheme = Theme.of(context).colorScheme;

    if (engine.isSyncing) {
      return const Padding(
        padding: EdgeInsets.symmetric(horizontal: 14),
        child: Center(
          child: SizedBox(
            height: 14,
            width: 14,
            child: CircularProgressIndicator(strokeWidth: 2),
          ),
        ),
      );
    }

    if (engine.isOnline && engine.pendingCount == 0) {
      return const SizedBox.shrink();
    }

    final offline = !engine.isOnline;
    final label = engine.pendingCount > 0
        ? '${engine.pendingCount} unsent'
        : 'Offline';

    return Tooltip(
      message: offline
          ? 'Cannot reach the server. Edits are saved locally and will sync '
                'when it comes back.'
          : 'Waiting to send queued edits.',
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8),
        child: Row(
          children: [
            Icon(
              offline ? Icons.cloud_off : Icons.cloud_upload_outlined,
              size: 16,
              color: offline ? scheme.error : scheme.tertiary,
            ),
            const SizedBox(width: 5),
            Text(
              label,
              style: TextStyle(
                fontSize: 12,
                color: offline ? scheme.error : scheme.tertiary,
              ),
            ),
          ],
        ),
      ),
    );
  }
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
            await notifier.save(
              settings.copyWith(darkMode: !settings.darkMode),
            );
          case 'bigger':
            await notifier.save(
              settings.copyWith(
                fontSize: (settings.fontSize + 1).clamp(11, 26),
              ),
            );
          case 'smaller':
            await notifier.save(
              settings.copyWith(
                fontSize: (settings.fontSize - 1).clamp(11, 26),
              ),
            );
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

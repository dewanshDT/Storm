import 'dart:async';

import 'package:drift/drift.dart' show Value;
import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../cache/cache_db.dart';
import '../router.dart';
import '../state/app_state.dart';
import '../editor/frontmatter_edit.dart' as fme;
import '../state/vault_config.dart';
import '../state/wikilinks.dart';
import 'accents.dart';
import 'tokens.dart';
import '../sync/sync_engine.dart';
import 'attachment_strip.dart';
import 'backlinks_panel.dart';
import 'note_editor.dart';
import 'shell/nav_bubble.dart';
import 'shell/storm_scaffold.dart';
import 'shell/vault_gate.dart';

/// One note, opened from a route.
///
/// The route owns which note is open: navigating here loads it, so a deep
/// link and a tap land in exactly the same state.
class NoteScreen extends ConsumerStatefulWidget {
  const NoteScreen({super.key, required this.noteId});

  final String noteId;

  @override
  ConsumerState<NoteScreen> createState() => _NoteScreenState();
}

class _NoteScreenState extends ConsumerState<NoteScreen> {
  bool _showContext = false;

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void didUpdateWidget(NoteScreen old) {
    super.didUpdateWidget(old);
    if (old.noteId != widget.noteId) _load();
  }

  void _load() {
    // After the frame: opening mutates providers, which cannot happen during
    // a build.
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      if (!mounted) return;
      final session = ref.read(noteSessionProvider);
      if (session.isDirty) await session.save();
      if (!mounted) return;
      ref.read(openNoteIdProvider.notifier).state = widget.noteId;
      await ref.read(noteSessionProvider).open(widget.noteId);
      if (!mounted) return;
      unawaited(_recordOpen());
    });
  }

  /// Tells the server this note was opened, for the dashboard's recents.
  ///
  /// Fire-and-forget, and never surfaced: failing to record an open is not
  /// worth a message, and must not stop the note from being read.
  Future<void> _recordOpen() async {
    final vaultId = ref.read(activeVaultProvider);
    final api = ref.read(apiProvider);
    if (api == null || vaultId.isEmpty) return;

    // Mirrored locally first so it shows immediately and survives being
    // offline; the server's copy wins at the next dashboard refresh.
    final meta = ref.read(noteSessionProvider).meta;
    if (meta != null) {
      await ref
          .read(cacheProvider)
          .noteOpened(
            RecentsCompanion.insert(
              vaultId: vaultId,
              noteId: meta.id,
              path: Value(meta.path),
              title: Value(meta.title),
              openedAt: DateTime.now(),
            ),
          );
    }
    try {
      await api.markOpened(vaultId, widget.noteId);
    } catch (_) {
      // Offline. The local mirror already carries it.
    }
  }

  void _toast(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text(message)));
  }

  /// Opens the note a `[[wikilink]]` points at.
  ///
  /// Saves first: following a link replaces the buffer, and an unsaved edit
  /// left behind would be lost. An unresolved link says so rather than
  /// creating a note — creating one silently is how a typo becomes a file.
  Future<void> _followLink(String target) async {
    final notes = ref.read(treeProvider).value ?? const [];
    final found = resolveWikilink(notes, target);
    if (found == null) {
      _toast('No note called “$target”');
      return;
    }
    if (found.id == widget.noteId) return;

    final session = ref.read(noteSessionProvider);
    if (session.isDirty) await session.save();
    if (!mounted) return;
    context.push(Routes.note(VaultGate.of(context), found.id));
  }

  Future<void> _togglePin() async {
    final pinned = ref.read(pinnedNotesProvider).value ?? const <String>{};
    final nowPinned = !pinned.contains(widget.noteId);
    await ref.read(syncEngineProvider).setPinned(widget.noteId, nowPinned);
    ref.invalidate(pinnedNotesProvider);
    _toast(nowPinned ? 'Kept available offline' : 'No longer kept offline');
  }

  Future<void> _attach() async {
    final session = ref.read(noteSessionProvider);
    if (!session.isOpen) return;

    final file = await openFile();
    if (file == null) return;
    final bytes = await file.readAsBytes();
    if (bytes.isEmpty) return;

    final result = await ref
        .read(syncEngineProvider)
        .attach(fileName: file.name, bytes: bytes);
    if (result.path == null) {
      _toast(result.error ?? 'Could not upload the attachment');
      return;
    }

    final ext = file.name.contains('.')
        ? file.name.split('.').last.toLowerCase()
        : '';
    final isImage = const {
      'png',
      'jpg',
      'jpeg',
      'gif',
      'webp',
      'bmp',
    }.contains(ext);
    final link = '${isImage ? '!' : ''}[${file.name}](${result.path})';

    final body = session.body;
    session.editBody(
      body.endsWith('\n') ? '$body\n$link\n' : '$body\n\n$link\n',
    );
    await session.save();
    _toast('Attached ${file.name}');
  }

  Future<void> _rename() async {
    final session = ref.read(noteSessionProvider);
    final note = session.meta;
    if (note == null) return;

    final path = await promptForPath(
      context,
      title: 'Rename or move',
      initial: note.path,
    );
    if (path == null || path == note.path || !mounted) return;

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

  Future<void> _delete() async {
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
    if (confirmed != true || !mounted) return;

    final api = ref.read(apiProvider);
    if (api == null) return;
    try {
      await api.deleteNote(ref.read(activeVaultProvider), note.id);
      ref.read(noteSessionProvider).close();
      ref.read(openNoteIdProvider.notifier).state = null;
      ref.invalidate(treeProvider);
      if (mounted) context.go(Routes.dashboard);
    } catch (e) {
      _toast('$e');
    }
  }

  @override
  Widget build(BuildContext context) {
    ref.watch(syncListenerProvider);
    final session = ref.watch(noteSessionProvider);
    final pinned = ref.watch(pinnedNotesProvider).value ?? const <String>{};
    final isPinned = pinned.contains(widget.noteId);
    // Asked here, above the Scaffold, where the inset is still readable and
    // depending on it still rebuilds. See keyboardIsOpen.
    final keyboard = keyboardIsOpen(context);
    final vaultId = VaultGate.of(context);
    // The note's own colour, from its `color:` property. A wash rather than
    // the full card tint: this sits behind a screen of prose, and the card
    // colour at full strength fights the text.
    final accent = Accent.parse(
      fme.findSpan(session.buffer, kColorKey)?.displayValue,
    );
    final tint = accent.isNone ? null : accent.wash(context.tokens);

    return NoteContextRequest(
      onRequest: () => setState(() => _showContext = !_showContext),
      child: NewNoteRequest(
        onRequest: () {},
        child: Scaffold(
          backgroundColor: tint,
          appBar: AppBar(
            backgroundColor: tint,
            leading: IconButton(
              icon: const Icon(Icons.arrow_back),
              onPressed: () => context.canPop()
                  ? context.pop()
                  : context.go(Routes.dashboard),
            ),
            title: Text(
              session.meta?.fileName ?? '',
              overflow: TextOverflow.ellipsis,
            ),
            actions: [
              // One menu, not five icons: an overflowing AppBar silently drops
              // what doesn't fit, which is how the attach button once appeared
              // to be missing.
              PopupMenuButton<VoidCallback>(
                tooltip: 'Note actions',
                onSelected: (action) => action(),
                itemBuilder: (c) => [
                  PopupMenuItem(
                    value: _togglePin,
                    child: ListTile(
                      dense: true,
                      contentPadding: EdgeInsets.zero,
                      leading: Icon(
                        isPinned ? Icons.push_pin : Icons.push_pin_outlined,
                        size: 20,
                      ),
                      title: Text(
                        isPinned ? 'Stop keeping offline' : 'Keep offline',
                      ),
                    ),
                  ),
                  PopupMenuItem(
                    value: _attach,
                    child: const ListTile(
                      dense: true,
                      contentPadding: EdgeInsets.zero,
                      leading: Icon(Icons.attach_file, size: 20),
                      title: Text('Attach a file'),
                    ),
                  ),
                  PopupMenuItem(
                    value: _rename,
                    child: const ListTile(
                      dense: true,
                      contentPadding: EdgeInsets.zero,
                      leading: Icon(Icons.drive_file_rename_outline, size: 20),
                      title: Text('Rename or move'),
                    ),
                  ),
                  const PopupMenuDivider(),
                  PopupMenuItem(
                    value: _delete,
                    child: ListTile(
                      dense: true,
                      contentPadding: EdgeInsets.zero,
                      leading: Icon(
                        Icons.delete_outline,
                        size: 20,
                        color: Theme.of(c).colorScheme.error,
                      ),
                      title: Text(
                        'Delete',
                        style: TextStyle(color: Theme.of(c).colorScheme.error),
                      ),
                    ),
                  ),
                ],
              ),
            ],
          ),
          body: Stack(
            children: [
              Positioned.fill(
                child: Column(
                  children: [
                    Expanded(
                      child: NoteEditor(
                        onFollowLink: _followLink,
                        showToolbar: keyboard,
                      ),
                    ),
                    AttachmentStrip(body: session.body),
                    if (_showContext)
                      BacklinksPanel(
                        noteId: widget.noteId,
                        onOpen: (note) =>
                            context.push(Routes.note(vaultId, note.id)),
                      ),
                  ],
                ),
              ),
              if (!keyboard)
                const Positioned(
                  left: 0,
                  right: 0,
                  bottom: 0,
                  child: NavBubble(),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Navigate to a note from anywhere within a vault.
void openNote(BuildContext context, String id) =>
    context.push(Routes.note(VaultGate.of(context), id));

import 'dart:async';

import 'package:drift/drift.dart' show Value;
import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../agent/agent_state.dart' show agentOverviewProvider;
import '../cache/cache_db.dart';
import '../router.dart';
import '../state/agent_writes.dart' show seenVersionsProvider;
import '../state/app_state.dart';
import '../state/note_session.dart' show NoteSession;
import '../editor/frontmatter_edit.dart' as fme;
import '../state/vault_config.dart';
import '../state/wikilinks.dart';
import 'accents.dart';
import 'tokens.dart';
import '../sync/sync_engine.dart';
import 'surfaces.dart';
import 'widgets.dart';
import 'mentions_section.dart';
import 'properties_panel.dart';
import 'breakpoints.dart';
import 'note_editor.dart';
import 'shell/nav_bubble.dart' show keyboardIsOpen;
import 'browse_screen.dart' show BackLink, leaveTo;
import 'note_header.dart';
import '../api/models.dart' show VaultInfo;
import 'shell/storm_scaffold.dart';
import 'shell/vault_gate.dart';

/// One note, opened from a route.
///
/// The route owns which note is open: navigating here loads it, so a deep
/// link and a tap land in exactly the same state.
class NoteScreen extends ConsumerStatefulWidget {
  const NoteScreen({super.key, required this.noteId, this.fromSession});

  final String noteId;

  /// The agent session this note was pushed from (phone), whose name the
  /// back link carries.
  final String? fromSession;

  @override
  ConsumerState<NoteScreen> createState() => _NoteScreenState();
}

class _NoteScreenState extends ConsumerState<NoteScreen> {
  @override
  void initState() {
    super.initState();
    _load();
  }

  /// Whatever version of this note is on screen counts as seen here, which
  /// is what clears its unseen dot.
  void _markSeen(NoteSession s) {
    if (s.noteId != widget.noteId || s.baseVersion <= 0) return;
    final vaultId = ref.read(activeVaultProvider);
    if (vaultId.isEmpty) return;
    ref
        .read(seenVersionsProvider.notifier)
        .markSeen(vaultId, widget.noteId, s.baseVersion);
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

  /// Tells the server this note was opened, for the cross-vault recents.
  ///
  /// Fire-and-forget, and never surfaced: failing to record an open is not
  /// worth a message, and must not stop the note from being read.
  Future<void> _recordOpen() async {
    final vaultId = ref.read(activeVaultProvider);
    final api = ref.read(apiProvider);
    if (api == null || vaultId.isEmpty) return;

    // Mirrored locally first so it shows immediately and survives being
    // offline; the server's copy wins at the next refresh.
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
    if (mounted) ref.invalidate(recentsProvider);
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
      if (mounted) context.go(Routes.browse(ref.read(activeVaultProvider)));
    } catch (e) {
      _toast('$e');
    }
  }

  String _sessionName(String id) =>
      ref.watch(agentOverviewProvider).value?.byId(id)?.name ?? 'Session';

  /// Back to where the note was opened from, else its folder.
  void _leaveNote() {
    final folder = ref.read(noteSessionProvider).meta?.folder ?? '';
    final vaultId = VaultGate.of(context);
    leaveTo(
      context,
      folder.isEmpty ? Routes.browse(vaultId) : Routes.folder(vaultId, folder),
    );
  }

  @override
  Widget build(BuildContext context) {
    ref.watch(syncListenerProvider);
    ref.listen(noteSessionProvider, (_, s) => _markSeen(s));
    final session = ref.watch(noteSessionProvider);
    final pinned = ref.watch(pinnedNotesProvider).value ?? const <String>{};
    final isPinned = pinned.contains(widget.noteId);
    // Read above the Scaffold, where the inset is still visible; see
    // keyboardIsOpen.
    final keyboard = keyboardIsOpen(context);
    final vaultId = VaultGate.of(context);
    final wide = context.isExpanded;
    final showProperties = ref.watch(propertiesOpenProvider);
    final accent = Accent.parse(
      fme.findSpan(session.buffer, kColorKey)?.displayValue,
    );
    final tint = accent.isNone ? null : accent.wash(context.tokens);
    final folder = session.meta?.folder ?? '';
    final vaultName =
        (ref.watch(vaultsProvider).value ?? const <VaultInfo>[])
            .where((v) => v.id == vaultId)
            .firstOrNull
            ?.name ??
        '';
    void toggleDrawer() =>
        ref.read(propertiesOpenProvider.notifier).update((open) => !open);

    return Scaffold(
      backgroundColor: tint,
      body: StormChrome(
        // No pill on the phone note (Q5).
        showNav: false,
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(
              child: NoteEditor(
                key: ValueKey(widget.noteId),
                onFollowLink: _followLink,
                showToolbar: keyboard,
                onActions: () => _noteActions(isPinned),
                onEscape: _leaveNote,
                leading: wide
                    ? NoteCrumb(parts: [vaultName, ...folder.split('/')])
                    : Align(
                        alignment: Alignment.centerLeft,
                        child: widget.fromSession != null
                            ? BackLink(
                                label: _sessionName(widget.fromSession!),
                                onTap: () => leaveTo(
                                  context,
                                  Routes.agentSession(widget.fromSession!),
                                ),
                              )
                            : BackLink(
                                label: folder.isEmpty
                                    ? vaultName
                                    : folder.split('/').last,
                                onTap: _leaveNote,
                              ),
                      ),
                provenance: NoteProvenance(
                  vaultId: vaultId,
                  noteId: widget.noteId,
                ),
                actions: [
                  StartSessionButton(compact: !wide),
                  if (wide)
                    DrawerToggle(open: showProperties, onTap: toggleDrawer)
                  else ...[
                    _HeaderButton(
                      icon: LucideIcons.ellipsis,
                      tooltip: 'Note actions',
                      onTap: () => _noteActions(isPinned),
                    ),
                    _HeaderButton(
                      icon: LucideIcons.sliders_horizontal,
                      tooltip: 'Properties',
                      onTap: () => PropertiesPanel.showSheet(
                        context,
                        content: session.buffer,
                        onChanged: session.editProperties,
                      ),
                    ),
                  ],
                ],
                footer: MentionsSection(
                  noteId: widget.noteId,
                  onOpen: (note) => context.push(Routes.note(vaultId, note.id)),
                ),
              ),
            ),
            if (wide && showProperties)
              PropertiesDrawer(
                content: session.buffer,
                onChanged: session.editProperties,
                onClose: () =>
                    ref.read(propertiesOpenProvider.notifier).state = false,
              ),
          ],
        ),
      ),
    );
  }

  /// Pin, attach, rename and delete.
  ///
  /// Reached by long-pressing the header rather than from a visible menu: the
  /// design's note chrome is back, path and properties, and long-press is
  /// already how this app offers a row's secondary actions.
  Future<void> _noteActions(bool isPinned) async {
    final action = await showStormSheet<VoidCallback>(
      context: context,
      title: 'Note',
      heightFactor: 0.5,
      builder: (sheetContext) => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          PopoverItem(
            label: isPinned ? 'Stop keeping offline' : 'Keep offline',
            leading: Icon(
              isPinned ? LucideIcons.pin : LucideIcons.pin_off,
              size: context.tokens.bodySize,
              color: context.tokens.text3,
            ),
            onTap: () => Navigator.pop(sheetContext, _togglePin),
          ),
          PopoverItem(
            label: 'Attach a file',
            leading: Icon(
              LucideIcons.paperclip,
              size: context.tokens.bodySize,
              color: context.tokens.text3,
            ),
            onTap: () => Navigator.pop(sheetContext, _attach),
          ),
          PopoverItem(
            label: 'Rename or move',
            leading: Icon(
              LucideIcons.pencil_line,
              size: context.tokens.bodySize,
              color: context.tokens.text3,
            ),
            onTap: () => Navigator.pop(sheetContext, _rename),
          ),
          const PopoverDivider(),
          PopoverItem(
            label: 'Delete',
            tone: PopoverTone.danger,
            leading: Icon(
              LucideIcons.trash_2,
              size: context.tokens.bodySize,
              color: context.tokens.danger,
            ),
            onTap: () => Navigator.pop(sheetContext, _delete),
          ),
        ],
      ),
    );
    action?.call();
  }
}

class _HeaderButton extends StatelessWidget {
  const _HeaderButton({
    required this.icon,
    required this.tooltip,
    required this.onTap,
  });

  final IconData icon;
  final String tooltip;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Tooltip(
      message: tooltip,
      child: InkWell(
        onTap: onTap,
        customBorder: const CircleBorder(),
        child: SizedBox(
          width: t.sp * 4,
          height: t.sp * 4,
          child: Center(
            child: Icon(icon, size: t.headingSize, color: t.text3),
          ),
        ),
      ),
    );
  }
}

/// Navigate to a note from anywhere within a vault.
void openNote(BuildContext context, String id) =>
    context.push(Routes.note(VaultGate.of(context), id));

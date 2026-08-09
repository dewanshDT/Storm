import 'dart:async';

import 'package:drift/drift.dart' show Value;
import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
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
import 'surfaces.dart';
import 'widgets.dart';
import 'mentions_section.dart';
import 'properties_panel.dart';
import 'breakpoints.dart';
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
    // Survives opening another note: the drawer is a pane at this width, not
    // a thing belonging to one note's screen.
    final showProperties = ref.watch(propertiesOpenProvider);
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
          body: StormChrome(
            showNav: !keyboard,
            // No header at desk width: the sidebar already says where the
            // note lives and offers the way back, and the design's note pane
            // starts at `v12 · Saved`.
            header: context.isExpanded
                ? null
                : _Header(
                    key: const Key('note-header'),
                    folder: session.meta?.folder ?? '',
                    onBack: () => context.canPop()
                        ? context.pop()
                        : context.go(Routes.dashboard),
                    onUp: () => context.go(
                      Routes.folder(vaultId, session.meta?.folder ?? ''),
                    ),
                    onProperties: () => PropertiesPanel.showSheet(
                      context,
                      content: session.buffer,
                      onChanged: session.editProperties,
                    ),
                    onActions: () => _noteActions(isPinned),
                  ),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Expanded(
                  child: NoteEditor(
                    onFollowLink: _followLink,
                    showToolbar: keyboard,
                    onActions: () => _noteActions(isPinned),
                    footer: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        AttachmentStrip(body: session.body),
                        MentionsSection(
                          noteId: widget.noteId,
                          initiallyExpanded: _showContext,
                          onOpen: (note) =>
                              context.push(Routes.note(vaultId, note.id)),
                        ),
                      ],
                    ),
                  ),
                ),
                // The rail carries the properties toggle at desk width, where
                // the sheet would cover a note that has room beside it.
                if (context.isExpanded)
                  _PropertiesRail(
                    open: showProperties,
                    onToggle: () => ref
                        .read(propertiesOpenProvider.notifier)
                        .update((open) => !open),
                  ),
                if (context.isExpanded && showProperties)
                  PropertiesDrawer(
                    content: session.buffer,
                    onChanged: session.editProperties,
                    onClose: () =>
                        ref.read(propertiesOpenProvider.notifier).state = false,
                  ),
              ],
            ),
          ),
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

/// Back, where the note lives, and the way into its properties.
class _Header extends StatelessWidget {
  const _Header({
    super.key,
    required this.folder,
    required this.onBack,
    required this.onUp,
    required this.onProperties,
    required this.onActions,
  });

  final String folder;
  final VoidCallback onBack;
  final VoidCallback onUp;
  final VoidCallback onProperties;
  final VoidCallback onActions;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    // Hand-rolled hit targets rather than IconButton: its 8px padding plus a
    // 48px minimum pushed the chevron well inboard of the header's own inset
    // and made the row twice the height the design draws. The tap area is
    // still 44 — it is the *box* that shrinks, not the target.
    //
    // Every button is the same 44 square with its glyph centred, and the row
    // hangs past the content inset by exactly half the difference. Aligning
    // the *boxes* instead left the first glyph twelve pixels right of the
    // prose below it, and the gaps between the three uneven, because one was
    // left-aligned in its box, one centred and one right-aligned.
    final overhang = StormChrome.buttonOverhang(context);

    return GestureDetector(
      onLongPress: onActions,
      behavior: HitTestBehavior.opaque,
      child: Padding(
        padding: EdgeInsets.symmetric(
          horizontal: StormChrome.contentInset(context) - overhang,
        ),
        child: SizedBox(
          height: t.sp * 5.5,
          child: Row(
            key: const Key('note-header-row'),
            children: [
              _HeaderButton(
                icon: LucideIcons.chevron_left,
                tooltip: 'Back',
                color: t.text2,
                onTap: onBack,
              ),
              Expanded(
                child: GestureDetector(
                  onTap: folder.isEmpty ? null : onUp,
                  child: Padding(
                    // Back onto the inset, since the button beside it is
                    // hanging off the edge.
                    padding: EdgeInsets.only(left: overhang),
                    child: Text(
                      folder.isEmpty ? 'Vault root' : folder,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontFamily: StormTokens.monoFamily,
                        fontSize: t.codeSize,
                        color: t.text3,
                      ),
                    ),
                  ),
                ),
              ),
              // Attach, pin, rename and delete were long-press only, which is
              // no way to find "keep offline". The long-press still works.
              _HeaderButton(
                icon: LucideIcons.ellipsis,
                tooltip: 'Note actions',
                color: t.text3,
                onTap: onActions,
              ),
              _HeaderButton(
                icon: LucideIcons.sliders_horizontal,
                tooltip: 'Properties',
                color: t.accent,
                onTap: onProperties,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _HeaderButton extends StatelessWidget {
  const _HeaderButton({
    required this.icon,
    required this.tooltip,
    required this.color,
    required this.onTap,
  });

  final IconData icon;
  final String tooltip;
  final Color color;
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
          width: t.sp * 5.5,
          height: t.sp * 5.5,
          // Centre rather than letting the SizedBox stretch the Icon to fill
          // it: an Icon given tight 44px constraints reports a 44px box and
          // draws the glyph in the middle of it, which makes every alignment
          // measurement — including a test's — off by the difference.
          child: Center(
            // One size for all three, so the space between glyphs is the
            // space between boxes and not a function of which icon is in them.
            child: Icon(icon, size: t.headingSize, color: color),
          ),
        ),
      ),
    );
  }
}

/// The narrow strip between the note and its properties at desk width.
class _PropertiesRail extends StatelessWidget {
  const _PropertiesRail({required this.open, required this.onToggle});

  final bool open;
  final VoidCallback onToggle;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      width: t.sp * 7,
      decoration: BoxDecoration(
        border: Border(
          left: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      child: Column(
        children: [
          // 16 from the pane top, as the prototype's rail has it.
          SizedBox(height: t.sp * 2),
          Tooltip(
            message: 'Properties',
            child: InkWell(
              onTap: onToggle,
              borderRadius: BorderRadius.circular(t.rControl * 0.8),
              child: Container(
                width: t.sp * 4,
                height: t.sp * 4,
                alignment: Alignment.center,
                decoration: BoxDecoration(
                  // Filled either way — the design draws it as a button, and
                  // an outline-less transparent square reads as an icon that
                  // happens to be there.
                  color: open ? t.accentSoft : t.surface2,
                  borderRadius: BorderRadius.circular(t.rControl * 0.8),
                ),
                child: Icon(
                  LucideIcons.sliders_horizontal,
                  size: t.bodySize,
                  color: open ? t.accent : t.text2,
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// Navigate to a note from anywhere within a vault.
void openNote(BuildContext context, String id) =>
    context.push(Routes.note(VaultGate.of(context), id));

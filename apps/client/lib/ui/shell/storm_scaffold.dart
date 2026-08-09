import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../router.dart';
import '../../state/app_state.dart';
import '../../state/vault_config.dart' show kColorKey;
import '../browse_screen.dart' show createFolder;
import '../new_note_dialog.dart';
import '../breakpoints.dart';
import '../tokens.dart';
import 'corner_bubbles.dart';
import 'vault_gate.dart';
import 'nav_bubble.dart';

/// Every screen inside the vault: content, the persistent corner bubbles, the
/// floating nav bubble, and the new-note action wired in one place.
///
/// There is no app bar. The design puts the vault and settings bubbles at the
/// top corners of *every* vault screen and gives each screen its own [header]
/// underneath them, so a title bar would be a third band of chrome nothing
/// asked for.
class StormScaffold extends ConsumerStatefulWidget {
  const StormScaffold({
    super.key,
    required this.child,
    this.header,
    this.showNav = true,
    this.showBubbles = true,
  });

  final Widget child;

  /// The screen's own chrome, laid out below the corner bubbles.
  final Widget? header;
  final bool showNav;
  final bool showBubbles;

  @override
  ConsumerState<StormScaffold> createState() => _StormScaffoldState();
}

class _StormScaffoldState extends ConsumerState<StormScaffold> {
  /// Creating a note is offered from the nav bubble on every screen, so it
  /// lives here rather than being duplicated onto each one.
  Future<void> _createNote() async {
    // The folder follows where they are rather than being typed — a note made
    // from inside `Projects/Storm` almost always belongs there.
    final here = Routes.folderOf(GoRouterState.of(context).uri);
    final wanted = await promptForNewNote(context, folder: here);
    if (wanted == null || !mounted) return;

    final vaultId = VaultGate.of(context);
    final created = await ref
        .read(syncEngineProvider)
        .create(
          path: noteFileName(wanted.name, folder: here),
          // A colour chosen up front is just the note's first property.
          content: wanted.accent.isNone
              ? ''
              : '---\n$kColorKey: ${wanted.accent.name}\n---\n\n',
        );
    if (!mounted) return;

    if (created.meta == null) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(created.error ?? 'Could not create the note')),
      );
      return;
    }
    ref.invalidate(treeProvider);
    context.push(Routes.note(vaultId, created.meta!.id));
  }

  /// Creating a folder, offered alongside it.
  Future<void> _createFolder() async {
    final here = Routes.folderOf(GoRouterState.of(context).uri);
    await createFolder(context, ref, VaultGate.of(context), here);
  }

  @override
  Widget build(BuildContext context) {
    // Providers are lazy: without this nothing subscribes to the engine's
    // change stream, so remote edits never reach the open editor.
    ref.watch(syncListenerProvider);
    final keyboard = keyboardIsOpen(context);

    return NewNoteRequest(
      onRequest: _createNote,
      child: NewFolderRequest(
        onRequest: _createFolder,
        child: Scaffold(
          body: StormChrome(
            header: widget.header,
            showNav: widget.showNav && !keyboard,
            showBubbles: widget.showBubbles,
            child: widget.child,
          ),
        ),
      ),
    );
  }
}

/// The chrome itself: corner bubbles, the screen's header, the content, and
/// the nav pill over the lot.
///
/// Separate from [StormScaffold] because the note screen tints its own
/// background and so builds its `Scaffold` itself, but must not therefore
/// grow a second, slightly different set of bubbles.
class StormChrome extends StatelessWidget {
  const StormChrome({
    super.key,
    required this.child,
    this.header,
    this.showNav = true,
    this.showBubbles = true,
  });

  final Widget child;
  final Widget? header;
  final bool showNav;
  final bool showBubbles;

  /// How far the content has to start below the top edge to clear the bubbles.
  static double contentTop(BuildContext context) {
    final t = context.tokens;
    // bubble inset + bubble side + a gap.
    return t.sp * 2.5 + t.sp * 5.5 + t.sp;
  }

  /// What every scrolling child leaves at the bottom for the nav pill.
  static double navClearance(BuildContext context) => context.tokens.sp * 13.5;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    // Wide screens carry the sidebar and its own switcher, so the corners are
    // free of both bubbles there.
    final bubbles = showBubbles && !context.isExpanded;

    return SafeArea(
      bottom: false,
      child: Stack(
        children: [
          Positioned.fill(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                if (bubbles)
                  SizedBox(height: t.sp * 5.5 + t.sp * 2.5)
                else
                  SizedBox(height: t.sp),
                if (header != null)
                  Padding(
                    padding: EdgeInsets.fromLTRB(
                      t.sp * 2.5,
                      0,
                      t.sp * 2.5,
                      t.sp * 0.75,
                    ),
                    child: header!,
                  ),
                Expanded(child: child),
              ],
            ),
          ),
          if (bubbles) ...[
            Positioned(
              top: t.sp * 2.5,
              left: t.sp * 2.5,
              child: const VaultBubble(),
            ),
            Positioned(
              top: t.sp * 2.5,
              right: t.sp * 2.5,
              child: const SettingsBubble(),
            ),
          ],
          if (showNav)
            const Positioned(left: 0, right: 0, bottom: 0, child: NavBubble()),
        ],
      ),
    );
  }
}

/// Prompts for a vault-relative path.
///
/// A StatefulWidget owns the controller, because disposing it when
/// `showDialog` returns kills it while the route is still animating out — the
/// red screen that appeared right after naming a new note.
Future<String?> promptForPath(
  BuildContext context, {
  required String title,
  required String initial,
  bool isFolder = false,
}) {
  return showDialog<String>(
    context: context,
    builder: (_) =>
        _PathDialog(title: title, initial: initial, isFolder: isFolder),
  );
}

class _PathDialog extends StatefulWidget {
  const _PathDialog({
    required this.title,
    required this.initial,
    this.isFolder = false,
  });

  final String title;
  final String initial;

  /// Folders take no `.md`, and the field says so.
  final bool isFolder;

  @override
  State<_PathDialog> createState() => _PathDialogState();
}

class _PathDialogState extends State<_PathDialog> {
  late final TextEditingController _controller = TextEditingController(
    text: widget.initial,
  );
  String? _error;

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _submit() {
    final value = _controller.text.trim();
    final problem = validateVaultPath(value);
    if (problem != null) {
      setState(() => _error = problem);
      return;
    }
    // Notes get `.md` appended; folders must not, or every new folder would
    // be called `Projects.md`.
    Navigator.pop(
      context,
      widget.isFolder || value.endsWith('.md') ? value : '$value.md',
    );
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(widget.title),
      content: TextField(
        controller: _controller,
        autofocus: true,
        decoration: InputDecoration(
          labelText: widget.isFolder ? 'Folder name' : 'Path in vault',
          hintText: widget.isFolder ? 'Projects' : 'Folder/Note.md',
          errorText: _error,
        ),
        onSubmitted: (_) => _submit(),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Cancel'),
        ),
        FilledButton(onPressed: _submit, child: const Text('OK')),
      ],
    );
  }
}

/// Mirrors the server's path rules, so the two cannot drift into disagreeing
/// about what a legal path is.
String? validateVaultPath(String path) {
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

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../router.dart';
import '../../state/app_state.dart';
import 'corner_bubbles.dart';
import 'nav_bubble.dart';

/// Every screen inside the vault: content, the floating nav bubble, and the
/// new-note action wired in one place rather than per screen.
class StormScaffold extends ConsumerStatefulWidget {
  const StormScaffold({
    super.key,
    required this.child,
    this.title,
    this.leading,
    this.actions = const [],
    this.showNav = true,
  });

  final Widget child;
  final Widget? title;
  final Widget? leading;
  final List<Widget> actions;
  final bool showNav;

  @override
  ConsumerState<StormScaffold> createState() => _StormScaffoldState();
}

class _StormScaffoldState extends ConsumerState<StormScaffold> {
  /// Creating a note is offered from the nav bubble on every screen, so it
  /// lives here rather than being duplicated onto each one.
  Future<void> _createNote() async {
    final path = await promptForPath(
      context,
      title: 'New note',
      initial: 'Untitled.md',
    );
    if (path == null || !mounted) return;

    final created = await ref.read(syncEngineProvider).create(path: path);
    if (!mounted) return;

    if (created.meta == null) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(created.error ?? 'Could not create the note')),
      );
      return;
    }
    ref.invalidate(treeProvider);
    context.go(Routes.note(created.meta!.id));
  }

  @override
  Widget build(BuildContext context) {
    // Providers are lazy: without this nothing subscribes to the engine's
    // change stream, so remote edits never reach the open editor.
    ref.watch(syncListenerProvider);
    final keyboard = keyboardIsOpen(context);

    return NewNoteRequest(
      onRequest: _createNote,
      child: Scaffold(
        appBar: AppBar(
          leading: widget.leading,
          title: widget.title,
          actions: widget.actions,
        ),
        body: Stack(
          children: [
            Positioned.fill(child: widget.child),
            if (widget.showNav && !keyboard)
              const Positioned(
                left: 0,
                right: 0,
                bottom: 0,
                child: NavBubble(),
              ),
          ],
        ),
      ),
    );
  }
}

/// The dashboard's own header, which uses the corner bubbles instead of a
/// conventional app bar.
class DashboardHeader extends StatelessWidget implements PreferredSizeWidget {
  const DashboardHeader({super.key});

  @override
  Size get preferredSize => const Size.fromHeight(64);

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      bottom: false,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 10, 16, 6),
        child: Row(
          children: [
            const VaultBubble(),
            Expanded(
              child: Center(
                child: Text(
                  'Storm',
                  style: Theme.of(context).textTheme.titleMedium?.copyWith(
                        fontWeight: FontWeight.w600,
                      ),
                ),
              ),
            ),
            const ProfileBubble(),
          ],
        ),
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
}) {
  return showDialog<String>(
    context: context,
    builder: (_) => _PathDialog(title: title, initial: initial),
  );
}

class _PathDialog extends StatefulWidget {
  const _PathDialog({required this.title, required this.initial});

  final String title;
  final String initial;

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
    Navigator.pop(context, value.endsWith('.md') ? value : '$value.md');
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(widget.title),
      content: TextField(
        controller: _controller,
        autofocus: true,
        decoration: InputDecoration(
          labelText: 'Path in vault',
          hintText: 'Folder/Note.md',
          errorText: _error,
          border: const OutlineInputBorder(),
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

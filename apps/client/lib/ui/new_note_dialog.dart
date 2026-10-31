import 'package:flutter/material.dart';

import 'accents.dart';
import 'tokens.dart';
import 'widgets.dart';

/// What a new note needs from the user: a name, and optionally a colour.
class NewNote {
  const NewNote({required this.name, required this.accent});

  final String name;
  final Accent accent;
}

/// Asks for a note's name, which may carry folders: `Ideas/Plan` makes
/// `Plan` in `Ideas` under [folder], `/Ideas/Plan` the same from the top of
/// the vault, and [noteFileName] turns either into a path. Missing folders
/// are made by the create.
Future<NewNote?> promptForNewNote(BuildContext context, {String folder = ''}) =>
    showDialog<NewNote>(
      context: context,
      builder: (_) => _NewNoteDialog(folder: folder),
    );

/// Turns a note's name into a vault-relative path.
///
/// A `/` separates folders, relative to [folder] unless the name starts with
/// one. Everything the server's path rules forbid is removed rather than
/// refused, so a name can never fail validation downstream: empty segments
/// and `.`/`..` go, as do each segment's leading dots. [newNoteNameError] is
/// what the dialog refuses first.
String noteFileName(String name, {String folder = ''}) {
  final fromRoot = name.trimLeft().startsWith('/');
  final parts = [
    for (final s in name.split('/'))
      if (_segment(s) case final c when c.isNotEmpty) c,
  ];
  var leaf = parts.isEmpty ? 'Untitled' : parts.removeLast();
  if (!leaf.toLowerCase().endsWith('.md')) leaf = '$leaf.md';
  return [if (!fromRoot && folder.isNotEmpty) folder, ...parts, leaf].join('/');
}

String _segment(String s) => s
    .replaceAll(r'\', ' ')
    .replaceAll(RegExp(r'^[\s.]+'), '')
    .replaceAll(RegExp(r'\s+'), ' ')
    .trim();

/// Why the dialog won't create [name]: a `..`, which would otherwise be
/// dropped and land the note somewhere unasked, or nothing after the last
/// `/`.
String? newNoteNameError(String name) {
  final segments = name.trim().split('/');
  if (segments.any((s) => s.trim() == '..')) {
    return "A name can't use “..”: start it with / to begin at the top "
        'of the vault.';
  }
  if (segments.length > 1 && segments.last.trim().isEmpty) {
    return 'Name the note after the last /.';
  }
  return null;
}

/// A `StatefulWidget` so the controller outlives the closing animation —
/// disposing it as `showDialog` returns kills it mid-animation, which is the
/// red screen that once appeared right after naming a note.
class _NewNoteDialog extends StatefulWidget {
  const _NewNoteDialog({required this.folder});

  final String folder;

  @override
  State<_NewNoteDialog> createState() => _NewNoteDialogState();
}

class _NewNoteDialogState extends State<_NewNoteDialog> {
  final _controller = TextEditingController();
  Accent _accent = Accent.none;

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  String? _error;

  void _submit() {
    final error = newNoteNameError(_controller.text);
    if (error != null) {
      setState(() => _error = error);
      return;
    }
    Navigator.pop(context, NewNote(name: _controller.text, accent: _accent));
  }

  /// Said, not asked: where the note will land as the name stands.
  String _where() {
    final path = noteFileName(_controller.text, folder: widget.folder);
    final cut = path.lastIndexOf('/');
    return cut < 0 ? 'at the top of the vault' : 'in ${path.substring(0, cut)}';
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;

    return AlertDialog(
      title: const Text('New note'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          StormInput(
            controller: _controller,
            autofocus: true,
            textCapitalization: TextCapitalization.sentences,
            labelText: 'Name',
            hintText: 'Weekly review',
            onSubmitted: (_) => _submit(),
            onChanged: (_) => setState(() => _error = null),
          ),
          SizedBox(height: t.sp),
          Text(
            _error ?? _where(),
            key: const Key('new-note-where'),
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.labelSize,
              color: _error == null ? t.text3 : t.danger,
            ),
          ),
          SizedBox(height: t.sp * 0.5),
          Text(
            'A / makes folders, from here; start with / to begin at the top '
            'of the vault.',
            key: const Key('new-note-hint'),
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.labelSize,
              color: t.text3,
            ),
          ),
          SizedBox(height: t.sp * 2.25),
          const SectionLabel('Colour'),
          SizedBox(height: t.sp),
          AccentPicker(
            size: t.sp * 3.25,
            selected: _accent,
            onSelected: (accent) => setState(() => _accent = accent),
          ),
        ],
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Cancel'),
        ),
        FilledButton(onPressed: _submit, child: const Text('Create')),
      ],
    );
  }
}

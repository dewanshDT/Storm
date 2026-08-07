import 'package:flutter/material.dart';

import 'accents.dart';

/// What a new note needs from the user: a name, and optionally a colour.
class NewNote {
  const NewNote({required this.name, required this.accent});

  final String name;
  final Accent accent;
}

/// Asks for a note's name — not its path.
///
/// The old dialog asked for `Folder/Note.md` and made the user supply both the
/// folder and the extension. Neither is a decision worth asking about: the
/// folder is wherever they already are, and every note is markdown. So this
/// takes a name and [noteFileName] turns it into a path.
Future<NewNote?> promptForNewNote(BuildContext context, {String folder = ''}) =>
    showDialog<NewNote>(
      context: context,
      builder: (_) => _NewNoteDialog(folder: folder),
    );

/// Turns a note's name into a vault-relative path.
///
/// Separators are stripped rather than rejected: typing `Q1/Plan` into a name
/// field means the words, not a folder — and letting it through would create
/// a directory the user did not ask for. Everything the server's path rules
/// forbid is removed here, so a name can never fail validation downstream.
String noteFileName(String name, {String folder = ''}) {
  // Separators first: `Q1/Plan` is two words, not a folder.
  var clean = name.replaceAll(RegExp(r'[/\\]'), ' ');
  // Then every leading dot and space together, not just the first run — a
  // `../../` becomes `.. .. ` above, and stopping after one run would leave
  // a stray `..` in the filename.
  clean = clean.replaceAll(RegExp(r'^[\s.]+'), '');
  // Collapse what the substitutions left behind.
  clean = clean.replaceAll(RegExp(r'\s+'), ' ').trim();

  if (clean.isEmpty) clean = 'Untitled';
  if (!clean.toLowerCase().endsWith('.md')) clean = '$clean.md';
  return folder.isEmpty ? clean : '$folder/$clean';
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

  void _submit() =>
      Navigator.pop(context, NewNote(name: _controller.text, accent: _accent));

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;

    return AlertDialog(
      title: const Text('New note'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          TextField(
            controller: _controller,
            autofocus: true,
            textCapitalization: TextCapitalization.sentences,
            decoration: const InputDecoration(
              labelText: 'Name',
              hintText: 'Weekly review',
            ),
            onSubmitted: (_) => _submit(),
          ),
          if (widget.folder.isNotEmpty) ...[
            const SizedBox(height: 8),
            // Said, not asked: the folder follows where they are.
            Text(
              'in ${widget.folder}',
              style: TextStyle(fontSize: 12, color: scheme.onSurfaceVariant),
            ),
          ],
          const SizedBox(height: 18),
          Text(
            'Colour',
            style: TextStyle(fontSize: 12, color: scheme.onSurfaceVariant),
          ),
          const SizedBox(height: 8),
          AccentPicker(
            size: 26,
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

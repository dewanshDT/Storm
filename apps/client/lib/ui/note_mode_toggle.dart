import 'package:flutter/material.dart';

import 'tokens.dart';

/// Whether the open note is being read or edited.
///
/// Read Mode renders Markdown with [StormMarkdownView]; Edit Mode keeps the
/// existing source editor. Markdown is the single source of truth either way.
enum NoteViewMode { read, edit }

/// Compact Read / Edit control in Storm chrome language.
///
/// Not Material's [SegmentedButton] — that brings its own density and colours.
/// Two small labels sharing a surface, with the active side on [accentSoft].
class NoteModeToggle extends StatelessWidget {
  const NoteModeToggle({
    super.key,
    required this.mode,
    required this.onChanged,
  });

  final NoteViewMode mode;
  final ValueChanged<NoteViewMode> onChanged;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Semantics(
      label: 'Note view mode',
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: t.surface2,
          borderRadius: BorderRadius.circular(t.rControl * 0.8),
          border: Border.all(color: t.border, width: t.bw),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            _Segment(
              key: const Key('mode-read'),
              label: 'Read',
              selected: mode == NoteViewMode.read,
              onTap: () => onChanged(NoteViewMode.read),
            ),
            _Segment(
              key: const Key('mode-edit'),
              label: 'Edit',
              selected: mode == NoteViewMode.edit,
              onTap: () => onChanged(NoteViewMode.edit),
            ),
          ],
        ),
      ),
    );
  }
}

class _Segment extends StatelessWidget {
  const _Segment({
    super.key,
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Material(
      color: selected ? t.accentSoft : Colors.transparent,
      borderRadius: BorderRadius.circular(t.rControl * 0.7),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(t.rControl * 0.7),
        child: Padding(
          padding: EdgeInsets.symmetric(
            horizontal: t.sp * 1.25,
            vertical: t.sp * 0.5,
          ),
          child: Text(
            label,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.labelSize,
              fontWeight: selected ? FontWeight.w600 : FontWeight.w500,
              color: selected ? t.accent : t.text3,
            ),
          ),
        ),
      ),
    );
  }
}

import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';

import 'tokens.dart';
import 'widgets.dart';

/// Minimal in-note find chrome — query, match count, next/prev, close.
///
/// Searches the note body buffer; the editor selects the current match. Not
/// vault FTS (that is ⌘K → search route).
class NoteFindBar extends StatelessWidget {
  const NoteFindBar({
    super.key,
    required this.controller,
    required this.focusNode,
    required this.matchLabel,
    required this.onChanged,
    required this.onNext,
    required this.onPrevious,
    required this.onClose,
  });

  final TextEditingController controller;
  final FocusNode focusNode;
  final String matchLabel;
  final ValueChanged<String> onChanged;
  final VoidCallback onNext;
  final VoidCallback onPrevious;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Material(
      color: t.surface2,
      child: Padding(
        padding: EdgeInsets.symmetric(horizontal: t.sp, vertical: t.sp * 0.5),
        child: Row(
          children: [
            Icon(LucideIcons.search, size: t.codeSize, color: t.text3),
            SizedBox(width: t.sp * 0.75),
            Expanded(
              child: StormInput(
                key: const Key('note-find-field'),
                controller: controller,
                focusNode: focusNode,
                hintText: 'Find in note',
                autofocus: true,
                onChanged: onChanged,
                onSubmitted: (_) => onNext(),
              ),
            ),
            SizedBox(width: t.sp * 0.5),
            Text(
              matchLabel,
              key: const Key('note-find-count'),
              style: TextStyle(
                fontFamily: StormTokens.monoFamily,
                fontSize: t.labelSize,
                color: t.text3,
              ),
            ),
            IconButton(
              key: const Key('note-find-prev'),
              tooltip: 'Previous',
              icon: Icon(LucideIcons.chevron_up, size: t.codeSize),
              onPressed: onPrevious,
            ),
            IconButton(
              key: const Key('note-find-next'),
              tooltip: 'Next',
              icon: Icon(LucideIcons.chevron_down, size: t.codeSize),
              onPressed: onNext,
            ),
            IconButton(
              key: const Key('note-find-close'),
              tooltip: 'Close',
              icon: Icon(LucideIcons.x, size: t.codeSize),
              onPressed: onClose,
            ),
          ],
        ),
      ),
    );
  }
}

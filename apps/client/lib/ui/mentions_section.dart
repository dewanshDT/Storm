import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../state/app_state.dart';
import 'states.dart';
import 'tokens.dart';
import 'widgets.dart';

/// "Linked mentions" — the notes that link *to* the one being edited.
///
/// Collapsed by default and the last thing in the note's own scroll rather
/// than a third column: it is a reference, not something to keep in view while
/// writing, and a third column doesn't fit on a phone.
///
/// The server resolves these by wikilink target title, so a note only appears
/// once its `[[Link]]` has been indexed — which happens on write.
class MentionsSection extends ConsumerStatefulWidget {
  const MentionsSection({
    super.key,
    required this.noteId,
    required this.onOpen,
    this.initiallyExpanded = false,
  });

  final String noteId;
  final void Function(NoteMeta) onOpen;
  final bool initiallyExpanded;

  @override
  ConsumerState<MentionsSection> createState() => _MentionsSectionState();
}

class _MentionsSectionState extends ConsumerState<MentionsSection> {
  late bool _expanded = widget.initiallyExpanded;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final online = ref.watch(syncEngineProvider).isOnline;
    final backlinks = online
        ? ref.watch(backlinksProvider(widget.noteId))
        : const AsyncValue<List<NoteMeta>>.data([]);
    final count = backlinks.value?.length ?? 0;

    return Container(
      margin: EdgeInsets.only(top: t.sp * 3),
      padding: EdgeInsets.only(top: t.sp * 2),
      decoration: BoxDecoration(
        border: Border(
          top: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          InkWell(
            onTap: () => setState(() => _expanded = !_expanded),
            borderRadius: BorderRadius.circular(t.rControl),
            child: Padding(
              padding: EdgeInsets.symmetric(vertical: t.sp * 0.5),
              child: Row(
                children: [
                  Icon(
                    _expanded ? Icons.expand_more : Icons.chevron_right,
                    size: t.bodySize,
                    color: t.text3,
                  ),
                  SizedBox(width: t.sp * 0.5),
                  SectionLabel('Mentions ($count)'),
                ],
              ),
            ),
          ),
          if (_expanded)
            Padding(
              padding: EdgeInsets.only(top: t.sp),
              child: !online
                  // Offline the link index is unavailable, and an empty list
                  // would read as "nothing links here" — a different and wrong
                  // claim.
                  ? _Note(text: 'Mentions need the server.')
                  : backlinks.when(
                      loading: () => const SkeletonRows(rows: 2),
                      error: (e, _) => _Note(text: describeFailure(e)),
                      data: (notes) => notes.isEmpty
                          ? _Note(text: 'Nothing links here yet.')
                          : Column(
                              mainAxisSize: MainAxisSize.min,
                              children: [
                                for (final note in notes)
                                  _MentionCard(
                                    note: note,
                                    onTap: () => widget.onOpen(note),
                                  ),
                              ],
                            ),
                    ),
            ),
        ],
      ),
    );
  }
}

class _MentionCard extends StatelessWidget {
  const _MentionCard({required this.note, required this.onTap});

  final NoteMeta note;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.only(bottom: t.sp),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(t.rControl),
        child: Container(
          width: double.infinity,
          padding: EdgeInsets.symmetric(
            horizontal: t.sp * 1.5,
            vertical: t.sp * 1.25,
          ),
          decoration: BoxDecoration(
            color: t.surface,
            borderRadius: BorderRadius.circular(t.rControl),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                note.title.isEmpty ? note.path : note.title,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  fontWeight: FontWeight.w500,
                  color: t.text2,
                ),
              ),
              if (note.folder.isNotEmpty) ...[
                SizedBox(height: t.sp * 0.25),
                Text(
                  note.folder,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.labelSize,
                    color: t.text3,
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

class _Note extends StatelessWidget {
  const _Note({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.symmetric(vertical: t.sp),
      child: Text(
        text,
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontSize: t.labelSize,
          color: t.text3,
        ),
      ),
    );
  }
}

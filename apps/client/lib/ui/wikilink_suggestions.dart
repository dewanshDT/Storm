import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../editor/storm_markdown_controller.dart';
import '../state/app_state.dart';
import '../state/wikilinks.dart';

/// Note suggestions for a `[[` being typed.
///
/// Sits directly above the formatting toolbar rather than floating at the
/// caret. Anchoring to the caret means measuring a glyph position inside a
/// scrolling multi-line field on every keystroke, and on a phone the result
/// would spend most of its life underneath the keyboard anyway.
///
/// Whether it shows at all is derived from the buffer and the caret — see
/// [activeWikilinkQuery] — so it cannot get out of step with the text.
class WikilinkSuggestions extends ConsumerWidget {
  const WikilinkSuggestions({super.key, required this.controller});

  final StormMarkdownController controller;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Rebuilds on every text *and* caret change: moving the caret out of a
    // link has to dismiss this, and that is not a text edit.
    return ValueListenableBuilder<TextEditingValue>(
      valueListenable: controller,
      builder: (context, value, _) {
        final query = activeWikilinkQuery(
          value.text,
          value.selection.baseOffset,
        );
        if (query == null || !value.selection.isCollapsed) {
          return const SizedBox.shrink();
        }

        final notes = ref.watch(treeProvider).value ?? const [];
        final matches = suggestWikilinks(notes, query.query);
        if (matches.isEmpty) return const SizedBox.shrink();

        final scheme = Theme.of(context).colorScheme;
        return Container(
          height: 42,
          decoration: BoxDecoration(
            color: scheme.surfaceContainer,
            border: Border(top: BorderSide(color: scheme.outlineVariant)),
          ),
          child: ListView.builder(
            scrollDirection: Axis.horizontal,
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
            itemCount: matches.length,
            itemBuilder: (context, i) {
              final note = matches[i];
              final label = wikilinkDisplayName(note);
              return Padding(
                padding: const EdgeInsets.symmetric(horizontal: 3),
                child: ActionChip(
                  label: Text(label, overflow: TextOverflow.ellipsis),
                  visualDensity: VisualDensity.compact,
                  onPressed: () => controller.completeWikilink(
                    wikilinkTargetFor(notes, note),
                  ),
                ),
              );
            },
          ),
        );
      },
    );
  }
}

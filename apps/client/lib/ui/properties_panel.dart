import 'package:flutter/material.dart';

import 'atoms.dart';
import 'breakpoints.dart';
import 'note_properties.dart';
import 'tokens.dart';

/// A note's frontmatter, given a surface of its own.
///
/// It used to sit inline above the prose, which cost a phone's first screen
/// permanently: a note with a dozen keys pushed the writing off the bottom
/// before a word of it was visible. The design gives properties their own
/// place — a sheet you pull up on a phone, a drawer beside the note on a wide
/// screen — with a header and a way out.
///
/// The panel is presentation only. Every write still goes through
/// [NoteProperties], which splices individual frontmatter lines; there is no
/// second path to the YAML and no raw mode.
class PropertiesPanel extends StatelessWidget {
  const PropertiesPanel({
    super.key,
    required this.content,
    required this.onChanged,
    this.onClose,
  });

  final String content;
  final ValueChanged<String> onChanged;
  final VoidCallback? onClose;

  /// How wide the drawer is beside a note.
  static const drawerWidth = 320.0;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        Padding(
          padding: EdgeInsets.fromLTRB(
            t.cardPad,
            t.sp * 2,
            t.sp * 1.5,
            t.sp * 1.5,
          ),
          child: Row(
            children: [
              const Expanded(child: SectionLabel('Properties')),
              if (onClose != null)
                IconButton(
                  icon: const Icon(Icons.close, size: 18),
                  color: t.text3,
                  visualDensity: VisualDensity.compact,
                  tooltip: 'Close',
                  onPressed: onClose,
                ),
            ],
          ),
        ),
        Flexible(
          child: SingleChildScrollView(
            padding: EdgeInsets.fromLTRB(t.cardPad, 0, t.cardPad, t.cardPad),
            child: NoteProperties(content: content, onChanged: onChanged),
          ),
        ),
      ],
    );
  }

  /// Opens the panel the way this width wants it opened.
  ///
  /// On a wide screen the drawer is already beside the note, so there is
  /// nothing to open — the caller toggles it instead. This is only the phone
  /// path.
  static Future<void> showSheet(
    BuildContext context, {
    required String content,
    required ValueChanged<String> onChanged,
  }) {
    return showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      showDragHandle: true,
      builder: (sheetContext) => SafeArea(
        child: ConstrainedBox(
          // Enough for a dozen properties, never the whole screen: the note
          // stays visible behind it, which is what makes it a sheet rather
          // than another page.
          constraints: BoxConstraints(
            maxHeight: MediaQuery.sizeOf(context).height * 0.7,
          ),
          child: PropertiesPanel(
            content: content,
            onChanged: onChanged,
            onClose: () => Navigator.of(sheetContext).pop(),
          ),
        ),
      ),
    );
  }
}

/// The drawer form, for a wide screen.
class PropertiesDrawer extends StatelessWidget {
  const PropertiesDrawer({
    super.key,
    required this.content,
    required this.onChanged,
    required this.onClose,
  });

  final String content;
  final ValueChanged<String> onChanged;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    if (!context.isExpanded) return const SizedBox.shrink();

    return Row(
      children: [
        Container(width: t.bw, color: t.border),
        SizedBox(
          width: PropertiesPanel.drawerWidth,
          child: Material(
            color: t.bg,
            child: PropertiesPanel(
              content: content,
              onChanged: onChanged,
              onClose: onClose,
            ),
          ),
        ),
      ],
    );
  }
}

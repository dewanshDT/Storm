import 'package:flutter/material.dart';

import '../editor/storm_markdown_controller.dart';

/// The formatting bar that takes the nav bubble's place while the keyboard is
/// up.
///
/// It calls the controller's editing methods and nothing else. It never
/// touches `controller.text` or `controller.selection` directly: those are two
/// assignments where the buffer and the caret disagree in between, and the
/// controller's methods exist precisely so there is one place that gets that
/// right.
class EditorToolbar extends StatelessWidget {
  const EditorToolbar({super.key, required this.controller});

  final StormMarkdownController controller;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;

    // The toolbar counts as *inside* the text field for tap purposes.
    // `TextField`'s default `onTapOutside` unfocuses on desktop and web, and an
    // unfocused field closes the keyboard — which hides this toolbar mid-tap
    // and leaves the caret behind. Sharing EditableText's tap-region group is
    // the sanctioned way to say "this is part of the editor".
    return TapRegion(
      groupId: EditableText,
      child: Material(
        color: scheme.surfaceContainerHigh,
        elevation: 8,
        child: SafeArea(
          top: false,
          child: SizedBox(
            height: 48,
            // Scrolls rather than wraps: an overflowing bar drops buttons
            // silently, which is how the attach action once appeared to vanish.
            child: ListView(
              scrollDirection: Axis.horizontal,
              padding: const EdgeInsets.symmetric(horizontal: 6),
              children: [
                _Button(
                  icon: Icons.title,
                  tooltip: 'Heading',
                  // A picker, not a blind cycle: cycling means tapping four
                  // times to find out what you got.
                  onTap: () => _pickHeading(context),
                ),
                const _Divider(),
                _Button(
                  icon: Icons.format_bold,
                  tooltip: 'Bold',
                  onTap: () => controller.toggleInline('**'),
                ),
                _Button(
                  icon: Icons.format_italic,
                  tooltip: 'Italic',
                  onTap: () => controller.toggleInline('*'),
                ),
                _Button(
                  icon: Icons.code,
                  tooltip: 'Code',
                  onTap: () => controller.toggleInline('`'),
                ),
                _Button(
                  icon: Icons.format_strikethrough,
                  tooltip: 'Strikethrough',
                  onTap: () => controller.toggleInline('~~'),
                ),
                _Button(
                  icon: Icons.border_color_outlined,
                  tooltip: 'Highlight',
                  onTap: () => controller.toggleInline('=='),
                ),
                const _Divider(),
                _Button(
                  icon: Icons.format_list_bulleted,
                  tooltip: 'Bullet list',
                  onTap: () => controller.setBlockPrefix('- '),
                ),
                _Button(
                  icon: Icons.format_list_numbered,
                  tooltip: 'Numbered list',
                  onTap: () => controller.setBlockPrefix('1. '),
                ),
                _Button(
                  icon: Icons.check_box_outlined,
                  tooltip: 'Task',
                  onTap: () => controller.setBlockPrefix('- [ ] '),
                ),
                _Button(
                  icon: Icons.format_quote,
                  tooltip: 'Quote',
                  onTap: () => controller.setBlockPrefix('> '),
                ),
                const _Divider(),
                _Button(
                  icon: Icons.link,
                  tooltip: 'Link to a note',
                  onTap: controller.insertWikilink,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Future<void> _pickHeading(BuildContext context) async {
    final box = context.findRenderObject() as RenderBox?;
    final origin = box == null
        ? Offset.zero
        : box.localToGlobal(Offset.zero, ancestor: null);

    // "Paragraph" means null to setBlockPrefix, but a dismissed menu also
    // returns null — so it travels as a sentinel and is translated below.
    // Without this, tapping outside the menu would strip your heading.
    const paragraph = '';

    final choice = await showMenu<String>(
      context: context,
      position: RelativeRect.fromLTRB(
        origin.dx + 8,
        origin.dy - 200,
        origin.dx + 8,
        origin.dy,
      ),
      items: const [
        PopupMenuItem(value: '# ', child: Text('Heading 1')),
        PopupMenuItem(value: '## ', child: Text('Heading 2')),
        PopupMenuItem(value: '### ', child: Text('Heading 3')),
        PopupMenuItem(value: paragraph, child: Text('Paragraph')),
      ],
    );
    if (choice == null) return;

    // Deliberately *not* guarded on `context.mounted`. Opening this menu closes
    // the keyboard, which makes `keyboardIsOpen` false, which unmounts this
    // toolbar — so by the time a choice comes back, our own context is always
    // gone. Gating on it meant the heading buttons never did anything at all,
    // while every other button worked, because the rest apply synchronously.
    // The controller belongs to the editor and outlives us, so it is safe to
    // call; nothing below touches `context`.
    controller.setBlockPrefix(
      choice == paragraph ? null : choice,
      toggle: false,
    );
  }
}

class _Button extends StatelessWidget {
  const _Button({
    required this.icon,
    required this.tooltip,
    required this.onTap,
  });

  final IconData icon;
  final String tooltip;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: tooltip,
      child: InkWell(
        // The field keeps focus, so the keyboard does not close and reopen
        // between two formatting taps.
        onTap: onTap,
        borderRadius: BorderRadius.circular(8),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 11),
          child: Icon(
            icon,
            size: 21,
            color: Theme.of(context).colorScheme.onSurfaceVariant,
          ),
        ),
      ),
    );
  }
}

class _Divider extends StatelessWidget {
  const _Divider();

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Container(
        width: 1,
        height: 20,
        margin: const EdgeInsets.symmetric(horizontal: 5),
        color: Theme.of(context).dividerColor,
      ),
    );
  }
}

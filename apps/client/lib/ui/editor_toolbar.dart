import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';

import '../editor/storm_markdown_controller.dart';
import 'tokens.dart';

/// The formatting bar that takes the nav bubble's place while the keyboard is
/// up.
///
/// It calls the controller's editing methods and nothing else. It never
/// touches `controller.text` or `controller.selection` directly: those are two
/// assignments where the buffer and the caret disagree in between, and the
/// controller's methods exist precisely so there is one place that gets that
/// right.
class EditorToolbar extends StatelessWidget {
  const EditorToolbar({super.key, required this.controller, this.onDone});

  final StormMarkdownController controller;

  /// Puts the keyboard away. Without it the only way out of editing is the
  /// system gesture, which on Android is also the way out of the note.
  final VoidCallback? onDone;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;

    // The toolbar counts as *inside* the text field for tap purposes.
    // `TextField`'s default `onTapOutside` unfocuses on desktop and web, and an
    // unfocused field closes the keyboard — which hides this toolbar mid-tap
    // and leaves the caret behind. Sharing EditableText's tap-region group is
    // the sanctioned way to say "this is part of the editor".
    return TapRegion(
      groupId: EditableText,
      child: Material(
        color: t.surface,
        child: DecoratedBox(
          decoration: BoxDecoration(
            border: Border(
              top: BorderSide(color: t.border, width: t.bw),
            ),
          ),
          child: SafeArea(
            top: false,
            child: SizedBox(
              height: t.sp * 6.5,
              // Rebuilt on every selection change, which is what makes the
              // active state honest: bold lights up when the caret is inside
              // `**…**`, not when it was last tapped.
              child: ValueListenableBuilder<TextEditingValue>(
                valueListenable: controller,
                builder: (context, _, _) {
                  final block = controller.blockPrefixHere();
                  return Row(
                    children: [
                      Expanded(
                        // Scrolls rather than wraps: an overflowing bar drops
                        // buttons silently, which is how the attach action
                        // once appeared to vanish.
                        child: ListView(
                          scrollDirection: Axis.horizontal,
                          padding: EdgeInsets.symmetric(
                            horizontal: t.sp * 0.75,
                          ),
                          children: [
                            _Button(
                              icon: LucideIcons.heading,
                              tooltip: 'Heading',
                              active: block.startsWith('#'),
                              // A picker, not a blind cycle: cycling means
                              // tapping four times to find out what you got.
                              onTap: () => _pickHeading(context),
                            ),
                            _Button(
                              icon: LucideIcons.bold,
                              tooltip: 'Bold',
                              active: controller.inlineActive('**'),
                              onTap: () => controller.toggleInline('**'),
                            ),
                            _Button(
                              icon: LucideIcons.italic,
                              tooltip: 'Italic',
                              active: controller.inlineActive('*'),
                              onTap: () => controller.toggleInline('*'),
                            ),
                            _Button(
                              icon: LucideIcons.code,
                              tooltip: 'Code',
                              active: controller.inlineActive('`'),
                              onTap: () => controller.toggleInline('`'),
                            ),
                            _Button(
                              icon: LucideIcons.strikethrough,
                              tooltip: 'Strikethrough',
                              active: controller.inlineActive('~~'),
                              onTap: () => controller.toggleInline('~~'),
                            ),
                            _Button(
                              icon: LucideIcons.highlighter,
                              tooltip: 'Highlight',
                              active: controller.inlineActive('=='),
                              onTap: () => controller.toggleInline('=='),
                            ),
                            _Button(
                              icon: LucideIcons.list,
                              tooltip: 'Bullet list',
                              active: block == '- ',
                              onTap: () => controller.setBlockPrefix('- '),
                            ),
                            _Button(
                              icon: LucideIcons.list_ordered,
                              tooltip: 'Numbered list',
                              active: orderedMarker.hasMatch(block),
                              onTap: () => controller.setBlockPrefix('1. '),
                            ),
                            _Button(
                              icon: LucideIcons.square_check,
                              tooltip: 'Task',
                              active: block.startsWith('- ['),
                              onTap: () => controller.setBlockPrefix('- [ ] '),
                            ),
                            _Button(
                              icon: LucideIcons.text_quote,
                              tooltip: 'Quote',
                              active: block.startsWith('>'),
                              onTap: () => controller.setBlockPrefix('> '),
                            ),
                            _Button(
                              icon: LucideIcons.link,
                              tooltip: 'Link to a note',
                              onTap: controller.insertWikilink,
                            ),
                          ],
                        ),
                      ),
                      if (onDone != null)
                        TextButton(
                          onPressed: onDone,
                          child: Text(
                            'Done',
                            style: TextStyle(
                              fontFamily: StormTokens.sansFamily,
                              fontSize: t.codeSize,
                              fontWeight: FontWeight.w600,
                              color: t.accent,
                            ),
                          ),
                        ),
                    ],
                  );
                },
              ),
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
    this.active = false,
  });

  final IconData icon;
  final String tooltip;
  final VoidCallback onTap;
  final bool active;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Tooltip(
      message: tooltip,
      child: InkWell(
        // The field keeps focus, so the keyboard does not close and reopen
        // between two formatting taps.
        onTap: onTap,
        borderRadius: BorderRadius.circular(t.rControl * 0.8),
        child: Container(
          margin: EdgeInsets.symmetric(vertical: t.sp * 0.75),
          padding: EdgeInsets.symmetric(horizontal: t.sp * 1.25),
          alignment: Alignment.center,
          decoration: BoxDecoration(
            color: active ? t.accentSoft : null,
            borderRadius: BorderRadius.circular(t.rControl * 0.8),
          ),
          child: Icon(
            icon,
            size: t.headingSize,
            color: active ? t.accent : t.text2,
          ),
        ),
      ),
    );
  }
}

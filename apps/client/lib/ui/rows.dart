/// List rows for notes and the reading measure.
library;

import 'package:flutter/material.dart';

import 'tokens.dart';

/// A note in a list: title and one metadata line.
class NoteRow extends StatelessWidget {
  const NoteRow({
    super.key,
    required this.title,
    this.meta,
    this.leading,
    this.trailing,
    this.onTap,
    this.onLongPress,
    this.divider = false,
    this.selected = false,
    this.padding,
  });

  final String title;
  final String? meta;
  final Widget? leading;
  final Widget? trailing;
  final VoidCallback? onTap;
  final VoidCallback? onLongPress;
  final bool divider;
  final bool selected;
  final EdgeInsetsGeometry? padding;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return InkWell(
      onTap: onTap,
      onLongPress: onLongPress,
      borderRadius: BorderRadius.zero,
      child: Container(
        padding:
            padding ??
            EdgeInsets.symmetric(horizontal: t.sp * 1.75, vertical: t.sp * 2),
        decoration: BoxDecoration(
          color: selected ? t.accentSoft : null,
          border: divider
              ? Border(
                  bottom: BorderSide(color: t.border, width: t.bw),
                )
              : null,
        ),
        child: Row(
          children: [
            if (leading != null) ...[leading!, SizedBox(width: t.sp * 1.5)],
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.bodySize * 0.95,
                      fontWeight: FontWeight.w600,
                      color: t.text,
                      height: 1.3,
                    ),
                  ),
                  if (meta != null) ...[
                    SizedBox(height: t.sp * 0.375),
                    Text(
                      meta!,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.codeSize,
                        color: t.text3,
                        height: 1.2,
                      ),
                    ),
                  ],
                ],
              ),
            ),
            if (trailing != null) ...[SizedBox(width: t.sp), trailing!],
          ],
        ),
      ),
    );
  }
}

/// A paragraph's measure on a page that fills the pane: about 72 characters
/// of UI type. Only the text is held to it; the page's cards, tables and
/// grids take the width.
class ReadingMeasure extends StatelessWidget {
  const ReadingMeasure({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) => Align(
    alignment: AlignmentDirectional.centerStart,
    child: ConstrainedBox(
      constraints: BoxConstraints(maxWidth: context.tokens.sp * 72),
      child: child,
    ),
  );
}

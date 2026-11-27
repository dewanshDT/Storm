/// List rows for notes and folders, breadcrumbs and the reading measure.
library;

import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';

import 'icons.dart';
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

/// A folder in a list: glyph, name, count, chevron.
class FolderRow extends StatelessWidget {
  const FolderRow({
    super.key,
    required this.name,
    this.count,
    this.onTap,
    this.onLongPress,
    this.divider = false,
    this.leading,
    this.padding,
  });

  final String name;
  final int? count;
  final VoidCallback? onTap;
  final VoidCallback? onLongPress;
  final bool divider;

  /// Replaces the folder glyph — the sidebar tree passes its twisty.
  final Widget? leading;
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
        decoration: divider
            ? BoxDecoration(
                border: Border(
                  bottom: BorderSide(color: t.border, width: t.bw),
                ),
              )
            : null,
        child: Row(
          children: [
            // Grey, not accent: the folder glyph is furniture, and accent is
            // reserved for what is interactive.
            // The same solid folder the nav pill carries. Lucide's outline
            // folder is a different weight from the glyph two inches below it
            // on the same screen.
            leading ??
                StormIcon(StormGlyph.folder, size: t.bodySize, color: t.text2),
            SizedBox(width: t.sp * 1.5),
            Expanded(
              child: Text(
                name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.bodySize * 0.95,
                  fontWeight: FontWeight.w500,
                  color: t.text,
                ),
              ),
            ),
            if (count != null)
              Text(
                '$count',
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  color: t.text3,
                ),
              ),
            SizedBox(width: t.sp * 0.75),
            Icon(LucideIcons.chevron_right, size: t.bodySize, color: t.text2),
          ],
        ),
      ),
    );
  }
}

/// One step in a [Breadcrumb].
@immutable
class Crumb {
  const Crumb(this.label, {this.onTap});

  final String label;
  final VoidCallback? onTap;
}

/// `Vaults › Work › Projects` — mono, accent where it navigates.
class Breadcrumb extends StatelessWidget {
  const Breadcrumb({super.key, required this.crumbs});

  final List<Crumb> crumbs;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    // Code-sized, not label-sized: the breadcrumb is read, not glanced at,
    // and at the label step it disappeared under the bubbles above it.
    final mono = TextStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: t.bodySize * 0.95,
      color: t.text2,
      height: 1.2,
    );

    final children = <Widget>[];
    for (var i = 0; i < crumbs.length; i++) {
      if (i > 0) {
        children.add(
          Padding(
            padding: EdgeInsets.symmetric(horizontal: t.sp * 0.5),
            child: Text('›', style: mono),
          ),
        );
      }
      final crumb = crumbs[i];
      final last = i == crumbs.length - 1;
      final label = Text(
        crumb.label,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: mono.copyWith(
          color: last ? t.text : (crumb.onTap != null ? t.accent : t.text3),
        ),
      );
      children.add(
        crumb.onTap == null
            ? label
            : GestureDetector(onTap: crumb.onTap, child: label),
      );
    }

    // Left-aligned, not reversed. `reverse: true` parks a short trail against
    // the right edge, where on a phone it sits underneath the settings bubble.
    return SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      child: Row(mainAxisSize: MainAxisSize.min, children: children),
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

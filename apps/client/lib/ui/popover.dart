/// The popover menu item and its tones.
library;

import 'package:flutter/material.dart';

import 'tokens.dart';

/// One line in a popover or sheet, at token padding rather than `ListTile`'s
/// 56px Material rhythm.
class PopoverItem extends StatelessWidget {
  const PopoverItem({
    super.key,
    required this.label,
    this.leading,
    this.trailing,
    this.subtitle,
    this.onTap,
    this.tone = PopoverTone.normal,
    this.selected = false,
    this.tooltip,
  });

  final String label;

  /// The whole label on hover, for a label its menu elides.
  final String? tooltip;
  final Widget? leading;
  final Widget? trailing;
  final String? subtitle;
  final VoidCallback? onTap;
  final PopoverTone tone;
  final bool selected;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final color = switch (tone) {
      PopoverTone.normal => t.text,
      PopoverTone.accent => t.accent,
      PopoverTone.muted => t.text3,
      PopoverTone.danger => t.danger,
    };

    final item = InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(t.rTile),
      child: Container(
        width: double.infinity,
        padding: EdgeInsets.symmetric(horizontal: t.sp * 0.75, vertical: t.sp),
        decoration: BoxDecoration(
          color: selected ? t.accentSoft : null,
          borderRadius: BorderRadius.circular(t.rTile),
        ),
        child: Row(
          children: [
            if (leading != null) ...[leading!, SizedBox(width: t.sp)],
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: popoverItemStyle(t).copyWith(color: color),
                  ),
                  if (subtitle != null)
                    Text(
                      subtitle!,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.labelSize,
                        color: t.text3,
                      ),
                    ),
                ],
              ),
            ),
            if (trailing != null) ...[SizedBox(width: t.sp), trailing!],
          ],
        ),
      ),
    );
    return tooltip == null ? item : Tooltip(message: tooltip, child: item);
  }
}

enum PopoverTone { normal, accent, muted, danger }

/// A [PopoverItem]'s label face, also what menus measure to fit their lines.
TextStyle popoverItemStyle(StormTokens t) =>
    TextStyle(fontFamily: StormTokens.sansFamily, fontSize: t.codeSize);

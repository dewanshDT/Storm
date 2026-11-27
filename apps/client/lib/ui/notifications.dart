/// The notification inbox header and its rows.
library;

import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';

import 'tags.dart';
import 'tappable.dart';
import 'tokens.dart';

/// A panel's header: a title at `bodySize`/semibold — a compact popover's
/// own scale, not a full page's `titleSize` — an unread-count pill beside
/// it, and right-aligned icon actions (decision 161's redesign). Any panel
/// with the same shape should reuse this rather than hand-rolling a title
/// row.
class InboxHeader extends StatelessWidget {
  const InboxHeader({
    super.key,
    required this.title,
    this.unreadCount = 0,
    this.actions = const [],
  });

  final String title;
  final int unreadCount;
  final List<Widget> actions;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Row(
      children: [
        // `Expanded` (not `Flexible` beside a `Spacer`): the two would
        // split the free space evenly by their equal flex, truncating a
        // short title that has plenty of room — this claims all of it, so
        // only a title actually too long for what's left ellipsizes.
        Expanded(
          child: Row(
            children: [
              Flexible(
                child: Text(
                  title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.bodySize,
                    fontWeight: FontWeight.w600,
                    color: t.text,
                    height: 1.2,
                  ),
                ),
              ),
              if (unreadCount > 0) ...[
                SizedBox(width: t.sp * 0.75),
                CountBadge(count: unreadCount, color: t.accent, small: true),
              ],
            ],
          ),
        ),
        for (final (i, a) in actions.indexed) ...[
          if (i > 0) SizedBox(width: t.sp * 0.25),
          a,
        ],
      ],
    );
  }
}

/// The "···" that replaces a [NotificationRow]'s time on hover, focus or
/// long-press.
class _RowMenuButton extends StatelessWidget {
  const _RowMenuButton({super.key, required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Tappable(
      semanticLabel: 'More actions',
      onTap: onTap,
      borderRadius: BorderRadius.circular(t.rChip),
      child: Padding(
        padding: EdgeInsets.all(t.sp * 0.375),
        child: Icon(
          LucideIcons.ellipsis,
          size: t.labelSize * 1.2,
          color: t.text2,
        ),
      ),
    );
  }
}

/// One row in a notice list (decision 161's redesign): a leading
/// [KindBadge] or similar glyph, a title the caller builds as spans (so it
/// can bold part of it — a session's name), an optional meta line, an
/// optional status tag, and an optional quoted body (another voice's own
/// words). The trailing time swaps for a "···" menu on hover, keyboard
/// focus, or long-press; [focused] draws the keyboard's own ring.
class NotificationRow extends StatefulWidget {
  const NotificationRow({
    super.key,
    required this.leading,
    required this.title,
    this.subtitle,
    this.tag,
    this.body,
    this.unread = false,
    this.time,
    this.focused = false,
    this.semanticLabel,
    this.onTap,
    this.onLongPress,
    this.onMenu,
  });

  final Widget leading;

  /// Built by the caller so it can bold the part that names a session.
  final List<InlineSpan> title;
  final String? subtitle;

  /// A held reason, drawn beside [subtitle].
  final Widget? tag;

  /// An agent's or a terminal's own words, quoted underneath.
  final Widget? body;
  final bool unread;
  final String? time;

  /// The keyboard's own ring (↑/↓ in the inbox).
  final bool focused;
  final String? semanticLabel;
  final VoidCallback? onTap;
  final VoidCallback? onLongPress;

  /// Opens this row's menu, anchored to the key this receives.
  final void Function(GlobalKey anchor)? onMenu;

  @override
  State<NotificationRow> createState() => _NotificationRowState();
}

class _NotificationRowState extends State<NotificationRow> {
  final _anchor = GlobalKey();
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final showMenu = widget.onMenu != null && (_hover || widget.focused);
    final content = Padding(
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 0.75,
        vertical: t.sp * 0.875,
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          widget.leading,
          SizedBox(width: t.sp * 1.25),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Expanded(
                      child: Text.rich(
                        TextSpan(children: widget.title),
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontFamily: StormTokens.sansFamily,
                          fontSize: t.codeSize,
                          fontWeight: widget.unread
                              ? FontWeight.w500
                              : FontWeight.w400,
                          // `text3`, not `text2`: weight alone read too
                          // close between read and unread (coordinator
                          // review item 6).
                          color: widget.unread ? t.text : t.text3,
                          height: 1.35,
                        ),
                      ),
                    ),
                    if (widget.unread) ...[
                      SizedBox(width: t.sp * 0.5),
                      Padding(
                        padding: EdgeInsets.only(top: t.sp * 0.5),
                        child: Container(
                          width: t.sp * 0.625,
                          height: t.sp * 0.625,
                          decoration: BoxDecoration(
                            shape: BoxShape.circle,
                            color: t.accent,
                          ),
                        ),
                      ),
                    ],
                  ],
                ),
                if (widget.subtitle != null || widget.tag != null) ...[
                  SizedBox(height: t.sp * 0.3),
                  Row(
                    children: [
                      if (widget.subtitle != null)
                        Flexible(
                          child: Text(
                            widget.subtitle!,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              fontFamily: StormTokens.sansFamily,
                              fontSize: t.labelSize * 1.09,
                              color: t.text3,
                            ),
                          ),
                        ),
                      if (widget.subtitle != null && widget.tag != null)
                        SizedBox(width: t.sp * 0.75),
                      if (widget.tag != null) Flexible(child: widget.tag!),
                    ],
                  ),
                ],
                if (widget.body != null) ...[
                  SizedBox(height: t.sp * 0.5),
                  widget.body!,
                ],
              ],
            ),
          ),
          SizedBox(width: t.sp * 0.5),
          SizedBox(
            width: t.sp * 2.75,
            child: Align(
              alignment: Alignment.topRight,
              child: showMenu
                  ? _RowMenuButton(
                      key: _anchor,
                      onTap: () => widget.onMenu!(_anchor),
                    )
                  : widget.time == null
                  ? const SizedBox.shrink()
                  : Padding(
                      padding: EdgeInsets.only(top: t.sp * 0.25),
                      child: Text(
                        widget.time!,
                        style: TextStyle(
                          fontFamily: StormTokens.monoFamily,
                          fontSize: t.labelSize,
                          color: t.text3,
                        ),
                      ),
                    ),
            ),
          ),
        ],
      ),
    );

    final row = Semantics(
      label: widget.semanticLabel,
      button: true,
      excludeSemantics: widget.semanticLabel != null,
      onTap: widget.onTap,
      child: Container(
        decoration: widget.focused
            ? BoxDecoration(
                border: Border.all(color: t.accent, width: t.bw * 1.5),
                borderRadius: BorderRadius.circular(t.rControl),
              )
            : null,
        child: InkWell(
          onTap: widget.onTap,
          onLongPress:
              widget.onLongPress ??
              (widget.onMenu == null ? null : () => widget.onMenu!(_anchor)),
          borderRadius: BorderRadius.circular(t.rControl),
          child: content,
        ),
      ),
    );
    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: row,
    );
  }
}

/// A folded run of a session's consecutive notices (decision 161's
/// redesign, item 5): "N updates", the newest one's own line under it, and
/// a chevron that expands the individual rows.
class NotificationGroupRow extends StatelessWidget {
  const NotificationGroupRow({
    super.key,
    required this.leading,
    required this.title,
    this.subtitle,
    this.unread = false,
    required this.expanded,
    required this.onToggle,
    this.focused = false,
    this.semanticLabel,
  });

  final Widget leading;

  /// Built by the caller so it can bold the session's name.
  final List<InlineSpan> title;
  final String? subtitle;
  final bool unread;
  final bool expanded;
  final VoidCallback onToggle;
  final bool focused;
  final String? semanticLabel;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Semantics(
      label: semanticLabel,
      button: true,
      excludeSemantics: semanticLabel != null,
      onTap: onToggle,
      child: Container(
        decoration: focused
            ? BoxDecoration(
                border: Border.all(color: t.accent, width: t.bw * 1.5),
                borderRadius: BorderRadius.circular(t.rControl),
              )
            : null,
        child: InkWell(
          onTap: onToggle,
          borderRadius: BorderRadius.circular(t.rControl),
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 0.75,
              vertical: t.sp * 0.875,
            ),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                leading,
                SizedBox(width: t.sp * 1.25),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Row(
                        children: [
                          Expanded(
                            child: Text.rich(
                              TextSpan(children: title),
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(
                                fontFamily: StormTokens.sansFamily,
                                fontSize: t.codeSize,
                                fontWeight: unread
                                    ? FontWeight.w500
                                    : FontWeight.w400,
                                color: unread ? t.text : t.text3,
                              ),
                            ),
                          ),
                          if (unread) ...[
                            SizedBox(width: t.sp * 0.5),
                            Container(
                              width: t.sp * 0.625,
                              height: t.sp * 0.625,
                              decoration: BoxDecoration(
                                shape: BoxShape.circle,
                                color: t.accent,
                              ),
                            ),
                          ],
                        ],
                      ),
                      if (subtitle != null) ...[
                        SizedBox(height: t.sp * 0.3),
                        Text(
                          subtitle!,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontFamily: StormTokens.sansFamily,
                            fontSize: t.labelSize * 1.09,
                            color: t.text3,
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
                SizedBox(width: t.sp * 0.5),
                Icon(
                  expanded
                      ? LucideIcons.chevron_down
                      : LucideIcons.chevron_right,
                  size: t.labelSize * 1.2,
                  color: t.text3,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

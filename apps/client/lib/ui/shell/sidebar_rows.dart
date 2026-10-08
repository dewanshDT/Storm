import 'package:flutter/material.dart';

import '../tokens.dart';

/// A vault's name as a small outlined tag beside a cross-vault row.
class VaultTag extends StatelessWidget {
  const VaultTag(this.name, {super.key});

  final String name;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      padding: EdgeInsets.symmetric(horizontal: t.sp * 0.625),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(t.rControl * 0.5),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Text(
        name,
        maxLines: 1,
        style: TextStyle(
          fontFamily: StormTokens.monoFamily,
          fontSize: t.labelSize,
          color: t.text3,
          height: 1.4,
        ),
      ),
    );
  }
}

/// The tree's ▸ / ▾, drawn rather than typed: the glyphs are not in Plex.
class Twisty extends StatelessWidget {
  const Twisty({super.key, required this.open, this.color});

  final bool open;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return SizedBox(
      width: t.sp * 1.5,
      height: t.sp * 1.5,
      child: CustomPaint(
        painter: _TwistyPainter(open: open, color: color ?? t.text3),
      ),
    );
  }
}

class _TwistyPainter extends CustomPainter {
  _TwistyPainter({required this.open, required this.color});

  final bool open;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final c = size.center(Offset.zero);
    final r = size.shortestSide * 0.25;
    final path = open
        ? (Path()
            ..moveTo(c.dx - r, c.dy - r * 0.6)
            ..lineTo(c.dx + r, c.dy - r * 0.6)
            ..lineTo(c.dx, c.dy + r * 0.8))
        : (Path()
            ..moveTo(c.dx - r * 0.6, c.dy - r)
            ..lineTo(c.dx - r * 0.6, c.dy + r)
            ..lineTo(c.dx + r * 0.8, c.dy));
    canvas.drawPath(path..close(), Paint()..color = color);
  }

  @override
  bool shouldRepaint(_TwistyPainter old) =>
      old.open != open || old.color != color;
}

/// A note an agent changed since it was last opened here.
class UnseenDot extends StatelessWidget {
  const UnseenDot({super.key});

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Tooltip(
      message: 'Changed by an agent since you last opened it',
      child: Container(
        width: t.sp * 0.75,
        height: t.sp * 0.75,
        decoration: BoxDecoration(color: t.accent, shape: BoxShape.circle),
      ),
    );
  }
}

/// One row of a sidebar list: `text2`, or `text` on `surface2` when it is the
/// open thing (handoff §7.5).
class SidebarRow extends StatelessWidget {
  const SidebarRow({
    super.key,
    required this.label,
    this.leading,
    this.trailing,
    this.selected = false,
    this.unseen = false,
    this.indent = 0,
    this.vertical,
    this.onTap,
    this.onLongPress,
  });

  final String label;
  final Widget? leading;
  final Widget? trailing;
  final bool selected;
  final bool unseen;
  final double indent;

  /// Defaults to the tree's 5; recent rows pass 6.
  final double? vertical;
  final VoidCallback? onTap;
  final VoidCallback? onLongPress;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final radius = BorderRadius.circular(t.rControl);
    return Material(
      color: selected ? t.surface2 : Colors.transparent,
      borderRadius: radius,
      child: InkWell(
        onTap: onTap,
        onLongPress: onLongPress,
        borderRadius: radius,
        child: Padding(
          padding: EdgeInsets.fromLTRB(
            t.sp + indent,
            vertical ?? t.sp * 0.625,
            t.sp,
            vertical ?? t.sp * 0.625,
          ),
          child: Row(
            children: [
              if (leading != null) ...[leading!, SizedBox(width: t.sp * 0.75)],
              Expanded(
                child: Text(
                  label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: selected ? t.text : t.text2,
                    height: 1.35,
                  ),
                ),
              ),
              if (unseen) ...[SizedBox(width: t.sp), const UnseenDot()],
              if (trailing != null) ...[SizedBox(width: t.sp), trailing!],
            ],
          ),
        ),
      ),
    );
  }
}

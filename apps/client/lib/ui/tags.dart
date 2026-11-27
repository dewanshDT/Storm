/// Tags, chips, badges and small labels.
library;

import 'package:flutter/material.dart';

import 'tappable.dart';
import 'tokens.dart';

enum OutlineTagShape { pill, tag }

/// The one pill/tag primitive: [MonoTag], [StatusPill], [TagChip] and
/// [VaultTag] are thin wrappers over it. A border is drawn only when
/// [borderColor] is given.
class OutlineTag extends StatelessWidget {
  const OutlineTag({
    super.key,
    required this.label,
    this.color,
    this.tone = StormTone.neutral,
    this.borderColor,
    this.fill,
    this.leading,
    this.trailing,
    this.onTap,
    this.shape = OutlineTagShape.pill,
    this.mono = true,
    this.maxWidth,
    this.minWidth,
    this.padding,
    this.lineHeight,
    this.wrap = false,
    this.radius,
    this.trailingGap,
  });

  final String label;

  /// Text colour; defaults from [tone].
  final Color? color;
  final StormTone tone;
  final Color? borderColor;
  final Color? fill;
  final Widget? leading;
  final Widget? trailing;
  final VoidCallback? onTap;
  final OutlineTagShape shape;
  final bool mono;
  final double? maxWidth;
  final double? minWidth;

  /// `sp` across, a quarter `sp` down, by default.
  final EdgeInsetsGeometry? padding;
  final double? lineHeight;

  /// Lets a long label wrap rather than ellipsise, as prose tags do.
  final bool wrap;

  /// Overrides the [shape]'s corner radius.
  final double? radius;

  /// Space before [trailing]; `0.75 sp` by default.
  final double? trailingGap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final corners = BorderRadius.circular(
      radius ?? (shape == OutlineTagShape.pill ? t.rPill : t.rInner),
    );
    final gap = SizedBox(width: t.sp * 0.75);
    final text = Text(
      label,
      maxLines: wrap ? null : 1,
      overflow: wrap ? TextOverflow.clip : TextOverflow.ellipsis,
      style: TextStyle(
        fontFamily: mono ? StormTokens.monoFamily : StormTokens.sansFamily,
        fontSize: t.labelSize,
        color: color ?? tone.color(t),
        height: lineHeight,
      ),
    );
    final tag = Container(
      constraints: (maxWidth != null || minWidth != null)
          ? BoxConstraints(
              minWidth: minWidth ?? 0,
              maxWidth: maxWidth ?? double.infinity,
            )
          : null,
      padding:
          padding ??
          EdgeInsets.symmetric(horizontal: t.sp, vertical: t.sp * 0.25),
      decoration: BoxDecoration(
        color: fill,
        borderRadius: corners,
        border: borderColor == null
            ? null
            : Border.all(color: borderColor!, width: t.bw),
      ),
      child: leading == null && trailing == null
          ? text
          : Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (leading != null) ...[leading!, gap],
                Flexible(child: text),
                if (trailing != null) ...[
                  trailingGap == null ? gap : SizedBox(width: trailingGap),
                  trailing!,
                ],
              ],
            ),
    );
    if (onTap == null) return tag;
    return Tappable(onTap: onTap, borderRadius: corners, child: tag);
  }
}

/// A tag, as it appears inline in a note and in the tag browser.
///
/// Amber on amber-soft, and deliberately about 26px tall rather than a 48px tap
/// target: tags sit *inside* prose, and a control-sized chip there would break
/// the line rhythm. Where one needs to be tappable the hit area is expanded
/// around it rather than the chip being grown.
class TagChip extends StatelessWidget {
  const TagChip({super.key, required this.label, this.onTap});

  final String label;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return OutlineTag(
      label: label,
      color: t.amber,
      fill: t.amberSoft,
      lineHeight: 1.3,
      wrap: true,
      onTap: onTap,
    );
  }
}

/// A vault's name as a small outlined tag beside a cross-vault row.
class VaultTag extends StatelessWidget {
  const VaultTag(this.name, {super.key, this.color});

  final String name;

  /// Over `text3`, for a row whose ground `text3` is not readable on.
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return OutlineTag(
      label: name,
      color: color ?? t.text3,
      borderColor: t.border,
      shape: OutlineTagShape.tag,
      padding: EdgeInsets.symmetric(horizontal: t.sp * 0.625),
      lineHeight: 1.4,
    );
  }
}

/// A vault's name as a small outlined mono tag, for cross-vault rows.
class MonoTag extends StatelessWidget {
  const MonoTag(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return OutlineTag(
      label: text,
      borderColor: t.border,
      shape: OutlineTagShape.tag,
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 0.75,
        vertical: t.sp * 0.125,
      ),
    );
  }
}

/// A frontmatter key, in the properties list.
///
/// Mono, because a key is an identifier the user typed and will grep for — the
/// same reason paths and version numbers are mono throughout.
class KeyChip extends StatelessWidget {
  const KeyChip({super.key, required this.label});

  final String label;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      padding: EdgeInsets.symmetric(horizontal: t.sp, vertical: t.sp * 0.35),
      decoration: BoxDecoration(
        color: t.surface2,
        borderRadius: BorderRadius.circular(t.rChip),
      ),
      child: Text(
        label,
        style: TextStyle(
          fontFamily: StormTokens.monoFamily,
          fontSize: t.labelSize,
          color: t.text2,
          height: 1.3,
        ),
      ),
    );
  }
}

/// A section label — `RECENTLY OPENED`, `VAULTS`, `PROPERTIES`.
///
/// Uppercase and tracked rather than a heading, so it groups without competing
/// with the content under it.
class SectionLabel extends StatelessWidget {
  const SectionLabel(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Text(text.toUpperCase(), style: t.labelStyle);
  }
}

/// A small pill count, on a card's corner or beside a rail icon — `NeedsYou`
/// (decision 153e) and the notification bell's unread count (decision 161)
/// are both this, in their own colour.
class CountBadge extends StatelessWidget {
  const CountBadge({
    super.key,
    required this.count,
    required this.color,
    this.small = false,
  });

  final int count;
  final Color color;

  /// The bell's own cap: "9+" at a tighter size, rather than "99+" at the
  /// card-corner size — a badge that small cannot hold three digits.
  final bool small;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final side = small ? t.sp * 1.625 : t.sp * 2.125;
    final cap = small ? 9 : 99;
    return IgnorePointer(
      child: Container(
        constraints: BoxConstraints(minWidth: side),
        height: side,
        padding: EdgeInsets.symmetric(horizontal: t.sp * 0.4),
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: color,
          borderRadius: BorderRadius.circular(t.rPill),
        ),
        child: Text(
          count > cap ? '$cap+' : '$count',
          style: TextStyle(
            fontFamily: StormTokens.monoFamily,
            fontSize: small ? t.labelSize * 0.85 : t.labelSize,
            height: 1,
            fontWeight: FontWeight.w600,
            color: t.onAccent,
          ),
        ),
      ),
    );
  }
}

/// A round icon button with a [CountBadge] on its corner, none at 0 —
/// the app bar's bell (decision 161) and anything else that is an icon
/// opening something with an unread count.
class IconBadgeButton extends StatelessWidget {
  const IconBadgeButton({
    super.key,
    required this.icon,
    required this.tooltip,
    required this.onTap,
    this.count = 0,
    this.badgeColor,
    this.dimension,
    this.anchorKey,
    this.smallBadge = false,
  });

  final IconData icon;
  final String tooltip;
  final VoidCallback onTap;

  /// A [CountBadge] on the corner; none at 0.
  final int count;

  /// `danger` by default.
  final Color? badgeColor;
  final double? dimension;

  /// The bell's "9+" variant (decision 161's redesign) rather than the
  /// card-corner "99+" one.
  final bool smallBadge;

  /// So a caller can anchor a popover to this button.
  final Key? anchorKey;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final side = dimension ?? t.sp * 3.5;
    final spoken = count == 0 ? tooltip : '$tooltip, $count unread';
    return Tappable(
      semanticLabel: spoken,
      borderRadius: BorderRadius.circular(t.rControl),
      onTap: onTap,
      child: SizedBox.square(
        key: anchorKey,
        dimension: side,
        child: Stack(
          clipBehavior: Clip.none,
          children: [
            Center(
              child: Icon(icon, size: t.uiSize, color: t.text2),
            ),
            if (count > 0)
              Positioned(
                top: -t.sp * 0.25,
                right: -t.sp * 0.375,
                child: CountBadge(
                  count: count,
                  color: badgeColor ?? t.danger,
                  small: smallBadge,
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// A small tinted circle around an icon, or a custom glyph in place of one
/// — the inbox's own leading mark for what a notice is (decision 161's
/// redesign): a question for needs you, danger for an error, and so on,
/// each kind its own colour. Any other list of typed, iconable things
/// should reuse this rather than a one-off tinted `Container`.
class KindBadge extends StatelessWidget {
  const KindBadge({
    super.key,
    this.icon,
    this.glyph,
    required this.color,
    this.size,
  }) : assert(icon != null || glyph != null, 'needs an icon or a glyph');

  /// Drawn in [color]; ignored when [glyph] is given.
  final IconData? icon;

  /// A custom mark — a provider's own glyph — already coloured by its
  /// caller, drawn in place of [icon].
  final Widget? glyph;
  final Color color;
  final double? size;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final side = size ?? t.sp * 4;
    return Container(
      width: side,
      height: side,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: Color.alphaBlend(color.withValues(alpha: 0.16), t.surface),
        shape: BoxShape.circle,
      ),
      child: glyph ?? Icon(icon, size: side * 0.55, color: color),
    );
  }
}

/// A first-segment heading over its child tags.
class TagGroup extends StatelessWidget {
  const TagGroup({
    super.key,
    required this.name,
    required this.tags,
    this.onTagTap,
  });

  final String name;
  final List<String> tags;
  final void Function(String tag)? onTagTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          name,
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.codeSize,
            fontWeight: FontWeight.w600,
            color: t.text2,
          ),
        ),
        SizedBox(height: t.sp * 0.75),
        Wrap(
          spacing: t.sp * 0.75,
          runSpacing: t.sp * 0.75,
          children: [
            for (final tag in tags)
              TagChip(
                label: tag,
                onTap: onTagTap == null ? null : () => onTagTap!(tag),
              ),
          ],
        ),
      ],
    );
  }
}

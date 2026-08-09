/// The shared widgets every screen is assembled from.
///
/// Each one reads its size, colour and radius from [StormTokens] and decides
/// nothing for itself — a change to a token input moves every screen together.
/// The `/gallery` route renders the whole set in all three themes at once.
library;

import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter/services.dart' show TextInputFormatter;

import 'tokens.dart';

/// Synced, syncing, or offline.
///
/// One dot, one meaning, everywhere it appears — green is synced and nothing
/// else, grey is offline and inactive, amber is work in progress.
class StatusDot extends StatelessWidget {
  const StatusDot({super.key, required this.status, this.size = 8, this.ring});

  final DotStatus status;
  final double size;

  /// The ground to punch the dot out of where it overlaps something — the
  /// vault bubble's corner. Null everywhere the dot sits in open space.
  final Color? ring;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: switch (status) {
          DotStatus.synced => t.green,
          DotStatus.syncing => t.amber,
          DotStatus.offline => t.text3,
        },
        border: ring == null
            ? null
            : Border.all(color: ring!, width: t.bw * 1.5),
      ),
    );
  }
}

/// Dark ink for anything sitting on an accent tile.
///
/// The tile is always a light colour whatever the theme, so this does not
/// re-derive per preset — and the accent test measures this exact pairing.
const kTileInk = Color(0xFF1A1626);

enum DotStatus { synced, syncing, offline }

/// The one place the engine's three flags become a dot, so the vault bubble,
/// the vault card and the popover cannot disagree about what "synced" is.
DotStatus dotStatusFor({
  required bool online,
  required bool syncing,
  required int pending,
}) {
  if (!online) return DotStatus.offline;
  if (syncing || pending > 0) return DotStatus.syncing;
  return DotStatus.synced;
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
    final chip = Container(
      padding: EdgeInsets.symmetric(horizontal: t.sp, vertical: t.sp * 0.25),
      decoration: BoxDecoration(
        color: t.amberSoft,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Text(
        label,
        style: TextStyle(
          fontFamily: StormTokens.monoFamily,
          fontSize: t.labelSize,
          color: t.amber,
          height: 1.3,
        ),
      ),
    );
    if (onTap == null) return chip;
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(999),
      child: chip,
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
        borderRadius: BorderRadius.circular(t.rControl * 0.6),
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

/// Where a note is in its save cycle.
///
/// Mono and colour-coded, because it is read at a glance and never actually
/// *read*: green means the server has it, amber means it is waiting, danger
/// means it did not go.
class SaveStateLabel extends StatelessWidget {
  const SaveStateLabel({super.key, required this.label, required this.tone});

  final String label;
  final SaveTone tone;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Text(
      label,
      style: TextStyle(
        fontFamily: StormTokens.monoFamily,
        fontSize: t.labelSize,
        color: switch (tone) {
          SaveTone.good => t.green,
          SaveTone.working => t.text3,
          SaveTone.waiting => t.amber,
          SaveTone.bad => t.danger,
        },
      ),
    );
  }
}

enum SaveTone { good, working, waiting, bad }

/// The product mark.
///
/// **The mint ground is fixed brand colour and does not re-theme.** It is the
/// one surface in the app the token layer deliberately cannot reach: a mark
/// that changes colour with the theme stops being a mark. The asset already
/// ships for the launcher icon, so this is the same image the OS shows.
class BrandMark extends StatelessWidget {
  const BrandMark({super.key, this.size = 28, this.withWordmark = false});

  final double size;
  final bool withWordmark;

  /// Fixed. Not a token, and not derived.
  static const groundColor = Color(0xFF96F2D7);

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final mark = ClipRRect(
      borderRadius: BorderRadius.circular(size * 0.28),
      child: Image.asset(
        'assets/icon/storm_icon_full.png',
        width: size,
        height: size,
        fit: BoxFit.cover,
      ),
    );
    if (!withWordmark) return mark;

    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        mark,
        SizedBox(width: t.sp),
        Text(
          'STORM',
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.labelSize,
            fontWeight: FontWeight.w600,
            letterSpacing: 1.6,
            color: t.text2,
          ),
        ),
      ],
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
    return Text(
      text.toUpperCase(),
      style: TextStyle(
        fontFamily: StormTokens.sansFamily,
        fontSize: t.labelSize,
        fontWeight: FontWeight.w600,
        letterSpacing: 1.1,
        color: t.text3,
      ),
    );
  }
}

/// A themed text field. Never sets a border: passing one discards the fill,
/// stroke and `rControl` radius that `inputDecorationTheme` supplies.
class StormInput extends StatelessWidget {
  const StormInput({
    super.key,
    this.controller,
    this.focusNode,
    this.hintText,
    this.labelText,
    this.prefixIcon,
    this.suffix,
    this.autofocus = false,
    this.obscureText = false,
    this.autocorrect = true,
    this.enabled = true,
    this.keyboardType,
    this.textCapitalization = TextCapitalization.none,
    this.textInputAction,
    this.inputFormatters,
    this.onChanged,
    this.onSubmitted,
  });

  final TextEditingController? controller;
  final FocusNode? focusNode;
  final String? hintText;
  final String? labelText;
  final IconData? prefixIcon;
  final Widget? suffix;
  final bool autofocus;
  final bool obscureText;
  final bool autocorrect;
  final bool enabled;
  final TextInputType? keyboardType;
  final TextCapitalization textCapitalization;
  final TextInputAction? textInputAction;
  final List<TextInputFormatter>? inputFormatters;
  final ValueChanged<String>? onChanged;
  final ValueChanged<String>? onSubmitted;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return TextField(
      controller: controller,
      focusNode: focusNode,
      autofocus: autofocus,
      obscureText: obscureText,
      autocorrect: autocorrect,
      enabled: enabled,
      keyboardType: keyboardType,
      textCapitalization: textCapitalization,
      textInputAction: textInputAction,
      inputFormatters: inputFormatters,
      onChanged: onChanged,
      onSubmitted: onSubmitted,
      style: TextStyle(
        fontFamily: StormTokens.sansFamily,
        fontSize: t.bodySize,
        color: t.text,
      ),
      decoration: InputDecoration(
        isDense: true,
        hintText: hintText,
        labelText: labelText,
        prefixIcon: prefixIcon == null
            ? null
            : Icon(prefixIcon, size: t.bodySize, color: t.text3),
        prefixIconConstraints: prefixIcon == null
            ? null
            : BoxConstraints(minWidth: t.sp * 4.5, minHeight: t.sp * 2),
        suffixIcon: suffix,
      ),
    );
  }
}

/// 34×20, so it sits inline at the end of a settings row. Material's own
/// `Switch` is a 52×32 slab with a hover halo.
class StormSwitch extends StatelessWidget {
  const StormSwitch({
    super.key,
    required this.value,
    required this.onChanged,
    this.enabled = true,
  });

  final bool value;
  final ValueChanged<bool>? onChanged;
  final bool enabled;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final height = t.sp * 2.5;
    final width = height * 1.7;
    final knob = height - t.sp * 0.5;
    final live = enabled && onChanged != null;

    return Opacity(
      opacity: live ? 1 : 0.5,
      child: GestureDetector(
        onTap: live ? () => onChanged!(!value) : null,
        child: AnimatedContainer(
          duration: t.duration,
          curve: Curves.easeOut,
          width: width,
          height: height,
          decoration: BoxDecoration(
            color: value ? t.accent : t.border,
            borderRadius: BorderRadius.circular(999),
          ),
          child: AnimatedAlign(
            duration: t.duration,
            curve: Curves.easeOut,
            alignment: value ? Alignment.centerRight : Alignment.centerLeft,
            child: Padding(
              padding: EdgeInsets.all(t.sp * 0.25),
              child: Container(
                width: knob,
                height: knob,
                decoration: const BoxDecoration(
                  color: Color(0xFFFFFFFF),
                  shape: BoxShape.circle,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// A rounded square at `rControl * 0.5`. Material's own is 18px of box inside
/// 48px of tap target, which cannot sit in a property row.
class StormCheckbox extends StatelessWidget {
  const StormCheckbox({
    super.key,
    required this.value,
    required this.onChanged,
    this.size,
  });

  final bool value;
  final ValueChanged<bool>? onChanged;
  final double? size;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final side = size ?? t.sp * 2.25;

    return GestureDetector(
      onTap: onChanged == null ? null : () => onChanged!(!value),
      behavior: HitTestBehavior.opaque,
      child: Container(
        width: side,
        height: side,
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: value ? t.accent : null,
          borderRadius: BorderRadius.circular(t.rControl * 0.5),
          border: value ? null : Border.all(color: t.text3, width: t.bw * 1.5),
        ),
        child: value
            ? Icon(LucideIcons.check, size: side * 0.72, color: t.onAccent)
            : null,
      ),
    );
  }
}

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
  });

  final String label;
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

    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(t.rControl * 0.8),
      child: Container(
        width: double.infinity,
        padding: EdgeInsets.symmetric(horizontal: t.sp * 0.75, vertical: t.sp),
        decoration: BoxDecoration(
          color: selected ? t.accentSoft : null,
          borderRadius: BorderRadius.circular(t.rControl * 0.8),
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
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.codeSize,
                      color: color,
                    ),
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
  }
}

enum PopoverTone { normal, accent, muted, danger }

/// A number and what it counts, as the dashboard shows them.
class StatBlock extends StatelessWidget {
  const StatBlock({super.key, required this.value, required this.label});

  final String value;
  final String label;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          value,
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.headingSize * 1.3,
            fontWeight: FontWeight.w600,
            color: t.text,
            height: 1.1,
          ),
        ),
        Text(
          label,
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.codeSize,
            color: t.text3,
          ),
        ),
      ],
    );
  }
}

// ---- rows, bars and cards -------------------------------------------

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
    this.dense = false,
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

  /// The sidebar's step down: the design's tree is set at 13px where a
  /// full-width list row is 15.
  final bool dense;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return InkWell(
      onTap: onTap,
      onLongPress: onLongPress,
      // Rounded only in the sidebar's tree, where a row is a standalone thing
      // you select. In a divided list it is a row, and a radius there draws a
      // card around every entry — and worse, Flutter renders a bottom-only
      // border on a rounded decoration as the bottom *arc* of that rect, so
      // the rule under the last row curled up at both ends.
      borderRadius: dense
          ? BorderRadius.circular(t.rControl)
          : BorderRadius.zero,
      child: Container(
        padding:
            padding ??
            EdgeInsets.symmetric(horizontal: t.sp * 1.75, vertical: t.sp * 2),
        decoration: BoxDecoration(
          color: selected ? t.accentSoft : null,
          borderRadius: dense ? BorderRadius.circular(t.rControl) : null,
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
                      fontSize: dense ? t.codeSize : t.bodySize * 0.95,
                      fontWeight: dense ? FontWeight.w400 : FontWeight.w600,
                      color: dense ? t.text2 : t.text,
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
    this.chevron = true,
    this.padding,
    this.dense = false,
  });

  final String name;
  final int? count;
  final VoidCallback? onTap;
  final VoidCallback? onLongPress;
  final bool divider;

  /// Replaces the folder glyph — the sidebar tree passes its twisty.
  final Widget? leading;
  final bool chevron;
  final EdgeInsetsGeometry? padding;

  /// See [NoteRow.dense].
  final bool dense;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return InkWell(
      onTap: onTap,
      onLongPress: onLongPress,
      borderRadius: dense
          ? BorderRadius.circular(t.rControl)
          : BorderRadius.zero,
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
            leading ??
                Icon(LucideIcons.folder, size: t.bodySize, color: t.text2),
            SizedBox(width: dense ? t.sp : t.sp * 1.5),
            Expanded(
              child: Text(
                name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: dense ? t.codeSize : t.bodySize * 0.95,
                  fontWeight: FontWeight.w500,
                  color: dense ? t.text2 : t.text,
                ),
              ),
            ),
            if (count != null)
              Text(
                '$count',
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: dense ? t.labelSize : t.codeSize,
                  color: t.text3,
                ),
              ),
            if (chevron) ...[
              SizedBox(width: t.sp * 0.75),
              Icon(LucideIcons.chevron_right, size: t.bodySize, color: t.text2),
            ],
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

/// `v12 · Saved`, with the save error if there is one.
class StatusBar extends StatelessWidget {
  const StatusBar({
    super.key,
    required this.version,
    required this.label,
    required this.tone,
    this.error,
  });

  final int version;
  final String label;
  final SaveTone tone;
  final String? error;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final mono = TextStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: t.labelSize,
      color: t.text3,
    );

    return Row(
      children: [
        Text('v$version', style: mono),
        Text('  ·  ', style: mono),
        SaveStateLabel(label: label, tone: tone),
        const Spacer(),
        if (error != null) ...[
          Icon(
            LucideIcons.circle_alert,
            size: t.labelSize * 1.2,
            color: t.danger,
          ),
          SizedBox(width: t.sp * 0.5),
          Flexible(
            child: Text(
              error!,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: mono.copyWith(color: t.danger),
            ),
          ),
        ],
      ],
    );
  }
}

/// A vault on the dashboard: initial on an accent tile, name, count, status.
class VaultCard extends StatelessWidget {
  const VaultCard({
    super.key,
    required this.name,
    required this.tile,
    required this.subtitle,
    this.status,
    this.muted = false,
    this.tinted = true,
    this.onTap,
    this.onLongPress,
  });

  final String name;

  /// The accent ground behind the initial, already resolved for this theme.
  final Color tile;

  /// False when [tile] is the neutral surface rather than an accent, which is
  /// what an uncoloured vault gets. [kTileInk] on it is black on near-black.
  final bool tinted;
  final String subtitle;
  final DotStatus? status;
  final bool muted;
  final VoidCallback? onTap;
  final VoidCallback? onLongPress;

  static String initialOf(String name) {
    final trimmed = name.trim();
    return trimmed.isEmpty ? 'S' : trimmed.characters.first.toUpperCase();
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final side = t.sp * 5;

    return InkWell(
      borderRadius: BorderRadius.circular(t.rCard),
      onTap: onTap,
      onLongPress: onLongPress,
      child: Container(
        padding: EdgeInsets.all(t.cardPad * 0.6),
        decoration: BoxDecoration(
          color: t.surface,
          borderRadius: BorderRadius.circular(t.rCard),
          border: Border.all(
            color: muted ? t.danger.withValues(alpha: 0.5) : t.border,
            width: t.bw,
          ),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: side,
              height: side,
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: muted ? t.danger.withValues(alpha: 0.15) : tile,
                borderRadius: BorderRadius.circular(t.rControl),
              ),
              child: Text(
                initialOf(name),
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  fontWeight: FontWeight.w600,
                  color: muted
                      ? t.danger
                      : tinted
                      ? kTileInk
                      : t.text2,
                ),
              ),
            ),
            SizedBox(height: t.sp * 1.25),
            Text(
              name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                fontWeight: FontWeight.w600,
                color: t.text,
              ),
            ),
            SizedBox(height: t.sp * 0.5),
            Row(
              children: [
                if (status != null) ...[
                  StatusDot(status: status!, size: t.sp * 0.75),
                  SizedBox(width: t.sp * 0.75),
                ],
                Flexible(
                  child: Text(
                    subtitle,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.labelSize,
                      color: t.text3,
                    ),
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

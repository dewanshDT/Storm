/// The smallest pieces of the design system.
///
/// Each one reads its size, colour and radius from [StormTokens] and decides
/// nothing for itself. That is the contract the whole system rests on: a change
/// to an input moves every atom, and therefore every screen, together.
///
/// From section 02 of `docs/design_handoff_storm_design_system/`. The `/gallery`
/// route renders all of them in all three themes at once.
library;

import 'package:flutter/material.dart';

import 'tokens.dart';

/// Synced, syncing, or offline.
///
/// One dot, one meaning, everywhere it appears — green is synced and nothing
/// else, grey is offline and inactive, amber is work in progress.
class StatusDot extends StatelessWidget {
  const StatusDot({super.key, required this.status, this.size = 8});

  final DotStatus status;
  final double size;

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
      ),
    );
  }
}

enum DotStatus { synced, syncing, offline }

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

/// Surfaces: the bordered card and the hairline rule box.
library;

import 'package:flutter/material.dart';

import 'tappable.dart';
import 'tokens.dart';

class StormCard extends StatelessWidget {
  const StormCard({
    super.key,
    required this.child,
    this.color,
    this.padding,
    this.density = StormDensity.regular,
    this.radius,
    this.border = true,
    this.borderColor,
    this.borderWidth,
    this.clip = false,
    this.onTap,
    this.onSecondaryTap,
  });

  final Widget child;

  /// `t.surface` by default.
  final Color? color;

  /// Defaults from [density].
  final EdgeInsetsGeometry? padding;
  final StormDensity density;

  /// `t.rCard` by default.
  final double? radius;
  final bool border;

  /// `t.border` by default.
  final Color? borderColor;

  /// `t.bw` by default.
  final double? borderWidth;
  final bool clip;
  final VoidCallback? onTap;
  final VoidCallback? onSecondaryTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final r = BorderRadius.circular(radius ?? t.rCard);
    final inset =
        padding ??
        EdgeInsets.all(switch (density) {
          StormDensity.compact => t.sp,
          StormDensity.regular => t.sp * 1.5,
          StormDensity.comfortable => t.sp * 2,
        });
    final side = border
        ? BorderSide(color: borderColor ?? t.border, width: borderWidth ?? t.bw)
        : BorderSide.none;
    if (onTap == null && onSecondaryTap == null) {
      return Container(
        padding: inset,
        clipBehavior: clip ? Clip.antiAlias : Clip.none,
        decoration: BoxDecoration(
          color: color ?? t.surface,
          borderRadius: r,
          border: border ? Border.fromBorderSide(side) : null,
        ),
        child: child,
      );
    }
    return Material(
      color: color ?? t.surface,
      clipBehavior: clip ? Clip.antiAlias : Clip.none,
      shape: RoundedRectangleBorder(borderRadius: r, side: side),
      child: Tappable(
        onTap: onTap,
        onSecondaryTap: onSecondaryTap,
        borderRadius: r,
        child: Padding(padding: inset, child: child),
      ),
    );
  }
}

/// A notice in a flow: a [StormCard] on `surface2` and the control radius.
class NoticeBanner extends StatelessWidget {
  const NoticeBanner({
    super.key,
    required this.child,
    this.color,
    this.borderColor,
    this.borderWidth,
    this.padding,
  });

  final Widget child;

  /// `t.surface2` by default.
  final Color? color;
  final Color? borderColor;
  final double? borderWidth;

  /// `2 sp` across, `1.5 sp` down, by default.
  final EdgeInsetsGeometry? padding;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return StormCard(
      color: color ?? t.surface2,
      radius: t.rControl,
      borderColor: borderColor,
      borderWidth: borderWidth,
      padding:
          padding ??
          EdgeInsets.symmetric(horizontal: t.sp * 2, vertical: t.sp * 1.5),
      child: child,
    );
  }
}

/// A 1px `t.border` rule on the chosen edges around [child].
class Hairline extends StatelessWidget {
  const Hairline({
    super.key,
    this.child,
    this.top = false,
    this.bottom = false,
    this.left = false,
    this.right = false,
    this.padding,
  });

  final Widget? child;
  final bool top;
  final bool bottom;
  final bool left;
  final bool right;
  final EdgeInsetsGeometry? padding;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final rule = BorderSide(color: t.border, width: t.bw);
    return Container(
      padding: padding,
      decoration: BoxDecoration(
        border: Border(
          top: top ? rule : BorderSide.none,
          bottom: bottom ? rule : BorderSide.none,
          left: left ? rule : BorderSide.none,
          right: right ? rule : BorderSide.none,
        ),
      ),
      child: child,
    );
  }
}

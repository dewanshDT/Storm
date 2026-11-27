/// Buttons and the toggle.
library;

import 'package:flutter/material.dart';

import 'tokens.dart';

enum StormButtonKind { primary, outline, soft, danger, text, dangerText }

class StormButton extends StatelessWidget {
  const StormButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.kind = StormButtonKind.primary,
    this.icon,
    this.expand = false,
    this.color,
  });

  const StormButton.primary({
    Key? key,
    required String label,
    required VoidCallback? onPressed,
    IconData? icon,
    bool expand = false,
  }) : this(
         key: key,
         label: label,
         onPressed: onPressed,
         icon: icon,
         expand: expand,
       );

  const StormButton.outline({
    Key? key,
    required String label,
    required VoidCallback? onPressed,
    IconData? icon,
    bool expand = false,
  }) : this(
         key: key,
         label: label,
         onPressed: onPressed,
         kind: StormButtonKind.outline,
         icon: icon,
         expand: expand,
       );

  const StormButton.soft({
    Key? key,
    required String label,
    required VoidCallback? onPressed,
    IconData? icon,
    bool expand = false,
  }) : this(
         key: key,
         label: label,
         onPressed: onPressed,
         kind: StormButtonKind.soft,
         icon: icon,
         expand: expand,
       );

  const StormButton.danger({
    Key? key,
    required String label,
    required VoidCallback? onPressed,
    IconData? icon,
    bool expand = false,
  }) : this(
         key: key,
         label: label,
         onPressed: onPressed,
         kind: StormButtonKind.danger,
         icon: icon,
         expand: expand,
       );

  const StormButton.text({
    Key? key,
    required String label,
    required VoidCallback? onPressed,
    IconData? icon,
  }) : this(
         key: key,
         label: label,
         onPressed: onPressed,
         kind: StormButtonKind.text,
         icon: icon,
       );

  final String label;
  final VoidCallback? onPressed;
  final StormButtonKind kind;
  final IconData? icon;
  final bool expand;

  /// Replaces the kind's own colour: the fill for primary and danger, the
  /// ink for the others.
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final (fill, ink, stroke) = switch (kind) {
      StormButtonKind.primary => (color ?? t.accent, t.onAccent, null),
      StormButtonKind.outline => (null, color ?? t.text, t.border),
      StormButtonKind.soft => (t.accentSoft, color ?? t.accent, null),
      StormButtonKind.danger => (color ?? t.danger, t.onAccent, null),
      StormButtonKind.text => (null, color ?? t.text2, null),
      StormButtonKind.dangerText => (null, color ?? t.danger, null),
    };
    final padding = switch (kind) {
      StormButtonKind.primary || StormButtonKind.danger => EdgeInsets.symmetric(
        horizontal: t.sp * 2,
        vertical: t.sp * 1.125,
      ),
      StormButtonKind.outline => EdgeInsets.symmetric(
        horizontal: t.sp * 1.75,
        vertical: t.sp,
      ),
      StormButtonKind.soft => EdgeInsets.symmetric(
        horizontal: t.sp * 1.375,
        vertical: t.sp * 0.75,
      ),
      StormButtonKind.text || StormButtonKind.dangerText =>
        EdgeInsets.symmetric(horizontal: t.sp * 0.5, vertical: t.sp * 0.5),
    };
    final weight =
        kind == StormButtonKind.outline ||
            kind == StormButtonKind.text ||
            kind == StormButtonKind.dangerText
        ? FontWeight.w400
        : FontWeight.w500;
    final radius = BorderRadius.circular(t.rControl);
    final live = onPressed != null;

    return Semantics(
      button: true,
      enabled: live,
      child: Opacity(
        opacity: live ? 1 : 0.5,
        child: Material(
          color: fill ?? Colors.transparent,
          shape: RoundedRectangleBorder(
            borderRadius: radius,
            side: stroke == null
                ? BorderSide.none
                : BorderSide(color: stroke, width: t.bw),
          ),
          child: InkWell(
            onTap: onPressed,
            borderRadius: radius,
            child: Padding(
              padding: padding,
              child: Row(
                mainAxisSize: expand ? MainAxisSize.max : MainAxisSize.min,
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  if (icon != null) ...[
                    Icon(icon, size: t.codeSize, color: ink),
                    SizedBox(width: t.sp * 0.75),
                  ],
                  Flexible(
                    child: Text(
                      label,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.codeSize,
                        fontWeight: weight,
                        color: ink,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// Track 38 × 22, knob 16 (handoff §7.4).
class StormToggle extends StatelessWidget {
  const StormToggle({
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
    final live = enabled && onChanged != null;

    return Semantics(
      toggled: value,
      enabled: live,
      child: Opacity(
        opacity: live ? 1 : 0.5,
        child: GestureDetector(
          onTap: live ? () => onChanged!(!value) : null,
          behavior: HitTestBehavior.opaque,
          child: AnimatedContainer(
            duration: t.durationOf(context),
            curve: t.curve,
            width: t.sp * 4.75,
            height: t.sp * 2.75,
            padding: EdgeInsets.all(t.sp * 0.25 - t.bw),
            decoration: BoxDecoration(
              color: value ? t.accent : t.surface2,
              borderRadius: BorderRadius.circular(t.rPill),
              border: Border.all(
                color: value ? t.accent : t.border,
                width: t.bw,
              ),
            ),
            child: AnimatedAlign(
              duration: t.durationOf(context),
              curve: t.curve,
              alignment: value ? Alignment.centerRight : Alignment.centerLeft,
              child: Container(
                width: t.sp * 2,
                height: t.sp * 2,
                decoration: BoxDecoration(
                  color: value ? t.onAccent : t.text3,
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

/// Storm v2 controls: buttons, the toggle, choice chips, settings rows and
/// numbered steps (handoff §7.4, §7.8).
library;

import 'package:flutter/material.dart';

import 'tokens.dart';

enum StormButtonKind { primary, outline, soft, danger, text }

class StormButton extends StatelessWidget {
  const StormButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.kind = StormButtonKind.primary,
    this.icon,
    this.expand = false,
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

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final (fill, ink, stroke) = switch (kind) {
      StormButtonKind.primary => (t.accent, t.onAccent, null),
      StormButtonKind.outline => (null, t.text, t.border),
      StormButtonKind.soft => (t.accentSoft, t.accent, null),
      StormButtonKind.danger => (t.danger, t.onAccent, null),
      StormButtonKind.text => (null, t.text2, null),
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
      StormButtonKind.text => EdgeInsets.symmetric(
        horizontal: t.sp * 0.5,
        vertical: t.sp * 0.5,
      ),
    };
    final weight =
        kind == StormButtonKind.outline || kind == StormButtonKind.text
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
            duration: t.duration,
            curve: Curves.easeOut,
            width: t.sp * 4.75,
            height: t.sp * 2.75,
            padding: EdgeInsets.all(t.sp * 0.25 - t.bw),
            decoration: BoxDecoration(
              color: value ? t.accent : t.surface2,
              borderRadius: BorderRadius.circular(999),
              border: Border.all(
                color: value ? t.accent : t.border,
                width: t.bw,
              ),
            ),
            child: AnimatedAlign(
              duration: t.duration,
              curve: Curves.easeOut,
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

class ChoiceOption<T> {
  const ChoiceOption(this.value, this.label, {this.leading});

  final T value;
  final String label;
  final Widget? leading;
}

/// Outline chips; the selected one is `accent` on `accentSoft`.
class ChoiceChips<T> extends StatelessWidget {
  const ChoiceChips({
    super.key,
    required this.options,
    required this.selected,
    required this.onSelected,
  });

  final List<ChoiceOption<T>> options;
  final T? selected;
  final ValueChanged<T>? onSelected;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Wrap(
      spacing: t.sp,
      runSpacing: t.sp,
      children: [
        for (final o in options)
          _Chip(
            option: o,
            selected: o.value == selected,
            onTap: onSelected == null ? null : () => onSelected!(o.value),
          ),
      ],
    );
  }
}

class _Chip<T> extends StatelessWidget {
  const _Chip({required this.option, required this.selected, this.onTap});

  final ChoiceOption<T> option;
  final bool selected;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final radius = BorderRadius.circular(t.rControl);
    return Semantics(
      selected: selected,
      button: true,
      child: Material(
        color: selected ? t.accentSoft : Colors.transparent,
        shape: RoundedRectangleBorder(
          borderRadius: radius,
          side: BorderSide(color: selected ? t.accent : t.border, width: t.bw),
        ),
        child: InkWell(
          onTap: onTap,
          borderRadius: radius,
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 1.5,
              vertical: t.sp,
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (option.leading != null) ...[
                  option.leading!,
                  SizedBox(width: t.sp),
                ],
                Text(
                  option.label,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    fontWeight: selected ? FontWeight.w500 : FontWeight.w400,
                    color: selected ? t.accent : t.text,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// Label over an optional sub-line, with a control or action trailing and a
/// hairline under it (handoff §5.3).
class SettingsRow extends StatelessWidget {
  const SettingsRow({
    super.key,
    required this.label,
    this.sub,
    this.monoSub = false,
    this.leading,
    this.trailing,
    this.onTap,
    this.muted = false,
    this.divider = true,
    this.labelColor,
    this.large = false,
  });

  /// The phone settings list's 15px rows with 13px padding.
  final bool large;

  final String label;
  final String? sub;

  /// `danger` for a destructive row such as Disconnect.
  final Color? labelColor;

  /// Metadata (a path, a key prefix) rather than a sentence.
  final bool monoSub;
  final Widget? leading;
  final Widget? trailing;
  final VoidCallback? onTap;
  final bool muted;
  final bool divider;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final row = Container(
      padding: EdgeInsets.symmetric(vertical: t.sp * (large ? 1.625 : 1.5)),
      decoration: divider
          ? BoxDecoration(
              border: Border(
                bottom: BorderSide(color: t.border, width: t.bw),
              ),
            )
          : null,
      child: Row(
        children: [
          if (leading != null) ...[leading!, SizedBox(width: t.sp * 1.5)],
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  label,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: large ? t.uiSize * 1.05 : t.uiSize,
                    color: labelColor ?? (muted ? t.text3 : t.text),
                  ),
                ),
                if (sub != null) ...[
                  SizedBox(height: t.sp * 0.25),
                  Text(
                    sub!,
                    style: TextStyle(
                      fontFamily: monoSub
                          ? StormTokens.monoFamily
                          : StormTokens.sansFamily,
                      fontSize: monoSub ? t.labelSize : t.codeSize,
                      height: 1.45,
                      color: t.text3,
                    ),
                  ),
                ],
              ],
            ),
          ),
          if (trailing != null) ...[SizedBox(width: t.sp * 1.5), trailing!],
        ],
      ),
    );
    return onTap == null ? row : InkWell(onTap: onTap, child: row);
  }
}

class NumberedStep {
  const NumberedStep(this.text, {this.action});

  final String text;

  /// Shown under the current step only.
  final Widget? action;
}

/// 26px circles; only the [current] step is in `accent` (handoff §7.8).
class NumberedSteps extends StatelessWidget {
  const NumberedSteps({
    super.key,
    required this.steps,
    this.current = 0,
    this.inline = false,
  });

  final List<NumberedStep> steps;
  final int current;

  /// The current step's action beside its text rather than under it.
  final bool inline;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        for (var i = 0; i < steps.length; i++)
          Padding(
            padding: EdgeInsets.only(bottom: t.sp * 1.75),
            child: Row(
              crossAxisAlignment: inline
                  ? CrossAxisAlignment.center
                  : CrossAxisAlignment.start,
              children: [
                Container(
                  key: Key('step-${i + 1}'),
                  width: t.sp * 3.25,
                  height: t.sp * 3.25,
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    border: Border.all(
                      color: i == current ? t.accent : t.border,
                      width: t.bw,
                    ),
                  ),
                  child: Text(
                    '${i + 1}',
                    style: TextStyle(
                      fontFamily: StormTokens.monoFamily,
                      fontSize: t.labelSize,
                      color: i == current ? t.accent : t.text3,
                    ),
                  ),
                ),
                SizedBox(width: t.sp * 1.5),
                if (inline) ...[
                  Flexible(
                    child: Text(
                      steps[i].text,
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.uiSize,
                        color: i == current ? t.text : t.text3,
                      ),
                    ),
                  ),
                  if (i == current && steps[i].action != null) ...[
                    SizedBox(width: t.sp * 1.5),
                    steps[i].action!,
                  ],
                ] else
                  Expanded(
                    child: Padding(
                      padding: EdgeInsets.only(top: t.sp * 0.25),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(
                            steps[i].text,
                            style: TextStyle(
                              fontFamily: StormTokens.sansFamily,
                              fontSize: t.uiSize,
                              color: i == current ? t.text : t.text3,
                            ),
                          ),
                          if (i == current && steps[i].action != null) ...[
                            SizedBox(height: t.sp),
                            steps[i].action!,
                          ],
                        ],
                      ),
                    ),
                  ),
              ],
            ),
          ),
      ],
    );
  }
}

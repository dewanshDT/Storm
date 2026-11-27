/// Settings rows and numbered steps.
library;

import 'package:flutter/material.dart';

import 'buttons.dart';
import 'tokens.dart';

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
    final toggle = trailing is StormToggle;
    final text = Column(
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
    );
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
          // A toggle carries the row's words, so the switch is named by them.
          Expanded(child: toggle ? ExcludeSemantics(child: text) : text),
          if (trailing != null) ...[
            SizedBox(width: t.sp * 1.5),
            toggle
                ? Semantics(label: label, hint: sub, child: trailing)
                : trailing!,
          ],
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

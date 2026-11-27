/// Text input and checkboxes.
library;

import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter/services.dart' show TextInputFormatter;

import 'tokens.dart';

/// A themed text field. Never sets a border: passing one discards the fill,
/// stroke and `rControl` radius that `inputDecorationTheme` supplies.
class StormInput extends StatelessWidget {
  const StormInput({
    super.key,
    this.controller,
    this.focusNode,
    this.hintText,
    this.labelText,
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
    this.minLines,
    this.maxLines = 1,
  });

  final TextEditingController? controller;
  final FocusNode? focusNode;
  final String? hintText;
  final String? labelText;
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

  /// A multiline field ("What happened?") sets both this and [maxLines];
  /// the default keeps every existing single-line caller unchanged.
  final int? minLines;
  final int? maxLines;

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
      minLines: minLines,
      maxLines: maxLines,
      style: TextStyle(
        fontFamily: StormTokens.sansFamily,
        fontSize: t.bodySize,
        color: t.text,
      ),
      decoration: InputDecoration(
        isDense: true,
        hintText: hintText,
        labelText: labelText,
        suffixIcon: suffix,
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
          borderRadius: BorderRadius.circular(t.rInner),
          border: value ? null : Border.all(color: t.text3, width: t.bw * 1.5),
        ),
        child: value
            // System text scale must not inflate the mark past the box —
            // the box itself is already sized for the user's Storm font size.
            ? Icon(
                LucideIcons.check,
                size: side * 0.72,
                color: t.onAccent,
                applyTextScaling: false,
              )
            : null,
      ),
    );
  }
}

/// A [StormCheckbox] with its label (or anything else) beside it, wrapped so
/// tapping anywhere in the row — not just the small box — toggles it: the
/// shape that kept appearing hand-rolled (a report's "Include diagnostics",
/// `crew_add.dart`'s session picker).
class StormCheckboxRow extends StatelessWidget {
  const StormCheckboxRow({
    super.key,
    required this.value,
    required this.onChanged,
    required this.child,
    this.checkboxKey,
    this.checkboxSize,
    this.padding,
    this.borderRadius,
  });

  final bool value;
  final ValueChanged<bool>? onChanged;

  /// Everything after the checkbox — usually a `Text`, sometimes more (a
  /// trailing detail, as in `crew_add.dart`'s session rows).
  final Widget child;

  /// So a caller can still find the checkbox itself by key.
  final Key? checkboxKey;
  final double? checkboxSize;
  final EdgeInsetsGeometry? padding;
  final BorderRadius? borderRadius;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final live = onChanged != null;
    return Semantics(
      checked: value,
      enabled: live,
      child: Opacity(
        opacity: live ? 1 : 0.5,
        child: InkWell(
          onTap: live ? () => onChanged!(!value) : null,
          borderRadius: borderRadius ?? BorderRadius.circular(t.rChip),
          child: Padding(
            padding: padding ?? EdgeInsets.symmetric(vertical: t.sp * 0.5),
            child: Row(
              children: [
                StormCheckbox(
                  key: checkboxKey,
                  value: value,
                  size: checkboxSize,
                  onChanged: onChanged,
                ),
                SizedBox(width: t.sp),
                Expanded(child: child),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

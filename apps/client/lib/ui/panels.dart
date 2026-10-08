/// Pieces of a side panel: its tabs, key/value rows, an inline confirmation
/// and a small mono tag (handoff §3.6, §7.4).
library;

import 'package:flutter/material.dart';

import 'controls.dart';
import 'tokens.dart';

/// Active `accent` 600 on `accentSoft`; inactive `text3` 500.
class PanelTabs<T> extends StatelessWidget {
  const PanelTabs({
    super.key,
    required this.tabs,
    required this.selected,
    required this.onSelected,
  });

  final List<(T, String)> tabs;
  final T selected;
  final ValueChanged<T> onSelected;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final radius = BorderRadius.circular(t.rControl);
    return Row(
      children: [
        for (final (value, label) in tabs) ...[
          Semantics(
            selected: value == selected,
            button: true,
            label: label,
            excludeSemantics: true,
            onTap: () => onSelected(value),
            child: Material(
              color: value == selected ? t.accentSoft : Colors.transparent,
              borderRadius: radius,
              child: InkWell(
                key: Key('panel-tab-$value'),
                borderRadius: radius,
                onTap: () => onSelected(value),
                child: Padding(
                  padding: EdgeInsets.symmetric(
                    horizontal: t.sp * 1.5,
                    vertical: t.sp * 0.75,
                  ),
                  child: Text(
                    label,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.codeSize,
                      fontWeight: value == selected
                          ? FontWeight.w600
                          : FontWeight.w500,
                      color: value == selected ? t.accent : t.text3,
                    ),
                  ),
                ),
              ),
            ),
          ),
          SizedBox(width: t.sp * 0.5),
        ],
      ],
    );
  }
}

/// Mono uppercase keys over `text` values (the session's About).
class KeyValueList extends StatelessWidget {
  const KeyValueList({super.key, required this.rows});

  final List<(String, String)> rows;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        for (var i = 0; i < rows.length; i++)
          Padding(
            padding: EdgeInsets.only(
              bottom: i == rows.length - 1 ? 0 : t.sp * 1.75,
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  rows[i].$1.toUpperCase(),
                  style: TextStyle(
                    fontFamily: StormTokens.monoFamily,
                    fontSize: t.labelSize,
                    letterSpacing: t.labelSize * 0.08,
                    color: t.text3,
                  ),
                ),
                SizedBox(height: t.sp * 0.375),
                Text(
                  rows[i].$2,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.uiSize,
                    height: 1.45,
                    color: t.text,
                  ),
                ),
              ],
            ),
          ),
      ],
    );
  }
}

/// A confirmation in place of the thing it confirms (H11): copy, Cancel, and
/// a `danger` action. Replaces a modal dialog.
class InlineConfirm extends StatelessWidget {
  const InlineConfirm({
    super.key,
    required this.message,
    required this.confirmLabel,
    required this.onConfirm,
    required this.onCancel,
  });

  final String message;
  final String confirmLabel;
  final VoidCallback? onConfirm;
  final VoidCallback onCancel;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 1.75,
        vertical: t.sp * 1.5,
      ),
      decoration: BoxDecoration(
        color: t.surface2,
        borderRadius: BorderRadius.circular(t.rControl),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Row(
        children: [
          Expanded(
            child: Text(
              message,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                height: 1.45,
                color: t.text,
              ),
            ),
          ),
          SizedBox(width: t.sp * 1.25),
          StormButton.text(
            key: const Key('confirm-cancel'),
            label: 'Cancel',
            onPressed: onCancel,
          ),
          SizedBox(width: t.sp * 0.75),
          Semantics(
            button: true,
            enabled: onConfirm != null,
            label: confirmLabel,
            excludeSemantics: true,
            onTap: onConfirm,
            child: Material(
              color: t.danger,
              borderRadius: BorderRadius.circular(t.rControl),
              child: InkWell(
                key: const Key('confirm-action'),
                borderRadius: BorderRadius.circular(t.rControl),
                onTap: onConfirm,
                child: Padding(
                  padding: EdgeInsets.symmetric(
                    horizontal: t.sp * 1.25,
                    vertical: t.sp * 0.75,
                  ),
                  child: Text(
                    confirmLabel,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.codeSize,
                      color: t.onAccent,
                    ),
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
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
    return Container(
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 0.75,
        vertical: t.sp * 0.125,
      ),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(t.rControl * 0.5),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Text(
        text,
        style: TextStyle(
          fontFamily: StormTokens.monoFamily,
          fontSize: t.labelSize,
          color: t.text3,
        ),
      ),
    );
  }
}

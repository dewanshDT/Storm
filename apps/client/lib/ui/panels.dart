/// Pieces of a side panel: its tabs, key/value rows, an inline confirmation
/// and a small mono tag (handoff §3.6, §7.4).
library;

import 'package:flutter/material.dart';

import 'controls.dart';
import 'tokens.dart';

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
                Text(rows[i].$1.toUpperCase(), style: t.labelStyle),
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
    this.title,
  });

  /// Bold, before [message] — never run into it.
  final String? title;
  final String message;
  final String confirmLabel;
  final VoidCallback? onConfirm;
  final VoidCallback onCancel;

  /// Below this the message wraps above the actions instead of beside them.
  static const _singleRow = 560.0;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final words = Row(
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        if (title != null) ...[
          Text(
            title!,
            key: const Key('confirm-title'),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.codeSize,
              fontWeight: FontWeight.w600,
              color: t.text,
            ),
          ),
          SizedBox(width: t.sp * 2),
        ],
        Expanded(
          child: Text(
            message,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.codeSize,
              height: 1.4,
              color: t.text2,
            ),
          ),
        ),
      ],
    );
    final actions = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        StormButton.text(
          key: const Key('confirm-cancel'),
          label: 'Cancel',
          onPressed: onCancel,
        ),
        SizedBox(width: t.sp),
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
                    fontWeight: FontWeight.w600,
                    color: t.onAccent,
                  ),
                ),
              ),
            ),
          ),
        ),
      ],
    );
    return Container(
      padding: EdgeInsets.symmetric(horizontal: t.sp * 2, vertical: t.sp * 1.5),
      decoration: BoxDecoration(
        color: t.surface2,
        borderRadius: BorderRadius.circular(t.rControl),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: LayoutBuilder(
        builder: (context, box) => box.maxWidth >= _singleRow
            ? Row(
                children: [
                  Expanded(child: words),
                  SizedBox(width: t.sp * 2),
                  actions,
                ],
              )
            : Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  words,
                  SizedBox(height: t.sp),
                  Align(alignment: Alignment.centerRight, child: actions),
                ],
              ),
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
        borderRadius: BorderRadius.circular(t.rInner),
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

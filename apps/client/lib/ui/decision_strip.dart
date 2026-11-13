import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'controls.dart';
import 'tokens.dart';

/// Whether a strip waits on the person (an agent asked) or confirms what the
/// person just asked for (decision 153g).
enum DecisionTone { waiting, confirm }

/// Where a strip is drawn: under a header, or as the body of a sheet on a
/// phone, where its keys sit above their values and its buttons are stacked
/// full width.
enum DecisionLayout { strip, sheet }

enum DecisionActionKind { text, ghost, primary, danger }

class DecisionAction {
  const DecisionAction({
    required this.label,
    required this.kind,
    required this.onPressed,
    this.key,
  });

  final String label;
  final DecisionActionKind kind;
  final VoidCallback? onPressed;
  final Key? key;
}

/// One key and its value: "CLAUDE CODE SAYS" and the reason, "RUNNING" and
/// the names. [quote] draws the value as words someone else wrote.
class DecisionRow {
  const DecisionRow(
    this.label,
    this.value, {
    this.key,
    this.quote = false,
    this.more = const [],
  });

  final String label;
  final String value;
  final Key? key;
  final bool quote;

  /// Further lines under the value, quieter: a folder's first paths.
  final List<String> more;
}

typedef DecisionTick = ({
  bool value,
  ValueChanged<bool> onChanged,
  String label,
});

/// The one pattern for anything that needs the person to decide (153g): a
/// label column, a title, its key–value rows and the buttons, one decision
/// at a time under the header. Never a dialog.
class DecisionStrip extends StatefulWidget {
  const DecisionStrip({
    super.key,
    required this.label,
    required this.title,
    required this.actions,
    this.tone = DecisionTone.confirm,
    this.layout = DecisionLayout.strip,
    this.rows = const [],
    this.note,
    this.tick,
    this.onEscape,
    this.escapeAllowed,
  });

  /// "NEEDS YOU · 1 OF 2", "CONFIRM".
  final String label;
  final String title;
  final List<DecisionRow> rows;
  final String? note;
  final DecisionTick? tick;

  /// Left to right in a strip; top to bottom, primary first, in a sheet.
  final List<DecisionAction> actions;
  final DecisionTone tone;
  final DecisionLayout layout;

  /// What Esc does: Later for a request, Cancel for a confirmation.
  final VoidCallback? onEscape;

  /// Whether Esc is the strip's now; a focused terminal keeps its own Esc.
  final bool Function()? escapeAllowed;

  @override
  State<DecisionStrip> createState() => _DecisionStripState();
}

class _DecisionStripState extends State<DecisionStrip> {
  final _focus = FocusNode(debugLabel: 'decision-strip');

  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_key);
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_key);
    _focus.dispose();
    super.dispose();
  }

  /// Esc anywhere on this route, unless something above it or a focused
  /// terminal owns the key. A [HardwareKeyboard] handler runs before the
  /// focus tree sees the key, so claiming it here keeps it from everything
  /// beneath.
  bool _key(KeyEvent e) {
    final escape = widget.onEscape;
    if (escape == null ||
        e is! KeyDownEvent ||
        e.logicalKey != LogicalKeyboardKey.escape) {
      return false;
    }
    if (!mounted) return false;
    final route = ModalRoute.of(context);
    if (route != null && !route.isCurrent) return false;
    final inside = _focus.hasFocus;
    if (!inside && !(widget.escapeAllowed?.call() ?? true)) return false;
    scheduleMicrotask(() {
      if (mounted) escape();
    });
    return true;
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final body = widget.layout == DecisionLayout.sheet ? _sheet(t) : _strip(t);
    return Semantics(
      container: true,
      liveRegion: widget.tone == DecisionTone.waiting,
      child: Focus(
        focusNode: _focus,
        child: GestureDetector(
          behavior: HitTestBehavior.translucent,
          onTap: _focus.requestFocus,
          child: body,
        ),
      ),
    );
  }

  TextStyle _keyStyle(StormTokens t, {Color? color}) => TextStyle(
    fontFamily: StormTokens.monoFamily,
    fontSize: t.labelSize,
    letterSpacing: t.labelSize * 0.08,
    color: color ?? t.text3,
  );

  TextStyle _sans(StormTokens t, {Color? color, FontWeight? weight}) =>
      TextStyle(
        fontFamily: StormTokens.sansFamily,
        fontSize: t.codeSize,
        height: 1.4,
        fontWeight: weight,
        color: color ?? t.text,
      );

  TextStyle _mono(StormTokens t, {Color? color}) => TextStyle(
    fontFamily: StormTokens.monoFamily,
    fontSize: t.labelSize * 1.09,
    height: 1.45,
    color: color ?? t.text2,
  );

  Color _labelColor(StormTokens t) =>
      widget.tone == DecisionTone.waiting ? t.danger : t.text3;

  Widget _button(StormTokens t, DecisionAction a, {bool expand = false}) =>
      switch (a.kind) {
        DecisionActionKind.text => StormButton(
          key: a.key,
          label: a.label,
          onPressed: a.onPressed,
          kind: StormButtonKind.text,
          expand: expand,
        ),
        DecisionActionKind.ghost => StormButton.outline(
          key: a.key,
          label: a.label,
          onPressed: a.onPressed,
          expand: expand,
        ),
        DecisionActionKind.primary => StormButton.primary(
          key: a.key,
          label: a.label,
          onPressed: a.onPressed,
          expand: expand,
        ),
        DecisionActionKind.danger => StormButton.danger(
          key: a.key,
          label: a.label,
          onPressed: a.onPressed,
          expand: expand,
        ),
      };

  Widget _tick(StormTokens t, DecisionTick tick) => InkWell(
    key: const Key('report-first'),
    onTap: () => tick.onChanged(!tick.value),
    borderRadius: BorderRadius.circular(t.rControl),
    child: Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Checkbox(
          value: tick.value,
          onChanged: (v) => tick.onChanged(v ?? false),
          activeColor: t.accent,
          checkColor: t.onAccent,
          side: BorderSide(color: t.text3, width: t.bw),
          visualDensity: VisualDensity.compact,
        ),
        Flexible(
          child: Text(tick.label, style: _sans(t, color: t.text2)),
        ),
      ],
    ),
  );

  Widget _strip(StormTokens t) {
    final labelWidth = t.sp * 18.5;
    final gap = t.sp * 0.875;
    final footer = widget.tick != null;
    Widget keyed(Widget left, Widget right) => Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(width: labelWidth, child: left),
        Expanded(child: right),
      ],
    );
    final actions = Wrap(
      spacing: t.sp,
      runSpacing: t.sp * 0.5,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [for (final a in widget.actions) _button(t, a)],
    );
    return Container(
      key: const Key('decision-strip'),
      padding: EdgeInsets.fromLTRB(
        t.sp * 2.5,
        t.sp * 1.25,
        t.sp * 2.5,
        t.sp * 1.5,
      ),
      decoration: BoxDecoration(
        color: widget.tone == DecisionTone.waiting
            ? Color.alphaBlend(t.danger.withValues(alpha: 0.08), t.bg)
            : t.surface,
        border: Border(
          bottom: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              SizedBox(
                width: labelWidth,
                child: Text(
                  widget.label,
                  key: const Key('decision-label'),
                  style: _keyStyle(t, color: _labelColor(t)),
                ),
              ),
              Expanded(
                child: Text(
                  widget.title,
                  key: const Key('decision-title'),
                  style: _sans(t, weight: FontWeight.w500),
                ),
              ),
              if (!footer && widget.actions.isNotEmpty) ...[
                SizedBox(width: t.sp * 2),
                actions,
              ],
            ],
          ),
          for (final r in widget.rows)
            Padding(
              key: r.key,
              padding: EdgeInsets.only(top: gap),
              child: keyed(
                Text(r.label.toUpperCase(), style: _keyStyle(t)),
                Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(r.quote ? '“${r.value}”' : r.value, style: _sans(t)),
                    for (final m in r.more)
                      Text(m, style: _mono(t, color: t.text3)),
                  ],
                ),
              ),
            ),
          if (widget.note case final note?)
            Padding(
              padding: EdgeInsets.only(top: gap),
              child: keyed(
                const SizedBox.shrink(),
                Text(
                  note,
                  key: const Key('decision-note'),
                  style: _sans(t, color: t.text2),
                ),
              ),
            ),
          if (footer)
            Padding(
              padding: EdgeInsets.only(top: gap),
              child: Row(
                children: [
                  Expanded(
                    child: Align(
                      alignment: Alignment.centerLeft,
                      child: _tick(t, widget.tick!),
                    ),
                  ),
                  SizedBox(width: t.sp * 2),
                  actions,
                ],
              ),
            ),
        ],
      ),
    );
  }

  Widget _sheet(StormTokens t) {
    final gap = t.sp * 2;
    return Column(
      key: const Key('decision-sheet'),
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          widget.title,
          key: const Key('decision-title'),
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.bodySize,
            height: 1.4,
            fontWeight: FontWeight.w600,
            color: t.text,
          ),
        ),
        for (final r in widget.rows) ...[
          SizedBox(key: r.key, height: gap),
          Text(r.label.toUpperCase(), style: _keyStyle(t)),
          SizedBox(height: t.sp),
          if (r.quote)
            Container(
              padding: EdgeInsets.all(t.sp * 1.25),
              decoration: BoxDecoration(
                color: t.surface2,
                borderRadius: BorderRadius.circular(t.rControl),
              ),
              child: Text(r.value, style: _mono(t)),
            )
          else
            Text(r.value, style: _sans(t)),
          for (final m in r.more) Text(m, style: _mono(t, color: t.text3)),
        ],
        if (widget.note case final note?) ...[
          SizedBox(height: gap),
          Text(
            note,
            key: const Key('decision-note'),
            style: _sans(t, color: t.text2),
          ),
        ],
        if (widget.tick case final tick?) ...[
          SizedBox(height: t.sp),
          Align(alignment: Alignment.centerLeft, child: _tick(t, tick)),
        ],
        SizedBox(height: gap),
        // Thumb-sized, as the phone frame (r15) draws them.
        for (final (i, a) in widget.actions.indexed) ...[
          if (i > 0) SizedBox(height: t.sp),
          SizedBox(height: t.sp * 5.25, child: _button(t, a, expand: true)),
        ],
      ],
    );
  }
}

/// The phone's strip (153g, §7): one line under the header, `danger`
/// outlined and tinted, "NEEDS YOU", the title and Review.
class CompactDecisionStrip extends StatelessWidget {
  const CompactDecisionStrip({
    super.key,
    required this.label,
    required this.title,
    required this.onReview,
  });

  final String label;
  final String title;
  final VoidCallback onReview;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Semantics(
      container: true,
      liveRegion: true,
      child: Container(
        key: const Key('decision-compact'),
        // 12 from the edges, as r14 draws it.
        margin: EdgeInsets.fromLTRB(
          t.sp * 1.5,
          t.sp * 1.25,
          t.sp * 1.5,
          t.sp * 1.25,
        ),
        padding: EdgeInsets.fromLTRB(
          t.sp * 1.5,
          t.sp * 0.5,
          t.sp * 0.5,
          t.sp * 0.5,
        ),
        decoration: BoxDecoration(
          color: Color.alphaBlend(t.danger.withValues(alpha: 0.08), t.bg),
          borderRadius: BorderRadius.circular(t.rControl),
          border: Border.all(color: t.danger, width: t.bw),
        ),
        child: Row(
          children: [
            Text(
              label,
              style: TextStyle(
                fontFamily: StormTokens.monoFamily,
                fontSize: t.labelSize,
                letterSpacing: t.labelSize * 0.08,
                color: t.danger,
              ),
            ),
            SizedBox(width: t.sp * 1.25),
            Expanded(
              child: Text(
                title,
                key: const Key('decision-title'),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  color: t.text,
                ),
              ),
            ),
            // A link, not a button. In `text`, not the frame's `accent`:
            // `accent` on the tint misses 4.5:1 in dark and earth (153-8).
            Semantics(
              button: true,
              label: 'Review',
              excludeSemantics: true,
              onTap: onReview,
              child: InkWell(
                key: const Key('decision-review'),
                onTap: onReview,
                borderRadius: BorderRadius.circular(t.rControl),
                child: Padding(
                  padding: EdgeInsets.symmetric(
                    horizontal: t.sp,
                    vertical: t.sp * 0.75,
                  ),
                  child: Text(
                    'Review',
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.codeSize,
                      fontWeight: FontWeight.w500,
                      color: t.text,
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

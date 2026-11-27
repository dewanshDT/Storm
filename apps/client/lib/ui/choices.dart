/// Choice chips and choice cards.
library;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'tokens.dart';

class ChoiceOption<T> {
  const ChoiceOption(this.value, this.label, {this.leading, this.key});

  final T value;
  final String label;
  final Widget? leading;
  final Key? key;
}

/// Outline chips; the selected one is `accent` on `accentSoft`.
class ChoiceChips<T> extends StatelessWidget {
  const ChoiceChips({
    super.key,
    required this.options,
    required this.selected,
    required this.onSelected,
    this.dense = false,
  });

  final List<ChoiceOption<T>> options;
  final T? selected;
  final ValueChanged<T>? onSelected;

  /// A tighter padding and `labelSize` text — a filter row under a compact
  /// panel's own header (decision 161's redesign), rather than the
  /// settings-page scale these default to.
  final bool dense;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Wrap(
      spacing: t.sp,
      runSpacing: t.sp,
      children: [
        for (final o in options)
          _Chip(
            key: o.key,
            option: o,
            selected: o.value == selected,
            dense: dense,
            onTap: onSelected == null ? null : () => onSelected!(o.value),
          ),
      ],
    );
  }
}

class _Chip<T> extends StatelessWidget {
  const _Chip({
    super.key,
    required this.option,
    required this.selected,
    this.dense = false,
    this.onTap,
  });

  final ChoiceOption<T> option;
  final bool selected;
  final bool dense;
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
              horizontal: t.sp * (dense ? 0.875 : 1.5),
              vertical: t.sp * (dense ? 0.4 : 1),
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
                    fontSize: dense ? t.labelSize * 1.09 : t.codeSize,
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

class ChoiceCardOption<T> {
  const ChoiceCardOption(
    this.value,
    this.title, {
    this.subtitle,
    this.note,
    this.alert,
    this.preview,
    this.semanticLabel,
    this.enabled = true,
    this.key,
  });

  final T value;
  final String title;
  final String? subtitle;

  /// Muted, e.g. a credit or a limit.
  final String? note;

  /// In `danger`, under the note.
  final Widget? alert;

  /// Above the title, full width.
  final Widget? preview;
  final String? semanticLabel;
  final bool enabled;
  final Key? key;
}

/// Equal-width selectable cards filling the width, as many per row as fit at
/// 220; cards in a row share a height.
class ChoiceCards<T> extends StatelessWidget {
  const ChoiceCards({
    super.key,
    required this.options,
    required this.selected,
    required this.onSelected,
  });

  final List<ChoiceCardOption<T>> options;
  final T? selected;
  final ValueChanged<T>? onSelected;

  static const _minCardWidth = 220;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final gap = t.gap;
    return LayoutBuilder(
      builder: (context, box) {
        final cols = ((box.maxWidth + gap) / (_minCardWidth + gap))
            .floor()
            .clamp(1, options.isEmpty ? 1 : options.length);
        Widget card(ChoiceCardOption<T> o) => _ChoiceCard(
          key: o.key,
          title: o.title,
          subtitle: o.subtitle,
          note: o.note,
          alert: o.alert,
          preview: o.preview,
          semanticLabel: o.semanticLabel,
          selected: o.value == selected,
          onTap: onSelected == null || !o.enabled
              ? null
              : () => onSelected!(o.value),
        );
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (var i = 0; i < options.length; i += cols) ...[
              if (i > 0) SizedBox(height: gap),
              IntrinsicHeight(
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    for (var j = i; j < i + cols; j++) ...[
                      if (j > i) SizedBox(width: gap),
                      Expanded(
                        child: j < options.length
                            ? card(options[j])
                            : const SizedBox.shrink(),
                      ),
                    ],
                  ],
                ),
              ),
            ],
          ],
        );
      },
    );
  }
}

/// A selectable card: outline at rest, `surface` on hover, `surface2` while
/// pressed, `accent` on `accentSoft` when selected (as [ChoiceChips]), an
/// `accent` ring when focused from the keyboard, and dimmed when [onTap] is
/// null.
class _ChoiceCard extends StatefulWidget {
  const _ChoiceCard({
    super.key,
    required this.title,
    required this.selected,
    required this.onTap,
    this.subtitle,
    this.note,
    this.alert,
    this.preview,
    this.semanticLabel,
  });

  final String title;
  final String? subtitle;
  final String? note;
  final Widget? alert;
  final Widget? preview;
  final String? semanticLabel;
  final bool selected;
  final VoidCallback? onTap;

  @override
  State<_ChoiceCard> createState() => _ChoiceCardState();
}

class _ChoiceCardState extends State<_ChoiceCard> {
  bool _hover = false;
  bool _pressed = false;
  bool _focusRing = false;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final live = widget.onTap != null;
    final selected = widget.selected;
    final radius = BorderRadius.circular(t.rCard);
    final fill = selected
        ? t.accentSoft
        : !live
        ? Colors.transparent
        : _pressed
        ? t.surface2
        : _hover
        ? t.surface
        : Colors.transparent;
    final ring = t.bw * 2;
    Text line(String s, Color color, {FontWeight? weight, double? size}) =>
        Text(
          s,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          softWrap: false,
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: size ?? t.labelSize,
            fontWeight: weight,
            color: color,
          ),
        );

    final body = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        if (widget.preview != null) ...[
          widget.preview!,
          SizedBox(height: t.sp * 1.25),
        ],
        line(
          widget.title,
          selected ? t.accent : t.text,
          weight: FontWeight.w500,
          size: t.codeSize,
        ),
        if (widget.subtitle != null) ...[
          SizedBox(height: t.sp * 0.25),
          line(widget.subtitle!, t.text2),
        ],
        if (widget.note != null) ...[
          SizedBox(height: t.sp * 0.25),
          line(widget.note!, t.text3),
        ],
        if (widget.alert != null) ...[
          SizedBox(height: t.sp * 0.25),
          widget.alert!,
        ],
      ],
    );

    return Semantics(
      container: true,
      button: true,
      selected: selected,
      enabled: live,
      label:
          widget.semanticLabel ??
          [widget.title, ?widget.subtitle, ?widget.note].join(', '),
      onTap: widget.onTap,
      excludeSemantics: true,
      child: Opacity(
        opacity: live ? 1 : 0.5,
        child: FocusableActionDetector(
          enabled: live,
          mouseCursor: live ? SystemMouseCursors.click : MouseCursor.defer,
          onShowFocusHighlight: (v) => setState(() => _focusRing = v),
          shortcuts: const {
            SingleActivator(LogicalKeyboardKey.enter): ActivateIntent(),
            SingleActivator(LogicalKeyboardKey.space): ActivateIntent(),
          },
          actions: {
            ActivateIntent: CallbackAction<ActivateIntent>(
              onInvoke: (_) => widget.onTap?.call(),
            ),
          },
          child: MouseRegion(
            onEnter: (_) => setState(() => _hover = true),
            onExit: (_) => setState(() => _hover = false),
            child: GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTap: widget.onTap,
              onTapDown: live ? (_) => setState(() => _pressed = true) : null,
              onTapUp: live ? (_) => setState(() => _pressed = false) : null,
              onTapCancel: live ? () => setState(() => _pressed = false) : null,
              child: Stack(
                clipBehavior: Clip.none,
                children: [
                  AnimatedContainer(
                    duration: t.durationOf(context),
                    curve: t.curve,
                    padding: EdgeInsets.all(t.sp * 1.5),
                    decoration: BoxDecoration(
                      color: fill,
                      borderRadius: radius,
                      border: Border.all(
                        color: selected ? t.accent : t.border,
                        width: t.bw,
                      ),
                    ),
                    child: body,
                  ),
                  if (_focusRing)
                    Positioned.fill(
                      left: -ring * 1.5,
                      top: -ring * 1.5,
                      right: -ring * 1.5,
                      bottom: -ring * 1.5,
                      child: IgnorePointer(
                        child: DecoratedBox(
                          key: const Key('choice-card-focus-ring'),
                          decoration: BoxDecoration(
                            borderRadius: BorderRadius.circular(
                              t.rCard + ring * 1.5,
                            ),
                            border: Border.all(color: t.accent, width: ring),
                          ),
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

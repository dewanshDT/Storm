/// The one tappable wrapper: focus, hover, press, Enter/Space, Semantics and
/// a minimum hit area that never changes the visual size.
library;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import 'tokens.dart';

/// Decision R8: 40 on desktop, 48 on touch.
double minHitSize(BuildContext context) => switch (Theme.of(context).platform) {
  TargetPlatform.android || TargetPlatform.iOS || TargetPlatform.fuchsia => 48,
  _ => 40,
};

class Tappable extends StatefulWidget {
  const Tappable({
    super.key,
    required this.child,
    required this.onTap,
    this.onSecondaryTap,
    this.onLongPress,
    this.semanticLabel,
    this.tooltip,
    this.borderRadius,
    this.minHitSize,
    this.focusRing = true,
  });

  final Widget child;

  /// Null disables the control: no focus, no hover, no activation.
  final VoidCallback? onTap;
  final VoidCallback? onSecondaryTap;
  final VoidCallback? onLongPress;
  final String? semanticLabel;
  final String? tooltip;
  final BorderRadius? borderRadius;

  /// Overrides the platform minimum; 0 turns the padding off.
  final double? minHitSize;
  final bool focusRing;

  @override
  State<Tappable> createState() => _TappableState();
}

class _TappableState extends State<Tappable> {
  bool _focusRing = false;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final live = widget.onTap != null;
    final ring = t.bw * 2;
    final radius = widget.borderRadius;

    Widget out = FocusableActionDetector(
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
      child: Stack(
        fit: StackFit.passthrough,
        clipBehavior: Clip.none,
        children: [
          InkWell(
            canRequestFocus: false,
            onTap: widget.onTap,
            onSecondaryTap: live ? widget.onSecondaryTap : null,
            onLongPress: live ? widget.onLongPress : null,
            borderRadius: radius,
            child: widget.child,
          ),
          if (_focusRing && widget.focusRing)
            Positioned.fill(
              left: -ring * 1.5,
              top: -ring * 1.5,
              right: -ring * 1.5,
              bottom: -ring * 1.5,
              child: IgnorePointer(
                child: DecoratedBox(
                  key: const Key('tappable-focus-ring'),
                  decoration: BoxDecoration(
                    borderRadius: BorderRadius.circular(
                      (radius?.topLeft.x ?? 0) + ring * 1.5,
                    ),
                    border: Border.all(color: t.accent, width: ring),
                  ),
                ),
              ),
            ),
        ],
      ),
    );

    out = Semantics(
      button: true,
      enabled: live,
      label: widget.semanticLabel,
      onTap: widget.onTap,
      excludeSemantics: widget.semanticLabel != null,
      container: widget.semanticLabel != null,
      child: out,
    );
    if (widget.tooltip != null) {
      out = Tooltip(message: widget.tooltip, child: out);
    }
    final min = widget.minHitSize ?? minHitSize(context);
    return min > 0 ? _HitSlop(min: min, child: out) : out;
  }
}

class _HitSlop extends SingleChildRenderObjectWidget {
  const _HitSlop({required this.min, required super.child});

  final double min;

  @override
  RenderObject createRenderObject(BuildContext context) => _RenderHitSlop(min);

  @override
  void updateRenderObject(BuildContext context, _RenderHitSlop renderObject) =>
      renderObject.min = min;
}

class _RenderHitSlop extends RenderProxyBox {
  _RenderHitSlop(this._min);

  double _min;
  set min(double v) {
    if (v == _min) return;
    _min = v;
    markNeedsLayout();
  }

  @override
  bool hitTest(BoxHitTestResult result, {required Offset position}) {
    final c = child;
    if (c == null) return false;
    final dx = (_min - size.width).clamp(0, double.infinity) / 2;
    final dy = (_min - size.height).clamp(0, double.infinity) / 2;
    final slop = Rect.fromLTRB(-dx, -dy, size.width + dx, size.height + dy);
    if (!slop.contains(position)) return false;
    final inside = size.contains(position);
    final at = inside ? position : size.center(Offset.zero);
    return result.addWithRawTransform(
      transform: inside
          ? null
          : MatrixUtils.forceToPoint(size.center(Offset.zero)),
      position: at,
      hitTest: (r, p) => c.hitTest(r, position: p),
    );
  }
}

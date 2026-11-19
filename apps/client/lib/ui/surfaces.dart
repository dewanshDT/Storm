/// Sheets and popovers, drawn once so Properties, Mentions, Tags and the two
/// corner menus stop each hand-rolling the same container.
library;

import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';

import 'breakpoints.dart';
import 'widgets.dart';
import 'tokens.dart';

/// A bottom sheet: `surface`, `rCard` top corners, a labelled header with a
/// close affordance.
class StormSheet extends StatelessWidget {
  const StormSheet({
    super.key,
    required this.title,
    required this.child,
    this.onClose,
    this.scrollable = true,
  });

  final String title;
  final Widget child;
  final VoidCallback? onClose;
  final bool scrollable;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final body = Padding(
      padding: EdgeInsets.fromLTRB(t.cardPad, 0, t.cardPad, t.cardPad),
      child: child,
    );

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: EdgeInsets.fromLTRB(
            t.cardPad,
            t.sp * 2,
            t.sp * 1.5,
            t.sp * 1.5,
          ),
          child: Row(
            children: [
              Expanded(child: SectionLabel(title)),
              if (onClose != null)
                IconButton(
                  icon: Icon(LucideIcons.x, size: t.bodySize, color: t.text3),
                  onPressed: onClose,
                  visualDensity: VisualDensity.compact,
                  tooltip: 'Close',
                ),
            ],
          ),
        ),
        if (scrollable)
          Flexible(child: SingleChildScrollView(child: body))
        else
          Flexible(child: body),
      ],
    );
  }
}

/// Raises [builder]'s widget as a [StormSheet]-shaped modal.
Future<T?> showStormSheet<T>({
  required BuildContext context,
  required String title,
  required WidgetBuilder builder,
  double heightFactor = 0.6,
}) {
  final t = context.tokens;
  return showModalBottomSheet<T>(
    context: context,
    isScrollControlled: true,
    backgroundColor: t.surface,
    shape: RoundedRectangleBorder(
      borderRadius: BorderRadius.vertical(top: Radius.circular(t.rCard)),
      side: BorderSide(color: t.border, width: t.bw),
    ),
    builder: (sheetContext) => SafeArea(
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxHeight: MediaQuery.sizeOf(sheetContext).height * heightFactor,
        ),
        child: StormSheet(
          title: title,
          onClose: () => Navigator.of(sheetContext).pop(),
          child: builder(sheetContext),
        ),
      ),
    ),
  );
}

/// Raises [builder]'s content as a dialog at desk width, a [StormSheet]
/// bottom sheet on a phone — the split "Report a problem" (decision 162a)
/// wanted and no sheet here had yet. A later sheet that needs the same split
/// should reach for this rather than hand-rolling the `context.isExpanded`
/// branch again.
Future<T?> showAdaptiveSheet<T>({
  required BuildContext context,
  required String title,
  required WidgetBuilder builder,
  double heightFactor = 0.6,
  double dialogMaxWidth = 480,
}) {
  if (!context.isExpanded) {
    return showStormSheet<T>(
      context: context,
      title: title,
      heightFactor: heightFactor,
      builder: builder,
    );
  }
  final t = context.tokens;
  return showDialog<T>(
    context: context,
    builder: (dialogContext) => Dialog(
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(t.rCard),
      ),
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxWidth: dialogMaxWidth,
          maxHeight: MediaQuery.sizeOf(dialogContext).height * 0.85,
        ),
        child: StormSheet(
          title: title,
          onClose: () => Navigator.of(dialogContext).pop(),
          child: builder(dialogContext),
        ),
      ),
    ),
  );
}

/// The anchored menu the corner bubbles drop.
class StormPopover extends StatelessWidget {
  const StormPopover({
    super.key,
    required this.children,
    this.width,
    this.padding,
  });

  final List<Widget> children;
  final double? width;
  final double? padding;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      width: width,
      padding: EdgeInsets.all(padding ?? t.sp * 1.5),
      decoration: BoxDecoration(
        color: t.surface2,
        borderRadius: BorderRadius.circular(t.rCard),
        border: Border.all(color: t.border, width: t.bw),
        boxShadow: t.shadow,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: children,
      ),
    );
  }
}

/// A hairline between popover groups.
class PopoverDivider extends StatelessWidget {
  const PopoverDivider({super.key});

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.symmetric(vertical: t.sp * 0.75),
      child: Container(height: t.bw, color: t.border),
    );
  }
}

enum PopoverSide { below, right }

typedef PopoverPosition = ({
  double? top,
  double? bottom,
  double? left,
  double? right,
});

/// Where a popover goes for [anchor] inside [overlay]. Below the anchor (or
/// beside it, for the rail), flipped to grow upward when the anchor sits low:
/// a menu dropped off the bottom edge reads as a button that does nothing.
PopoverPosition placePopover({
  required Rect anchor,
  required Size overlay,
  required double gap,
  PopoverSide side = PopoverSide.below,
  bool alignRight = false,
}) {
  switch (side) {
    case PopoverSide.below:
      final below = anchor.bottom + gap;
      final flip = overlay.height - below < overlay.height * 0.3;
      return (
        top: flip ? null : below,
        bottom: flip ? overlay.height - anchor.top + gap : null,
        left: alignRight ? null : anchor.left,
        right: alignRight ? overlay.width - anchor.right : null,
      );
    case PopoverSide.right:
      final flip = overlay.height - anchor.top < overlay.height * 0.3;
      return (
        top: flip ? null : anchor.top,
        bottom: flip ? overlay.height - anchor.bottom : null,
        left: anchor.right + gap,
        right: null,
      );
  }
}

/// How wide [label] sets as a [PopoverItem]'s line.
double popoverLabelWidth(BuildContext context, String label) {
  final style = (Theme.of(context).textTheme.bodyMedium ?? const TextStyle())
      .merge(popoverItemStyle(context.tokens));
  final painter = TextPainter(
    text: TextSpan(text: label, style: style),
    textDirection: TextDirection.ltr,
    textScaler: MediaQuery.textScalerOf(context),
    maxLines: 1,
  )..layout();
  final width = painter.width;
  painter.dispose();
  return width;
}

/// What a [StormPopover] at its default padding and a [PopoverItem] take
/// around a label.
double popoverChrome(StormTokens t) => (t.sp * 1.5 + t.sp * 0.75 + t.bw) * 2;

/// A [StormPopover] of [PopoverItem]s as wide as its widest of [labels],
/// within [min]..[max]: a menu cut to its anchor's width elides short names.
double fitPopoverWidth(
  BuildContext context,
  Iterable<String> labels, {
  double min = 180,
  double max = 320,
}) {
  var widest = 0.0;
  for (final label in labels) {
    widest = math.max(widest, popoverLabelWidth(context, label));
  }
  return (widest.ceilToDouble() + popoverChrome(context.tokens) + 1).clamp(
    min,
    max,
  );
}

/// Opens a [StormPopover] anchored to [anchorKey]'s widget.
///
/// Uses a transparent full-screen route rather than `showMenu`, which brings
/// Material's own menu geometry and cannot be given the popover's shape.
Future<T?> showStormPopover<T>({
  required BuildContext context,
  required GlobalKey anchorKey,
  required double width,
  required Widget Function(BuildContext context) builder,
  bool alignRight = false,
  PopoverSide side = PopoverSide.below,
}) {
  final t = context.tokens;
  final box = anchorKey.currentContext?.findRenderObject() as RenderBox?;
  final overlay =
      Navigator.of(context).overlay!.context.findRenderObject() as RenderBox;
  if (box == null) return Future<T?>.value();

  final origin = box.localToGlobal(Offset.zero, ancestor: overlay);
  final (:top, :bottom, left: placed, :right) = placePopover(
    anchor: origin & box.size,
    overlay: overlay.size,
    gap: t.sp,
    side: side,
    alignRight: alignRight,
  );
  // An anchor near the right edge (a page header's ··· at 900) would push
  // the popover off the window, where it can't be seen or tapped.
  final left = placed == null
      ? null
      : math.max(t.sp, math.min(placed, overlay.size.width - width - t.sp));

  return Navigator.of(context).push<T>(
    PageRouteBuilder<T>(
      opaque: false,
      barrierColor: const Color(0x00000000),
      barrierDismissible: true,
      barrierLabel: 'Dismiss',
      transitionDuration: t.duration,
      pageBuilder: (routeContext, animation, _) => Stack(
        children: [
          Positioned(
            top: top,
            bottom: bottom,
            left: left,
            right: right,
            width: width,
            child: FadeTransition(
              opacity: animation,
              child: Material(
                color: const Color(0x00000000),
                child: builder(routeContext),
              ),
            ),
          ),
        ],
      ),
    ),
  );
}

/// A route whose content is a sheet raised over [backdrop].
///
/// Tags and Mentions are sheets in the design but routes in the app — `/tags`
/// has to keep resolving, and the deep-link test covers it. This renders the
/// screen they sit over, then raises the sheet on the first frame; dismissing
/// it runs [onDismiss] rather than leaving an empty route behind.
class SheetHost extends StatefulWidget {
  const SheetHost({
    super.key,
    required this.title,
    required this.backdrop,
    required this.builder,
    required this.onDismiss,
    this.heightFactor = 0.6,
  });

  final String title;
  final Widget backdrop;
  final WidgetBuilder builder;
  final VoidCallback onDismiss;
  final double heightFactor;

  @override
  State<SheetHost> createState() => _SheetHostState();
}

class _SheetHostState extends State<SheetHost> {
  bool _raised = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _raise());
  }

  Future<void> _raise() async {
    if (_raised || !mounted) return;
    _raised = true;
    await showStormSheet<void>(
      context: context,
      title: widget.title,
      heightFactor: widget.heightFactor,
      builder: widget.builder,
    );
    // Only leave if this route is still the one on screen: the sheet also
    // closes when something inside it navigates, and popping then would undo
    // the navigation it just did.
    if (!mounted) return;
    if (ModalRoute.of(context)?.isCurrent ?? false) widget.onDismiss();
  }

  @override
  Widget build(BuildContext context) => widget.backdrop;
}

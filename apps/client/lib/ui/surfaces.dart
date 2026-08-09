/// Sheets and popovers, drawn once so Properties, Mentions, Tags and the two
/// corner menus stop each hand-rolling the same container.
library;

import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';

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

/// The anchored menu the corner bubbles drop.
class StormPopover extends StatelessWidget {
  const StormPopover({super.key, required this.children, this.width});

  final List<Widget> children;
  final double? width;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      width: width,
      padding: EdgeInsets.all(t.sp * 1.5),
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

/// Opens a [StormPopover] anchored under [anchorKey]'s widget.
///
/// Uses a transparent full-screen route rather than `showMenu`, which brings
/// Material's own menu geometry and cannot be given the popover's shape.
Future<T?> showStormPopover<T>({
  required BuildContext context,
  required GlobalKey anchorKey,
  required double width,
  required Widget Function(BuildContext context) builder,
  bool alignRight = false,
}) {
  final t = context.tokens;
  final box = anchorKey.currentContext?.findRenderObject() as RenderBox?;
  final overlay =
      Navigator.of(context).overlay!.context.findRenderObject() as RenderBox;
  if (box == null) return Future<T?>.value();

  final origin = box.localToGlobal(Offset.zero, ancestor: overlay);
  // Below the anchor, unless there is no room below it — an anchor near the
  // foot of the window would otherwise drop its menu off the bottom edge,
  // where it is indistinguishable from a button that does nothing.
  final below = origin.dy + box.size.height + t.sp;
  final roomBelow = overlay.size.height - below;
  final flip = roomBelow < overlay.size.height * 0.3;
  final top = flip ? null : below;
  final bottom = flip ? overlay.size.height - origin.dy + t.sp : null;
  final left = alignRight ? null : origin.dx;
  final right = alignRight
      ? overlay.size.width - (origin.dx + box.size.width)
      : null;

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

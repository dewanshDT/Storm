import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';

import '../ui/tokens.dart';

/// Whether a session's Details inspector is open at desk width; `null` until
/// toggled, then the window decides, as for the Properties drawer. Not
/// persisted, like the Properties drawer and the sidebar.
final sessionInspectorOpenProvider = StateProvider<bool?>((ref) => null);

/// The inspector's width once dragged; `null` keeps the default.
final sessionInspectorWidthProvider = StateProvider<double?>((ref) => null);

const kInspectorWidth = 320.0;
const kInspectorMinWidth = 260.0;
const kInspectorMaxWidth = 500.0;

/// What the terminal keeps, however wide the inspector is dragged.
const kSessionMinWidth = 360.0;

/// The width the inspector gets inside [available], the session workspace.
double inspectorWidth(double? wanted, double available) {
  final max = (available - kSessionMinWidth).clamp(
    kInspectorMinWidth,
    kInspectorMaxWidth,
  );
  return (wanted ?? kInspectorWidth).clamp(kInspectorMinWidth, max);
}

/// The session's Details column: a hairline and a draggable left edge, then
/// [child] on `bg`, beside the terminal.
class SessionInspector extends ConsumerWidget {
  const SessionInspector({
    super.key,
    required this.available,
    required this.child,
  });

  final double available;
  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final width = inspectorWidth(
      ref.watch(sessionInspectorWidthProvider),
      available,
    );
    void resize(double next) =>
        ref.read(sessionInspectorWidthProvider.notifier).state = inspectorWidth(
          next,
          available,
        );
    return SizedBox(
      key: const Key('session-inspector'),
      width: width + t.bw,
      child: Stack(
        children: [
          Positioned.fill(
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Container(width: t.bw, color: t.border),
                Expanded(child: child),
              ],
            ),
          ),
          Positioned(
            left: -t.sp * 0.5,
            top: 0,
            bottom: 0,
            width: t.sp * 1.25,
            child: Semantics(
              label: 'Resize details',
              slider: true,
              value: '${width.round()}',
              increasedValue:
                  '${inspectorWidth(width + 20, available).round()}',
              decreasedValue:
                  '${inspectorWidth(width - 20, available).round()}',
              onIncrease: () => resize(width + 20),
              onDecrease: () => resize(width - 20),
              child: MouseRegion(
                cursor: SystemMouseCursors.resizeColumn,
                child: GestureDetector(
                  key: const Key('inspector-resize'),
                  behavior: HitTestBehavior.translucent,
                  onHorizontalDragUpdate: (d) => resize(width - d.delta.dx),
                  onDoubleTap: () => resize(kInspectorWidth),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

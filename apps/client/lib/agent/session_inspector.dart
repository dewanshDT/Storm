import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';

import '../ui/tokens.dart';
import 'agent_state.dart' show SessionTab;

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

/// The icon rail on the inspector's left edge (153i).
const kInspectorRailWidth = 48.0;

/// One of the rail's icons: its tab, name (the tooltip), icon and count.
typedef InspectorRailItem = ({
  SessionTab tab,
  String name,
  IconData icon,
  ({int count, bool loud})? badge,
});

/// The session's inspector beside the terminal (153i): a hairline, the icon
/// rail, and while [open] the [panel] at its own width, resizable from its
/// left edge. Closed, the rail stays, so the counts stay in sight.
class SessionInspector extends ConsumerWidget {
  const SessionInspector({
    super.key,
    required this.available,
    required this.open,
    required this.items,
    required this.selected,
    required this.onPick,
    required this.panel,
  });

  final double available;
  final bool open;
  final List<InspectorRailItem> items;
  final SessionTab selected;
  final ValueChanged<SessionTab> onPick;
  final Widget panel;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final room = available - kInspectorRailWidth - t.bw;
    final width = inspectorWidth(
      ref.watch(sessionInspectorWidthProvider),
      room,
    );
    void resize(double next) =>
        ref.read(sessionInspectorWidthProvider.notifier).state = inspectorWidth(
          next,
          room,
        );
    final rail = _Rail(
      items: items,
      selected: open ? selected : null,
      onPick: onPick,
    );
    return SizedBox(
      key: const Key('session-inspector'),
      width: kInspectorRailWidth + t.bw + (open ? width + t.bw : 0),
      child: Stack(
        children: [
          Positioned.fill(
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Container(width: t.bw, color: t.border),
                rail,
                if (open) ...[
                  Container(width: t.bw, color: t.border),
                  SizedBox(
                    key: const Key('inspector-panel'),
                    width: width,
                    child: panel,
                  ),
                ],
              ],
            ),
          ),
          if (open)
            Positioned(
              left: -t.sp * 0.5,
              top: 0,
              bottom: 0,
              width: t.sp * 1.25,
              child: Semantics(
                label: 'Resize details',
                slider: true,
                value: '${width.round()}',
                increasedValue: '${inspectorWidth(width + 20, room).round()}',
                decreasedValue: '${inspectorWidth(width - 20, room).round()}',
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

class _Rail extends StatelessWidget {
  const _Rail({
    required this.items,
    required this.selected,
    required this.onPick,
  });

  final List<InspectorRailItem> items;
  final SessionTab? selected;
  final ValueChanged<SessionTab> onPick;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      key: const Key('inspector-rail'),
      width: kInspectorRailWidth,
      color: t.bg,
      padding: EdgeInsets.only(top: t.sp * 1.5),
      child: Column(
        children: [
          for (final item in items) ...[
            _RailButton(
              item: item,
              active: item.tab == selected,
              onTap: () => onPick(item.tab),
            ),
            SizedBox(height: t.sp * 0.75),
          ],
        ],
      ),
    );
  }
}

class _RailButton extends StatelessWidget {
  const _RailButton({
    required this.item,
    required this.active,
    required this.onTap,
  });

  final InspectorRailItem item;
  final bool active;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final badge = item.badge;
    const size = 36.0;
    const dot = 15.0;
    return Tooltip(
      message: item.name,
      child: Semantics(
        button: true,
        selected: active,
        label: badge == null ? item.name : '${item.name}, ${badge.count}',
        excludeSemantics: true,
        onTap: onTap,
        child: SizedBox(
          width: size,
          height: size,
          child: Stack(
            clipBehavior: Clip.none,
            children: [
              Positioned.fill(
                child: Material(
                  color: active ? t.accentSoft : Colors.transparent,
                  borderRadius: BorderRadius.circular(t.rControl),
                  child: InkWell(
                    key: Key('inspector-rail-${item.tab.name}'),
                    borderRadius: BorderRadius.circular(t.rControl),
                    onTap: onTap,
                    child: Icon(
                      item.icon,
                      size: t.bodySize * 1.0625,
                      color: active ? t.accent : t.text3,
                    ),
                  ),
                ),
              ),
              if (badge != null)
                Positioned(
                  top: -dot * 0.25,
                  right: -dot * 0.25,
                  child: IgnorePointer(
                    child: Container(
                      key: Key('inspector-badge-${item.tab.name}'),
                      constraints: const BoxConstraints(
                        minWidth: dot,
                        minHeight: dot,
                      ),
                      padding: EdgeInsets.symmetric(horizontal: t.sp * 0.375),
                      alignment: Alignment.center,
                      decoration: BoxDecoration(
                        color: badge.loud ? t.danger : t.surface2,
                        borderRadius: BorderRadius.circular(999),
                      ),
                      child: Text(
                        '${badge.count}',
                        style: TextStyle(
                          fontFamily: StormTokens.monoFamily,
                          fontSize: t.labelSize * 0.82,
                          height: 1,
                          fontWeight: FontWeight.w600,
                          color: badge.loud ? t.onAccent : t.text2,
                        ),
                      ),
                    ),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

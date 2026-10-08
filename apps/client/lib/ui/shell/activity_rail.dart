import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../agent/agent_state.dart';
import '../../state/health.dart';
import '../../state/nav_memory.dart';
import '../surfaces.dart';
import '../tokens.dart';

/// The desktop activity rail (handoff §2.1): Notes, Agents, then the health
/// dot and Settings at the foot.
class ActivityRail extends ConsumerWidget {
  const ActivityRail({super.key, required this.current});

  final Activity? current;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final memory = ref.watch(navMemoryProvider);
    final live = ref.watch(agentOverviewProvider).value?.live.length ?? 0;

    void open(Activity a) => context.go(memory.entryOf(a));

    return Container(
      width: t.sp * 7,
      decoration: BoxDecoration(
        color: t.bg,
        border: Border(
          right: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      padding: EdgeInsets.only(top: t.sp * 2, bottom: t.sp),
      child: SafeArea(
        right: false,
        child: Column(
          spacing: t.sp,
          children: [
            RailItem(
              key: const Key('rail-notes'),
              icon: LucideIcons.file_text,
              label: 'Notes',
              active: current == Activity.notes,
              onTap: () => open(Activity.notes),
            ),
            RailItem(
              key: const Key('rail-agents'),
              icon: LucideIcons.square_terminal,
              label: 'Agents',
              active: current == Activity.agents,
              badge: live,
              onTap: () => open(Activity.agents),
            ),
            const Spacer(),
            const RailStatusDot(),
            RailItem(
              key: const Key('rail-settings'),
              icon: LucideIcons.settings_2,
              label: 'Settings',
              showLabel: false,
              active: current == Activity.settings,
              onTap: () => open(Activity.settings),
            ),
          ],
        ),
      ),
    );
  }
}

class RailItem extends StatelessWidget {
  const RailItem({
    super.key,
    required this.icon,
    required this.label,
    required this.active,
    required this.onTap,
    this.badge = 0,
    this.showLabel = true,
  });

  final IconData icon;
  final String label;
  final bool active;
  final VoidCallback onTap;
  final int badge;
  final bool showLabel;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final ink = active ? t.accent : t.text3;
    final radius = BorderRadius.circular(t.rControl);
    return Tooltip(
      message: showLabel ? '' : label,
      child: Semantics(
        button: true,
        selected: active,
        label: label,
        onTap: onTap,
        excludeSemantics: true,
        child: Material(
          color: active ? t.accentSoft : Colors.transparent,
          borderRadius: radius,
          child: InkWell(
            onTap: onTap,
            borderRadius: radius,
            child: showLabel ? _labelled(t, ink) : _square(t, ink),
          ),
        ),
      ),
    );
  }

  /// The rail's foot controls share one square, centred on the rail's axis.
  Widget _square(StormTokens t, Color ink) => SizedBox.square(
    dimension: railFootSize(t),
    child: Center(
      child: Icon(icon, size: t.sp * 2.5, color: ink),
    ),
  );

  Widget _labelled(StormTokens t, Color ink) => SizedBox(
    width: t.sp * 5.75,
    child: Stack(
      clipBehavior: Clip.none,
      children: [
        Padding(
          padding: EdgeInsets.symmetric(
            vertical: t.sp * 0.875,
            horizontal: t.sp * 0.625,
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(icon, size: t.sp * 2.5, color: ink),
              SizedBox(height: t.sp * 0.375),
              Text(
                label,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.labelSize,
                  color: ink,
                ),
              ),
            ],
          ),
        ),
        if (badge > 0)
          Positioned(
            top: t.sp * 0.375,
            right: t.sp * 0.75,
            child: Container(
              key: const Key('rail-badge'),
              constraints: BoxConstraints(
                minWidth: t.sp * 1.875,
                minHeight: t.sp * 1.875,
              ),
              padding: EdgeInsets.symmetric(horizontal: t.sp * 0.5),
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: t.accent,
                borderRadius: BorderRadius.circular(999),
              ),
              child: Text(
                '$badge',
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  fontSize: t.labelSize,
                  fontWeight: FontWeight.w500,
                  color: t.onAccent,
                  height: 1.2,
                ),
              ),
            ),
          ),
      ],
    ),
  );
}

double railFootSize(StormTokens t) => t.sp * 5;

Color healthColor(HealthTone tone, StormTokens t) => switch (tone) {
  HealthTone.good => t.green,
  HealthTone.muted => t.text3,
  HealthTone.danger => t.danger,
};

class RailStatusDot extends ConsumerStatefulWidget {
  const RailStatusDot({super.key});

  @override
  ConsumerState<RailStatusDot> createState() => _RailStatusDotState();
}

class _RailStatusDotState extends ConsumerState<RailStatusDot> {
  final _anchor = GlobalKey();

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final tone = ref.watch(healthToneProvider);
    return Semantics(
      button: true,
      label: 'Status',
      onTap: _open,
      excludeSemantics: true,
      child: InkWell(
        key: const Key('rail-status'),
        borderRadius: BorderRadius.circular(t.rControl),
        onTap: _open,
        child: SizedBox.square(
          key: _anchor,
          dimension: railFootSize(t),
          child: Center(
            child: Container(
              width: t.sp * 1.125,
              height: t.sp * 1.125,
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: healthColor(tone, t),
              ),
            ),
          ),
        ),
      ),
    );
  }

  Future<void> _open() async {
    final t = context.tokens;
    await showStormPopover<void>(
      context: context,
      anchorKey: _anchor,
      width: t.sp * 35,
      side: PopoverSide.right,
      builder: (popContext) => HealthPopover(
        onAbout: () {
          Navigator.pop(popContext);
          context.go('/settings/health');
        },
      ),
    );
  }
}

class HealthPopover extends StatelessWidget {
  const HealthPopover({super.key, required this.onAbout});

  final VoidCallback onAbout;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return StormPopover(
      children: [
        const HealthRows(),
        InkWell(
          key: const Key('health-about'),
          onTap: onAbout,
          borderRadius: BorderRadius.circular(t.rControl),
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 0.75,
              vertical: t.sp,
            ),
            child: Text(
              'About & health ›',
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                color: t.accent,
              ),
            ),
          ),
        ),
      ],
    );
  }
}

class HealthRows extends ConsumerWidget {
  const HealthRows({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final row in ref.watch(healthRowsProvider))
          Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 0.75,
              vertical: t.sp * 0.75,
            ),
            child: Row(
              children: [
                Container(
                  width: t.sp,
                  height: t.sp,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    color: healthColor(row.tone, t),
                  ),
                ),
                SizedBox(width: t.sp * 1.5),
                Expanded(
                  child: Text(
                    row.text,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.codeSize,
                      color: t.text,
                    ),
                  ),
                ),
              ],
            ),
          ),
      ],
    );
  }
}

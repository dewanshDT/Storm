import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import '../ui/states.dart';
import '../ui/tokens.dart';
import '../ui/widgets.dart';
import 'agent_state.dart';
import 'agent_widgets.dart';
import 'agents_screen.dart';

/// The Agents band on the dashboard (decision 78).
///
/// The dashboard is home for both spaces on a phone, so an owner's running
/// agents are one tap from opening the app. **It always sits in the same
/// place** — directly above *Recently opened* — whether or not anything is
/// running: a home screen that reshuffles itself when an agent finishes
/// breaks the muscle memory it exists to serve. Idle, it shrinks to a single
/// row that says what the next step actually is.
///
/// Nothing at all for an account that is not an owner, gap included, so
/// their dashboard is exactly what it was (AC-S1).
class AgentsBand extends ConsumerWidget {
  const AgentsBand({super.key});

  /// Live sessions shown before "All".
  static const limit = 3;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (!(ref.watch(agentAccessProvider).value ?? false)) {
      return const SizedBox.shrink();
    }
    final t = context.tokens;
    final o = ref.watch(agentOverviewProvider).value;

    void openSpace() => context.push(Routes.agents);
    void openHosts() => context.push(Routes.agentHosts);

    final Widget body;
    if (o == null) {
      body = const SkeletonRows(rows: 1);
    } else if (o.unreachable) {
      body = _BandLine(
        key: const Key('agents-band-offline'),
        icon: LucideIcons.cloud_off,
        title: 'Agents need the server',
        detail: "Nothing to show until it's back.",
        onTap: () => ref.invalidate(agentOverviewProvider),
      );
    } else if (o.live.isNotEmpty) {
      body = Column(
        children: [
          for (final s in o.live.take(limit))
            AgentSessionRow(
              key: Key('band-${s.id}'),
              session: s,
              hostName: o.hostName(s.hostId),
              onTap: () {
                openAgentSession(ref, s.id);
                openSpace();
              },
            ),
        ],
      );
    } else if (o.hosts.isEmpty) {
      body = _BandLine(
        key: const Key('agents-band-setup'),
        icon: LucideIcons.server,
        title: 'Set up a host',
        detail: 'Agents run on a machine you enroll.',
        onTap: openHosts,
      );
    } else if (o.online.isEmpty) {
      body = _BandLine(
        key: const Key('agents-band-host-offline'),
        icon: LucideIcons.server_off,
        title: o.hosts.length == 1
            ? '${o.hosts.single.name} is offline'
            : 'Your hosts are offline',
        detail: 'Start storm-runtime to run agents.',
        onTap: openHosts,
      );
    } else {
      body = _BandLine(
        key: const Key('agents-band-idle'),
        icon: LucideIcons.square_terminal,
        title: 'No agents running',
        action: 'New session',
        onTap: () async {
          final launched = await launchAgentSession(context, ref);
          if (launched != null && context.mounted) openSpace();
        },
      );
    }

    // "All" is there whenever there is a list to go to — including the idle
    // case, where it is the way to yesterday's ended sessions.
    final showAll = o != null && !o.unreachable && o.hosts.isNotEmpty;
    final more = (o?.live.length ?? 0) - limit;

    return Padding(
      padding: EdgeInsets.only(bottom: t.sectionRhythm * 0.5),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              const SectionLabel('Agents'),
              const Spacer(),
              if (showAll)
                InkWell(
                  key: const Key('agents-all'),
                  borderRadius: BorderRadius.circular(t.rControl),
                  onTap: openSpace,
                  child: Padding(
                    padding: EdgeInsets.symmetric(
                      horizontal: t.sp * 0.5,
                      vertical: t.sp * 0.25,
                    ),
                    child: Text(
                      more > 0 ? 'All ${o.live.length} ›' : 'All ›',
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.labelSize,
                        fontWeight: FontWeight.w600,
                        color: t.accent,
                      ),
                    ),
                  ),
                ),
            ],
          ),
          const SizedBox(height: 10),
          body,
        ],
      ),
    );
  }
}

/// One quiet row: what is true, and what to do about it.
class _BandLine extends StatelessWidget {
  const _BandLine({
    super.key,
    required this.icon,
    required this.title,
    required this.onTap,
    this.detail,
    this.action,
  });

  final IconData icon;
  final String title;
  final String? detail;

  /// The next step, in the accent — "New session". Without one the whole
  /// row is the way forward, and a chevron says so.
  final String? action;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return InkWell(
      borderRadius: BorderRadius.circular(t.rControl),
      onTap: onTap,
      child: Padding(
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 0.5,
          vertical: t.sp * 1.25,
        ),
        child: Row(
          children: [
            // The same column the session rows keep their dot in.
            SizedBox(
              width: t.sp * 2.75,
              child: Align(
                alignment: Alignment.centerLeft,
                child: Icon(icon, size: t.codeSize, color: t.text3),
              ),
            ),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.bodySize,
                      fontWeight: FontWeight.w600,
                      color: t.text,
                    ),
                  ),
                  if (detail != null) ...[
                    SizedBox(height: t.sp * 0.25),
                    Text(
                      detail!,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.codeSize,
                        color: t.text3,
                      ),
                    ),
                  ],
                ],
              ),
            ),
            SizedBox(width: t.sp),
            if (action != null)
              Text(
                '$action ›',
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  fontWeight: FontWeight.w600,
                  color: t.accent,
                ),
              )
            else
              Icon(LucideIcons.chevron_right, size: t.bodySize, color: t.text3),
          ],
        ),
      ),
    );
  }
}

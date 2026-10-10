import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import '../ui/breakpoints.dart';
import '../ui/controls.dart';
import '../ui/shell/sidebar_frame.dart';
import '../ui/tokens.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart';
import 'agents_screen.dart';

/// The session a location shows, or null on the overview.
String? sessionIdOf(Uri location) {
  final seg = location.pathSegments;
  return seg.length == 3 && seg[0] == 'agents' && seg[1] == 's' ? seg[2] : null;
}

/// The frame every Agents screen sits in.
///
/// On a phone it is nothing at all: the child is the screen. On a wide
/// screen it puts [AgentsSidebar] beside it. Built by a `ShellRoute`, so the
/// sidebar keeps its state while the pane moves between the overview and
/// sessions.
class AgentsShell extends StatelessWidget {
  const AgentsShell({super.key, required this.location, required this.child});

  final Uri location;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    if (!context.isExpanded) return child;
    return Scaffold(
      body: Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          AgentsSidebar(selected: sessionIdOf(location)),
          Expanded(child: PaneSemantics(child: child)),
        ],
      ),
    );
  }
}

/// Title, ＋ New session while a host is online, Overview, then RUNNING and
/// ENDED (handoff §2.6).
class AgentsSidebar extends ConsumerWidget {
  const AgentsSidebar({super.key, this.selected});

  /// The open session, if any.
  final String? selected;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final o = ref.watch(agentOverviewProvider).value;
    final sessions = o?.sessions ?? const <AgentSession>[];
    final live = o?.live ?? const <AgentSession>[];
    final ended = o?.ended ?? const <AgentSession>[];
    final overview = selected == null;

    Widget section(String label, List<AgentSession> list, double top) => Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: EdgeInsets.fromLTRB(t.sp, top, t.sp, t.sp * 0.75),
          child: AgentsLabel(label),
        ),
        for (final s in list)
          SessionRow(
            session: s,
            host: o?.hostName(s.hostId) ?? 'host',
            selected: s.id == selected,
            onTap: () => openAgentSession(context, s.id),
          ),
      ],
    );

    return SidebarFrame(
      body: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: EdgeInsets.fromLTRB(
              t.sp * 1.5,
              t.sp * 2,
              t.sp * 1.5,
              t.sp,
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Padding(
                  padding: EdgeInsets.fromLTRB(
                    t.sp,
                    t.sp * 0.75,
                    t.sp,
                    t.sp * 0.5,
                  ),
                  child: Text(
                    'Agents',
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.uiSize,
                      fontWeight: FontWeight.w600,
                      color: t.text,
                    ),
                  ),
                ),
                if (o != null && o.online.isNotEmpty) ...[
                  SizedBox(height: t.sp * 0.75),
                  StormButton.primary(
                    key: const Key('new-session'),
                    label: 'New session',
                    icon: LucideIcons.plus,
                    expand: true,
                    onPressed: () => launchAgentSession(context, ref),
                  ),
                ],
                SizedBox(height: t.sp * 0.75),
                _OverviewRow(
                  selected: overview && sessions.isNotEmpty,
                  current: overview,
                  onTap: () => context.go(Routes.agents),
                ),
              ],
            ),
          ),
          Expanded(
            child: ListView(
              padding: EdgeInsets.fromLTRB(t.sp, 0, t.sp, t.sp * 1.5),
              children: [
                if (live.isNotEmpty) section('Running', live, t.sp * 1.5),
                if (ended.isNotEmpty) section('Ended', ended, t.sp * 2),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _OverviewRow extends StatelessWidget {
  const _OverviewRow({
    required this.selected,
    required this.current,
    required this.onTap,
  });

  /// Filled: the overview is showing and has something on it.
  final bool selected;

  /// In `text`: no session is open.
  final bool current;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final radius = BorderRadius.circular(t.rControl);
    return Semantics(
      button: true,
      selected: current,
      label: 'Overview',
      excludeSemantics: true,
      onTap: onTap,
      child: Material(
        color: selected ? t.surface2 : Colors.transparent,
        borderRadius: radius,
        child: InkWell(
          key: const Key('agents-overview'),
          borderRadius: radius,
          onTap: onTap,
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 1.25,
              vertical: t.sp * 0.875,
            ),
            child: Text(
              'Overview',
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                color: current ? t.text : t.text2,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

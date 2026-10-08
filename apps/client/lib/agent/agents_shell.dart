import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import '../ui/breakpoints.dart';
import '../ui/shell/sidebar_frame.dart';
import '../ui/shell/space_switch.dart';
import '../ui/tokens.dart';
import 'agent_state.dart';
import 'agents_screen.dart';

/// The frame every Agents screen sits in (decision 78).
///
/// On a phone it is nothing at all: the child is the screen. On a wide
/// screen it puts [AgentsSidebar] beside it, as `VaultShell` puts the folder
/// tree beside a note. A sibling of that shell rather than a mode of it,
/// because `VaultShell` carries `VaultGate` and its sidebar needs a vault —
/// and an agent has none.
///
/// Built by a `ShellRoute`, so the sidebar keeps its state while the pane
/// moves between the sessions and Hosts.
class AgentsShell extends StatelessWidget {
  const AgentsShell({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    if (!context.isExpanded) return child;
    return Scaffold(
      body: Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const AgentsSidebar(),
          Expanded(child: child),
        ],
      ),
    );
  }
}

/// The Agents side of the wide sidebar: the space switch, every session,
/// and the actions at the foot — the order the Notes side uses, so the eye
/// finds the same thing in the same place on both.
class AgentsSidebar extends ConsumerWidget {
  const AgentsSidebar({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final active = ref.watch(activeAgentTabProvider);
    final onHosts = GoRouterState.of(context).uri.path == Routes.agentHosts;

    // Selecting a session from Hosts has to bring the sessions pane back.
    void open(String id) {
      openAgentSession(ref, id);
      if (onHosts) context.go(Routes.agents);
    }

    Future<void> launch() async {
      final launched = await launchAgentSession(context, ref);
      if (launched != null && onHosts && context.mounted) {
        context.go(Routes.agents);
      }
    }

    return SidebarFrame(
      body: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const SpaceSwitch(current: StormSpace.agents),
          SizedBox(height: t.sp * 1.5),
          Expanded(
            child: AgentSessionList(
              dense: true,
              selected: onHosts ? null : active,
              onOpen: open,
              onLaunch: launch,
              onHosts: () => context.go(Routes.agentHosts),
            ),
          ),
        ],
      ),
      footer: Padding(
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 1.5,
          vertical: t.sp * 1.25,
        ),
        child: Row(
          children: [
            TextButton.icon(
              key: const Key('new-session'),
              onPressed: launch,
              icon: Icon(LucideIcons.plus, size: t.bodySize),
              label: const Text('New session'),
            ),
            const Spacer(),
            IconButton(
              key: const Key('open-hosts'),
              icon: Icon(
                LucideIcons.server,
                size: t.bodySize,
                color: onHosts ? t.accent : t.text3,
              ),
              tooltip: 'Hosts',
              visualDensity: VisualDensity.compact,
              padding: EdgeInsets.zero,
              constraints: BoxConstraints.tight(Size.square(t.sp * 4.75)),
              onPressed: onHosts ? null : () => context.go(Routes.agentHosts),
            ),
          ],
        ),
      ),
    );
  }
}

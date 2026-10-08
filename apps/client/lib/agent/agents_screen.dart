import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../agent/integrations_api.dart';
import '../api/models.dart';
import '../router.dart';
import '../state/app_state.dart';
import '../state/health.dart' show integrationsSummaryProvider;
import '../ui/breakpoints.dart';
import '../ui/controls.dart';
import '../ui/panels.dart';
import '../ui/session_status.dart';
import '../ui/shell/storm_scaffold.dart' show StormChrome;
import '../ui/states.dart';
import '../ui/tokens.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart';
import 'launcher.dart';

/// Agents at `/agents` (handoff §2.7, §2.8).
///
/// **The phone layout is the default** (the M12 invariant): a flat Running /
/// Ended list with ＋ New session in the pill. At [kExpandedWidth] and wider
/// this is only the pane beside `AgentsSidebar`: the overview of work, or the
/// first-session and no-host states. A session is its own route,
/// `/agents/s/:id` (`SessionScreen`).
class AgentsScreen extends ConsumerWidget {
  const AgentsScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final o = ref.watch(agentOverviewProvider).value;
    final wide = context.isExpanded;
    final t = context.tokens;

    if (wide) {
      final Widget body;
      if (o == null) {
        body = Padding(
          padding: EdgeInsets.fromLTRB(t.sp * 5, t.sp * 3.5, t.sp * 5, 0),
          child: const SkeletonRows(rows: 4),
        );
      } else if (o.unreachable) {
        body = _Offline(onRetry: () => reloadAgents(ref));
      } else if (o.hosts.isEmpty) {
        body = const _NoHost(wide: true);
      } else if (o.sessions.isEmpty) {
        body = _FirstSession(overview: o, wide: true);
      } else {
        body = _Overview(overview: o);
      }
      return Scaffold(
        backgroundColor: t.bg,
        body: SafeArea(left: false, child: body),
      );
    }

    final online = o != null && o.online.isNotEmpty;
    return Scaffold(
      body: StormChrome(
        showNav: false,
        child: Stack(
          children: [
            Positioned.fill(
              child: RefreshIndicator(
                onRefresh: () => reloadAgents(ref),
                child: ListView(
                  padding: EdgeInsets.fromLTRB(
                    StormChrome.contentInset(context),
                    0,
                    StormChrome.contentInset(context),
                    StormChrome.navClearance(context),
                  ),
                  children: [
                    _PageTitle('Agents'),
                    if (o == null)
                      const SkeletonRows(rows: 4)
                    else if (o.unreachable)
                      _Offline(onRetry: () => reloadAgents(ref))
                    else if (o.hosts.isEmpty)
                      const _NoHost(wide: false)
                    else ...[
                      if (o.sessions.isEmpty)
                        _FirstSession(overview: o, wide: false),
                      _PhoneList(overview: o),
                    ],
                  ],
                ),
              ),
            ),
            if (online)
              Positioned(
                left: 0,
                right: 0,
                bottom: 0,
                child: NewSessionPill(
                  onTap: () => launchAgentSession(context, ref),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// The launcher, and everything that follows a launch: the fallback
/// announced (never silent, freeze §6), the list refreshed, and the session
/// opened — on its Context tab when it has a note, else About.
///
/// [contextNote] starts it from a note (Start session); [provider] from an
/// agent card; [runAgain] prefills everything an ended session had.
Future<AgentSession?> launchAgentSession(
  BuildContext context,
  WidgetRef ref, {
  ({String vaultId, String noteId})? contextNote,
  String? provider,
  AgentSession? runAgain,
}) async {
  final overview = ref.read(agentOverviewProvider).value;
  final prefill = runAgain != null
      ? LaunchPrefill.runAgain(runAgain)
      : LaunchPrefill(context: contextNote, provider: provider);
  final launched = await showNewSessionLauncher(
    context,
    hosts: overview?.hosts ?? const [],
    wide: context.isExpanded,
    prefill: prefill,
  );
  if (launched == null || !context.mounted) return launched;
  final fb = launched.fallback;
  if (fb != null) {
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(
          "${providerLabel(fb.requested)} isn't installed on "
          '${overview?.hostName(launched.hostId) ?? 'that host'}. '
          'Using ${providerLabel(fb.used)}.',
        ),
      ),
    );
  }
  ref.invalidate(agentOverviewProvider);
  openAgentSession(
    context,
    launched.id,
    tab: launched.context != null ? SessionTab.context : SessionTab.about,
  );
  return launched;
}

class _PageTitle extends StatelessWidget {
  const _PageTitle(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Text(
      text,
      style: TextStyle(
        fontFamily: StormTokens.sansFamily,
        fontSize: t.titleSize,
        fontWeight: FontWeight.w600,
        color: t.text,
      ),
    );
  }
}

class _Copy extends StatelessWidget {
  const _Copy(this.text, {this.size});

  final String text;
  final double? size;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Text(
      text,
      style: TextStyle(
        fontFamily: StormTokens.sansFamily,
        fontSize: size ?? t.uiSize * 1.05,
        height: 1.55,
        color: t.text2,
      ),
    );
  }
}

class _Offline extends StatelessWidget {
  const _Offline({required this.onRetry});

  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) => EmptyState(
    icon: LucideIcons.cloud_off,
    title: kAgentsOfflineTitle,
    detail: kAgentsOfflineDetail,
    action: 'Try again',
    onAction: onRetry,
    fill: context.isExpanded,
  );
}

/// The desk states' frame: vertically centred, 64 from the pane's edge.
class _Intro extends StatelessWidget {
  const _Intro({required this.maxWidth, required this.children});

  final double maxWidth;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return LayoutBuilder(
      builder: (context, box) => SingleChildScrollView(
        child: ConstrainedBox(
          constraints: BoxConstraints(minHeight: box.maxHeight),
          child: Align(
            alignment: Alignment.centerLeft,
            child: Padding(
              padding: EdgeInsets.symmetric(
                horizontal: t.sp * 8,
                vertical: t.sp * 4,
              ),
              child: ConstrainedBox(
                constraints: BoxConstraints(maxWidth: maxWidth),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    for (var i = 0; i < children.length; i++) ...[
                      if (i > 0) SizedBox(height: t.sp * 1.75),
                      children[i],
                    ],
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// No host enrolled: where agents run, and the three steps to get one.
class _NoHost extends StatelessWidget {
  const _NoHost({required this.wide});

  final bool wide;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final steps = NumberedSteps(
      inline: true,
      steps: [
        NumberedStep(
          'Enroll a host',
          action: wide
              ? StormButton.primary(
                  key: const Key('enroll-host-step'),
                  label: 'Enroll a host',
                  onPressed: () => context.go(Routes.settingsPage('hosts')),
                )
              : null,
        ),
        const NumberedStep('Sign Claude Code or OpenCode in on that host'),
        const NumberedStep('Start a session'),
      ],
    );
    if (!wide) {
      return Padding(
        padding: EdgeInsets.only(top: t.sp),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const _Copy(
              'Agents run on a machine you own. Sessions keep going when '
              'this phone is off.',
            ),
            SizedBox(height: t.sp * 2.5),
            steps,
          ],
        ),
      );
    }
    return _Intro(
      maxWidth: t.sp * 60,
      children: [
        const _PageTitle('Agents run on a machine you own'),
        const _Copy(
          'Storm keeps each session running on that machine, so you can '
          'leave and pick it up from any device.',
        ),
        steps,
      ],
    );
  }
}

/// "build-vm is online with Claude Code and OpenCode."
String _onlineWith(AgentHost h) {
  final agents = [
    for (final p in h.providers)
      if (p.available && p.id != 'shell') p.label,
  ];
  if (agents.isEmpty) return '${h.name} is online';
  final list = agents.length == 1
      ? agents.first
      : '${agents.sublist(0, agents.length - 1).join(', ')} and ${agents.last}';
  return '${h.name} is online with $list';
}

/// Hosts, no sessions: start from a recent note, or without one.
class _FirstSession extends ConsumerWidget {
  const _FirstSession({required this.overview, required this.wide});

  final AgentOverview overview;
  final bool wide;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final host = overview.online.firstOrNull;
    final recents = (ref.watch(recentsProvider).value ?? const <RecentNote>[])
        .take(3)
        .toList();
    void start(RecentNote? n) => launchAgentSession(
      context,
      ref,
      contextNote: n == null ? null : (vaultId: n.vaultId, noteId: n.noteId),
    );

    if (!wide) {
      return Padding(
        padding: EdgeInsets.only(top: t.sp),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _Copy(
              host == null
                  ? 'No host is online. Start storm-runtime on one to begin.'
                  : '${host.name} is online. Start from a note, or tap ＋.',
            ),
            if (host != null)
              for (final n in recents)
                _NoteStartRow(
                  title: '▶  ${noteTitleOf(n.title, n.path)}',
                  vault: n.vaultName,
                  phone: true,
                  onTap: () => start(n),
                ),
          ],
        ),
      );
    }

    return _Intro(
      maxWidth: t.sp * 57.5,
      children: [
        const _PageTitle('Start your first session'),
        _Copy(
          host == null
              ? 'No host is online. Start storm-runtime on one of your hosts, '
                    'and it will appear here.'
              : '${_onlineWith(host)}. A session runs in one of its '
                    'workspaces and keeps going after you close Storm.',
        ),
        if (host != null && recents.isNotEmpty) ...[
          Padding(
            padding: EdgeInsets.only(top: t.sp),
            child: const AgentsLabel('Start from a note'),
          ),
          for (final n in recents)
            _NoteStartRow(
              title: noteTitleOf(n.title, n.path),
              vault: n.vaultName,
              onTap: () => start(n),
            ),
        ],
        if (host != null)
          Padding(
            padding: EdgeInsets.only(top: t.sp * 0.75),
            child: Row(
              children: [
                StormButton.primary(
                  key: const Key('first-new-session'),
                  label: 'New session',
                  onPressed: () => start(null),
                ),
                SizedBox(width: t.sp * 1.5),
                Flexible(
                  child: Text(
                    'or start without a note',
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.codeSize,
                      color: t.text3,
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

class _NoteStartRow extends StatelessWidget {
  const _NoteStartRow({
    required this.title,
    required this.vault,
    required this.onTap,
    this.phone = false,
  });

  final String title;
  final String vault;
  final VoidCallback onTap;
  final bool phone;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final row = Row(
      children: [
        Expanded(
          child: Text(
            title,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: phone ? t.uiSize * 1.05 : t.uiSize,
              color: t.text,
            ),
          ),
        ),
        SizedBox(width: t.sp),
        MonoTag(vault),
      ],
    );
    return Semantics(
      button: true,
      label: 'Start from $title',
      excludeSemantics: true,
      onTap: onTap,
      child: phone
          ? InkWell(
              onTap: onTap,
              child: Container(
                padding: EdgeInsets.symmetric(vertical: t.sp * 1.5),
                decoration: BoxDecoration(
                  border: Border(
                    bottom: BorderSide(color: t.border, width: t.bw),
                  ),
                ),
                child: row,
              ),
            )
          : _HoverCard(
              onTap: onTap,
              padding: EdgeInsets.symmetric(
                horizontal: t.sp * 1.75,
                vertical: t.sp * 1.25,
              ),
              child: row,
            ),
    );
  }
}

/// `surface`, a hairline that turns `accent` under the pointer.
class _HoverCard extends StatefulWidget {
  const _HoverCard({
    required this.onTap,
    required this.padding,
    required this.child,
    this.minWidth = 0,
  });

  final VoidCallback? onTap;
  final EdgeInsets padding;
  final Widget child;
  final double minWidth;

  @override
  State<_HoverCard> createState() => _HoverCardState();
}

class _HoverCardState extends State<_HoverCard> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final radius = BorderRadius.circular(t.rControl);
    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: Material(
        color: t.surface,
        shape: RoundedRectangleBorder(
          borderRadius: radius,
          side: BorderSide(
            color: _hover && widget.onTap != null ? t.accent : t.border,
            width: t.bw,
          ),
        ),
        child: InkWell(
          borderRadius: radius,
          onTap: widget.onTap,
          child: ConstrainedBox(
            constraints: BoxConstraints(minWidth: widget.minWidth),
            child: Padding(padding: widget.padding, child: widget.child),
          ),
        ),
      ),
    );
  }
}

/// "1 of 2 hosts online · 2 integrations, 1 needs sign-in".
String _infraLine(AgentOverview o, List<Integration>? integrations) {
  final n = o.hosts.length;
  final parts = ['${o.online.length} of $n host${n == 1 ? '' : 's'} online'];
  if (integrations != null) {
    final mine = [
      for (final i in integrations)
        if (!i.builtin && !i.disabled) i,
    ];
    final signIn = mine.where((i) => i.needsReconnect).length;
    var text = '${mine.length} integration${mine.length == 1 ? '' : 's'}';
    if (signIn > 0) text += ', $signIn need${signIn == 1 ? 's' : ''} sign-in';
    parts.add(text);
  }
  return parts.join(' · ');
}

class _InfraLine extends ConsumerWidget {
  const _InfraLine({required this.overview, this.phone = false});

  final AgentOverview overview;
  final bool phone;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final line = _infraLine(
      overview,
      ref.watch(integrationsSummaryProvider).value,
    );
    final style = TextStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: phone ? t.labelSize : t.labelSize * 1.09,
      height: 1.6,
      color: t.text3,
    );
    void go() => context.go(Routes.settingsPage('hosts'));
    return Semantics(
      link: true,
      label: '$line. Settings',
      excludeSemantics: true,
      onTap: go,
      child: GestureDetector(
        key: const Key('agents-infra'),
        behavior: HitTestBehavior.opaque,
        onTap: go,
        child: Text.rich(
          TextSpan(
            text: line,
            children: [
              if (!phone)
                TextSpan(
                  text: ' · ',
                  children: [
                    TextSpan(
                      text: 'Settings ›',
                      style: TextStyle(color: t.accent),
                    ),
                  ],
                ),
            ],
          ),
          style: style,
        ),
      ),
    );
  }
}

/// The desk overview: work by (workspace, host), the agents you can start,
/// and the infrastructure line (handoff §2.7).
class _Overview extends ConsumerWidget {
  const _Overview({required this.overview});

  final AgentOverview overview;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final o = overview;
    final groups = <(String, String), List<AgentSession>>{};
    for (final s in o.sessions) {
      (groups[(s.workspace, s.hostId)] ??= []).add(s);
    }

    return SingleChildScrollView(
      child: Align(
        alignment: Alignment.topLeft,
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: t.sp * 95),
          child: Padding(
            padding: EdgeInsets.fromLTRB(
              t.sp * 5,
              t.sp * 3.5,
              t.sp * 5,
              t.sp * 7.5,
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const _PageTitle('Agents'),
                SizedBox(height: t.sp * 0.75),
                _Copy(
                  'What your agents are working on. Sessions keep running on '
                  'your hosts when you close Storm.',
                  size: t.uiSize,
                ),
                SizedBox(height: t.sp * 3),
                const AgentsLabel('Work'),
                SizedBox(height: t.sp * 1.5),
                for (final MapEntry(key: (ws, host), value: rows)
                    in groups.entries) ...[
                  _WorkCard(
                    workspace: ws,
                    host: o.hostName(host),
                    sessions: rows,
                  ),
                  SizedBox(height: t.sp * 1.5),
                ],
                SizedBox(height: t.sp * 1.5),
                const AgentsLabel('Start an agent'),
                SizedBox(height: t.sp * 1.25),
                _AgentCards(overview: o),
                SizedBox(height: t.sp * 3),
                _InfraLine(overview: o),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _WorkCard extends StatelessWidget {
  const _WorkCard({
    required this.workspace,
    required this.host,
    required this.sessions,
  });

  final String workspace;
  final String host;
  final List<AgentSession> sessions;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      // Rows bleed a step past the header so their hover fill reaches
      // toward the card's edge (handoff §2.7).
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 1.25,
        vertical: t.sp * 2,
      ),
      decoration: BoxDecoration(
        color: t.surface,
        borderRadius: BorderRadius.circular(t.rCard),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: EdgeInsets.only(left: t.sp, bottom: t.sp * 0.75),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.baseline,
              textBaseline: TextBaseline.alphabetic,
              children: [
                Text(
                  workspace,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.uiSize * 1.05,
                    fontWeight: FontWeight.w600,
                    color: t.text,
                  ),
                ),
                SizedBox(width: t.sp),
                Text(
                  'on $host',
                  style: TextStyle(
                    fontFamily: StormTokens.monoFamily,
                    fontSize: t.labelSize * 1.09,
                    color: t.text3,
                  ),
                ),
              ],
            ),
          ),
          for (final s in sessions) _WorkRow(session: s),
        ],
      ),
    );
  }
}

class _WorkRow extends StatelessWidget {
  const _WorkRow({required this.session});

  final AgentSession session;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final s = session;
    final radius = BorderRadius.circular(t.rControl);
    void open() => openAgentSession(context, s.id);
    return Semantics(
      button: true,
      label: '${s.name}, ${s.statusLabel}',
      excludeSemantics: true,
      onTap: open,
      child: Material(
        color: Colors.transparent,
        borderRadius: radius,
        child: InkWell(
          key: Key('work-${s.id}'),
          borderRadius: radius,
          hoverColor: t.surface2,
          onTap: open,
          child: Padding(
            padding: EdgeInsets.all(t.sp),
            child: Row(
              children: [
                SessionStatusDot(status: s.status),
                SizedBox(width: t.sp * 1.25),
                Flexible(
                  child: Text(
                    s.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.uiSize,
                      color: t.text,
                    ),
                  ),
                ),
                SizedBox(width: t.sp * 1.25),
                Text(
                  '${providerLabel(s.provider)} · ${sessionWhen(s)}',
                  style: TextStyle(
                    fontFamily: StormTokens.monoFamily,
                    fontSize: t.labelSize * 1.09,
                    color: t.text3,
                  ),
                ),
                const Spacer(),
                if (s.context != null) ...[
                  SizedBox(width: t.sp),
                  NoteContextChip(title: s.context!.title),
                ],
                if (s.wroteCount > 0) ...[
                  SizedBox(width: t.sp * 1.25),
                  Text(
                    'wrote ${s.wroteCount}',
                    style: TextStyle(
                      fontFamily: StormTokens.monoFamily,
                      fontSize: t.labelSize,
                      color: t.accent,
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// One card per agent any host offers: where it can run, or why not.
class _AgentCards extends ConsumerWidget {
  const _AgentCards({required this.overview});

  final AgentOverview overview;

  static const _order = ['claude-code', 'opencode', 'shell'];

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final ids =
        <String>{
          for (final h in overview.hosts)
            for (final p in h.providers) p.id,
        }.toList()..sort((a, b) {
          final ia = _order.indexOf(a), ib = _order.indexOf(b);
          return (ia < 0 ? 99 : ia).compareTo(ib < 0 ? 99 : ib);
        });
    return Wrap(
      spacing: t.sp,
      runSpacing: t.sp,
      children: [for (final id in ids) _agentCard(context, ref, id)],
    );
  }

  Widget _agentCard(BuildContext context, WidgetRef ref, String id) {
    final t = context.tokens;
    bool offers(AgentHost h) =>
        h.providers.any((p) => p.id == id && p.available);
    final on = [
      for (final h in overview.online)
        if (offers(h)) h.name,
    ];
    final off = overview.hosts.where((h) => !h.online && offers(h)).length;
    final where = on.isNotEmpty
        ? on.join(', ')
        : '$off host${off == 1 ? '' : 's'} offline';
    final onTap = on.isEmpty
        ? null
        : () => launchAgentSession(context, ref, provider: id);
    return Semantics(
      button: true,
      enabled: onTap != null,
      label: 'Start ${providerLabel(id)}',
      excludeSemantics: true,
      onTap: onTap,
      child: _HoverCard(
        onTap: onTap,
        minWidth: t.sp * 18.75,
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 1.75,
          vertical: t.sp * 1.25,
        ),
        child: Column(
          key: Key('agent-card-$id'),
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              providerLabel(id),
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.uiSize,
                fontWeight: FontWeight.w500,
                color: t.text,
              ),
            ),
            SizedBox(height: t.sp * 0.25),
            Text(
              where,
              style: TextStyle(
                fontFamily: StormTokens.monoFamily,
                fontSize: t.labelSize,
                color: t.text3,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// The phone list: Running, then Ended, then the infrastructure line. No
/// work grouping — the rows already carry the workspace (handoff §2.8).
class _PhoneList extends StatelessWidget {
  const _PhoneList({required this.overview});

  final AgentOverview overview;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final o = overview;
    Widget section(String label, List<AgentSession> list, double top) => Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: EdgeInsets.only(top: top, bottom: t.sp * 0.5),
          child: AgentsLabel(label),
        ),
        for (final s in list)
          SessionRow(
            session: s,
            phone: true,
            onTap: () => openAgentSession(context, s.id),
          ),
      ],
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (o.live.isNotEmpty) section('Running', o.live, t.sp * 1.75),
        if (o.ended.isNotEmpty) section('Ended', o.ended, t.sp * 2.75),
        Padding(
          padding: EdgeInsets.only(top: t.sp * 2.25),
          child: _InfraLine(overview: o, phone: true),
        ),
      ],
    );
  }
}

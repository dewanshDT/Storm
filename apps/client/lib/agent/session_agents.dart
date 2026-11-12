import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../ui/breakpoints.dart';
import '../ui/decision_strip.dart' show DecisionLayout;
import '../ui/tokens.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart'
    show agentKey, agentMono, agentSans, clockAt, sessionWhen;
import 'sidebar_rules.dart';
import 'spawn_models.dart';
import 'spawn_state.dart';
import 'spawn_widgets.dart';

/// The session inspector's Agents tab (153i): the agents this session
/// started, each with its state in a word, then exactly what it sent them.
/// It took in About's CHILDREN and the Activity tab.

/// Whether a session has an Agents tab: its agent can start agents, or it
/// has started some.
bool showsAgentsTab(AgentSession s, AgentOverview? o) =>
    s.spawn.switchState != null ||
    (o != null && childrenOf(o, s.id).isNotEmpty);

/// The rail's count on Agents: children that need the person (loud), else
/// the ones still running (quiet).
({int count, bool loud})? agentsBadge(AgentSession s, AgentOverview? o) {
  if (o == null) return null;
  final kids = childrenOf(o, s.id);
  final needs = kids.where(needsYou).length;
  if (needs > 0) return (count: needs, loud: true);
  final live = kids.where((k) => !k.ended).length;
  return live > 0 ? (count: live, loud: false) : null;
}

enum ChildTone { danger, accent, quiet }

/// A child's state as one word (153i).
(String, ChildTone) childStateTag(AgentSession k, DateTime now) {
  if (needsYou(k)) return ('Needs you', ChildTone.danger);
  if (tierOf(k, now) == SidebarTier.parked) return ('Parked', ChildTone.quiet);
  if (k.ended) return ('Ended', ChildTone.quiet);
  return ('Running', ChildTone.accent);
}

/// Why, under a child's name: the sidebar's reason without the word its
/// tag already says.
String childReason(AgentSession k, DateTime now, AgentOverview? o) {
  if (workToApply(k) case final work?) return work;
  final tier = tierOf(k, now);
  final line = reasonLine(k, now: now, tier: tier, o: o)?.text;
  if (line == null) return sessionWhen(k);
  return tier == SidebarTier.parked && line.startsWith('parked ')
      ? line.substring('parked '.length)
      : line;
}

/// "START test-gateway", "MESSAGE test-gateway", with how it went when it
/// did not go through.
String sentTitle(AgentAct r, AgentOverview? o) {
  String nameOf(String? id) =>
      id == null ? 'a session' : o?.byId(id)?.displayName ?? id;
  final word = switch (r.kind) {
    'start' => 'START',
    'message' => 'MESSAGE',
    'end' => 'END',
    _ => r.kind.toUpperCase(),
  };
  final who = switch (r.kind) {
    'start' => r.name ?? (r.session == null ? 'a child' : nameOf(r.session)),
    _ => nameOf(r.target),
  };
  final how = switch (r.outcome) {
    'start_failed' => ' · couldn’t start',
    'denied' => ' · denied',
    'expired' => ' · expired',
    'refused' => ' · refused: ${requestRefusalText(r.reason, r.cap)}',
    _ => '',
  };
  return '$word $who$how';
}

/// "14:02 · Claude Code · haiku": when, and a start's agent and model.
String sentWhen(AgentAct r) {
  final when = DateTime.tryParse(r.decidedAt ?? r.createdAt);
  return [
    if (when != null) clockAt(when),
    if (r.kind == 'start' && r.agent != null) providerLabel(r.agent!),
    if (r.kind == 'start') ?r.model,
  ].join(' · ');
}

/// "round 2 · 14s · 12.3k in" (153j): the round an entry began, with what
/// the child's own events say it took. A start begins round 1; it shows
/// only once an event says so.
String? sentRound(AgentAct r, AgentOverview? o) {
  final child = r.kind == 'start' ? r.session : r.target;
  final round = r.round ?? (r.kind == 'start' ? 1 : null);
  if (round == null) return null;
  final events = child == null
      ? const <ChildEvent>[]
      : o?.byId(child)?.spawn.recentEvents ?? const <ChildEvent>[];
  final matching = events.where((e) => e.round == round).toList();
  final full = matching
      .where(
        (e) =>
            e.durationMs != null ||
            e.inputTokens != null ||
            e.outputTokens != null,
      )
      .firstOrNull;
  final e = full ?? matching.firstOrNull;
  final label = e == null ? null : roundLabel(e);
  if (label != null) return label.replaceFirst('Round', 'round');
  return r.round == null ? null : 'round ${r.round}';
}

/// The Agents tab, or in the phone's details sheet ([sheet]) the same
/// sections without their own scroll.
class AgentsPanel extends ConsumerStatefulWidget {
  const AgentsPanel({super.key, required this.session, this.sheet = false});

  final AgentSession session;
  final bool sheet;

  @override
  ConsumerState<AgentsPanel> createState() => _AgentsPanelState();
}

class _AgentsPanelState extends ConsumerState<AgentsPanel> {
  /// End all of them asked in place, where there is no desk strip.
  bool _confirm = false;

  void _endAll() {
    final s = widget.session;
    ref.invalidate(sessionDescendantsProvider(s.id));
    if (context.isExpanded) {
      ref.read(sessionConfirmProvider(s.id).notifier).state =
          SessionConfirm.endChildren;
    } else {
      setState(() => _confirm = true);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final s = widget.session;
    final o = ref.watch(agentOverviewProvider).value;
    final acts = ref.watch(sessionActsProvider(s.id)).value;
    // In the order they were started.
    final kids = o == null ? <AgentSession>[] : childrenOf(o, s.id)
      ..sort((a, b) => a.createdAt.compareTo(b.createdAt));
    final denials = o == null
        ? const <(AgentSession, ChildEvent)>[]
        : deniedOf(o, s.id);
    final now = DateTime.now();
    final sent = acts ?? const <AgentAct>[];

    final children = <Widget>[];
    if (kids.isEmpty && sent.isEmpty && denials.isEmpty) {
      children.add(
        Text(
          'It hasn’t started any agents. Each one it starts shows here, '
          'with what it sent.',
          key: const Key('agents-empty'),
          style: agentSans(t, color: t.text3),
        ),
      );
    }
    if (kids.isNotEmpty) {
      children.addAll([
        Text('STARTED BY THIS SESSION', style: agentKey(t)),
        SizedBox(height: t.sp * 0.5),
        for (final k in kids) _ChildRow(child: k, now: now, overview: o),
        if (kids.any((k) => !k.ended)) ...[
          if (_confirm && !context.isExpanded)
            Padding(
              padding: EdgeInsets.only(top: t.sp),
              child: EndTreeStrip(
                session: s,
                children: true,
                layout: DecisionLayout.sheet,
                onDone: () => setState(() => _confirm = false),
              ),
            )
          else
            _Divided(
              child: Align(
                alignment: Alignment.centerLeft,
                child: AccentLink(
                  key: const Key('end-children'),
                  label: 'End all of them…',
                  danger: true,
                  onTap: _endAll,
                ),
              ),
            ),
        ],
      ]);
    }
    if (sent.isNotEmpty || denials.isNotEmpty) {
      // Oldest first, so it reads as it happened; a child's denials among
      // what was sent to it.
      final log = <(String, Widget)>[
        for (final (k, e) in denials)
          (
            e.at,
            _SentEntry(
              key: Key('act-denied-${e.id}'),
              title: 'DENIED ${k.displayName}',
              when: [
                if (DateTime.tryParse(e.at) case final at?) clockAt(at),
                deniedLabel(e),
              ].join(' · '),
              text: (e.detail ?? '').isEmpty ? null : e.detail,
            ),
          ),
        for (final r in sent.reversed)
          (
            r.decidedAt ?? r.createdAt,
            _SentEntry(
              key: Key('act-${r.id}'),
              id: r.id,
              title: sentTitle(r, o),
              when: sentWhen(r),
              text: actText(r),
              round: sentRound(r, o),
            ),
          ),
      ];
      int at((String, Widget) x) =>
          DateTime.tryParse(x.$1)?.millisecondsSinceEpoch ?? 0;
      final order = {for (var i = 0; i < log.length; i++) log[i].$2: i};
      log.sort((a, b) {
        final byTime = at(a).compareTo(at(b));
        return byTime != 0 ? byTime : order[a.$2]!.compareTo(order[b.$2]!);
      });
      if (children.isNotEmpty) children.add(SizedBox(height: t.sp * 2.5));
      children.addAll([
        Text('WHAT IT SENT THEM', style: agentKey(t)),
        SizedBox(height: t.sp * 0.5),
        for (final (_, entry) in log) entry,
      ]);
    }

    final body = Column(
      key: const Key('agents-panel'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: children,
    );
    if (widget.sheet) return body;
    return SingleChildScrollView(
      padding: EdgeInsets.all(t.sp * 2),
      child: body,
    );
  }
}

/// A row with a hairline under it.
class _Divided extends StatelessWidget {
  const _Divided({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      padding: EdgeInsets.symmetric(vertical: t.sp * 1.5),
      decoration: BoxDecoration(
        border: Border(
          bottom: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      child: child,
    );
  }
}

class _ChildRow extends StatelessWidget {
  const _ChildRow({
    required this.child,
    required this.now,
    required this.overview,
  });

  final AgentSession child;
  final DateTime now;
  final AgentOverview? overview;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final k = child;
    final (word, tone) = childStateTag(k, now);
    final color = switch (tone) {
      ChildTone.danger => t.danger,
      ChildTone.accent => t.accent,
      ChildTone.quiet => t.text3,
    };
    final work = workToApply(k) != null;
    return _Divided(
      child: Row(
        key: Key('agent-child-${k.id}'),
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Wrap(
                  spacing: t.sp,
                  runSpacing: t.sp * 0.5,
                  crossAxisAlignment: WrapCrossAlignment.center,
                  children: [
                    Text(k.displayName, style: agentSans(t)),
                    Container(
                      key: Key('agent-child-tag-${k.id}'),
                      padding: EdgeInsets.symmetric(
                        horizontal: t.sp * 0.75,
                        vertical: t.sp * 0.125,
                      ),
                      decoration: BoxDecoration(
                        borderRadius: BorderRadius.circular(999),
                        border: Border.all(color: color, width: t.bw),
                      ),
                      child: Text(
                        word,
                        maxLines: 1,
                        style: agentMono(t, color: color),
                      ),
                    ),
                  ],
                ),
                SizedBox(height: t.sp * 0.375),
                Text(
                  childReason(k, now, overview),
                  key: Key('agent-child-reason-${k.id}'),
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: agentMono(t),
                ),
              ],
            ),
          ),
          SizedBox(width: t.sp),
          AccentLink(
            key: Key('agent-child-open-${k.id}'),
            label: work ? 'Work' : 'Open',
            onTap: () => openAgentSession(
              context,
              k.id,
              tab: work ? SessionTab.work : null,
            ),
          ),
        ],
      ),
    );
  }
}

/// One entry of what the session sent: the act, when, the exact text in a
/// plate, and the round it began.
class _SentEntry extends StatelessWidget {
  const _SentEntry({
    super.key,
    required this.title,
    required this.when,
    this.id,
    this.text,
    this.round,
  });

  final String? id;
  final String title;
  final String when;
  final String? text;
  final String? round;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return _Divided(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(title, style: agentMono(t, color: t.text)),
          if (when.isNotEmpty) ...[
            SizedBox(height: t.sp * 0.25),
            Text(when, style: agentMono(t)),
          ],
          if (text case final text?) ...[
            SizedBox(height: t.sp * 0.75),
            Container(
              key: id == null ? null : Key('act-text-$id'),
              padding: EdgeInsets.symmetric(
                horizontal: t.sp,
                vertical: t.sp * 0.75,
              ),
              decoration: BoxDecoration(
                color: t.surface2,
                borderRadius: BorderRadius.circular(t.rControl),
              ),
              child: Text(text, style: agentMono(t, color: t.text2)),
            ),
          ],
          if (round case final round?) ...[
            SizedBox(height: t.sp * 0.5),
            Text(
              round,
              key: id == null ? null : Key('act-round-$id'),
              style: agentMono(t),
            ),
          ],
        ],
      ),
    );
  }
}

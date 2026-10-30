import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../ui/controls.dart';
import '../ui/states.dart' show describeFailure;
import '../ui/tokens.dart';
import 'agent_api.dart' show AgentRefusal;
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart' show agentKey, agentMono, agentSans, clockAt;
import 'crew_changes.dart' show announceDismissedCrews;
import 'spawn_api.dart';
import 'spawn_models.dart';
import 'spawn_state.dart';

/// Decision 134's surfaces in a session (138p corrects the handoff's §6,
/// 4m–4w; 145a removed the approval cards): the spawn switch, what the
/// agent did, ending a
/// parent or its children, dismissing a tree, lineage and attention. Each
/// shows only what the server's view says, so on a host that cannot offer
/// spawning there is nothing here to show.

void _say(BuildContext context, String text) {
  if (!context.mounted) return;
  ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
}

String? _code(Object e) => e is AgentRefusal ? e.code : null;

// ---- attention and lineage --------------------------------------------

/// "Needs you" beside a session's activity (134p); nothing when it
/// waits for nobody.
class AttentionTag extends StatelessWidget {
  const AttentionTag({super.key, required this.kind});

  final String? kind;

  @override
  Widget build(BuildContext context) {
    final label = attentionLabel(kind);
    if (label == null) return const SizedBox.shrink();
    final t = context.tokens;
    return Container(
      key: Key('attention-$kind'),
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 0.75,
        vertical: t.sp * 0.125,
      ),
      decoration: BoxDecoration(
        color: kind == 'blocked' || kind == 'usage_limit'
            ? t.amberSoft
            : t.accentSoft,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Text(
        label,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: agentMono(
          t,
          color: kind == 'blocked' || kind == 'usage_limit'
              ? t.amber
              : t.accent,
        ),
      ),
    );
  }
}

/// "↳ from gateway-impl · own copy of storm": a child's sub-line in a list,
/// or null for a session nobody started.
String? lineageLine(AgentSession s, AgentOverview? o) {
  final parentId = s.spawn.parentSession;
  if (parentId == null) return null;
  final parent = o?.byId(parentId);
  final ws = parent?.workspace ?? 'its parent’s workspace';
  final mode = childWorkspaceLabel(s.spawn.childWorkspace, parent: ws);
  return [
    '↳ from ${parent?.displayName ?? 'its parent'}',
    mode,
    ?s.model,
    ?permissionModeWord(s.permissionMode),
  ].join(' · ');
}

/// A child's workspace named for what it is rather than its record's
/// `children/<id>`, with its [parent]'s workspace where the line has room.
String childWorkspaceLabel(ChildWorkspace? w, {String? parent}) =>
    switch (w?.mode) {
      'clone' => parent == null ? 'own copy' : 'own copy of $parent',
      'scratch' => parent == null ? 'scratch folder' : 'scratch beside $parent',
      _ => 'own workspace',
    };

/// [row] on the dashed indent when its parent is listed in [list] too
/// (134l).
Widget lineageIndented(AgentSession s, List<AgentSession> list, Widget row) {
  final p = s.spawn.parentSession;
  return p != null && list.any((o) => o.id == p)
      ? LineageIndent(child: row)
      : row;
}

/// A child's row on a dashed indent: lineage, drawn apart from a crew's
/// solid edge (138p, handoff 4m).
class LineageIndent extends StatelessWidget {
  const LineageIndent({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.only(left: t.sp * 1.25),
      child: CustomPaint(
        painter: _DashedEdge(color: t.text3, width: t.bw),
        child: Padding(
          padding: EdgeInsets.only(left: t.sp),
          child: child,
        ),
      ),
    );
  }
}

class _DashedEdge extends CustomPainter {
  _DashedEdge({required this.color, required this.width});

  final Color color;
  final double width;

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = color
      ..strokeWidth = width;
    const dash = 4.0;
    for (var y = 0.0; y < size.height; y += dash * 2) {
      canvas.drawLine(Offset(0, y), Offset(0, y + dash), paint);
    }
  }

  @override
  bool shouldRepaint(_DashedEdge old) =>
      old.color != color || old.width != width;
}

// ---- the spawn switch ---------------------------------------------------

/// What the switch row says under its label.
String spawnSwitchNote(SpawnSwitch sw, String agent) {
  final why = sw.refused == null ? null : spawnRefusalText(sw.refused!, agent);
  if (sw.enabled) {
    return why == null
        ? 'On · it starts, messages and ends its own agents · Turn off any time'
        : 'On, but not offered now: $why';
  }
  if (sw.refused == 'launched_without_spawn') return kSpawnAtLaunch;
  return why == null ? 'Off' : 'Unavailable · $why';
}

/// The confirmation for turning it on (4t, as 138p point 6 corrects it).
String spawnOnConfirmText(String name, String host) =>
    'Allow $name to start agents? It can then start, message and end '
    'sessions on $host without asking, and each shows here as it happens. '
    'Children never get more access than $name. You can turn this off at '
    'any time; children keep running.';

/// "Allow this session to start agents": a person's switch for one running
/// session (134c, B1, B2). Shown only where the server's view carries it.
class SpawnSwitchRow extends ConsumerStatefulWidget {
  const SpawnSwitchRow({super.key, required this.session, required this.host});

  final AgentSession session;
  final String host;

  @override
  ConsumerState<SpawnSwitchRow> createState() => _SpawnSwitchRowState();
}

class _SpawnSwitchRowState extends ConsumerState<SpawnSwitchRow> {
  bool _confirm = false;
  bool _busy = false;

  Future<void> _set(bool enabled) async {
    final api = agentApi(ref);
    if (api == null) return;
    setState(() => _busy = true);
    final s = widget.session;
    final c = ProviderScope.containerOf(context, listen: false);
    try {
      await api.setSpawn(s.id, enabled);
      if (!enabled && mounted) {
        _say(context, 'Starting agents is off for ${s.displayName}.');
      }
      if (mounted) setState(() => _confirm = false);
    } catch (e) {
      if (mounted) {
        final code = _code(e);
        final why = code == null
            ? null
            : spawnRefusalText(code, providerLabel(s.provider));
        _say(
          context,
          why == null
              ? describeFailure(e)
              : "Couldn't allow it: ${why.endsWith('.') ? why : '$why.'}",
        );
      }
    } finally {
      api.dispose();
      spawnChanged(c, s.id);
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final s = widget.session;
    final sw = s.spawn.switchState;
    if (sw == null || s.ended) return const SizedBox.shrink();
    final t = context.tokens;
    final canTurnOn = sw.refused == null;
    final row = Row(
      children: [
        StormToggle(
          key: const Key('spawn-switch'),
          value: sw.enabled,
          enabled: !_busy && (sw.enabled || canTurnOn),
          onChanged: (on) {
            if (on) {
              setState(() => _confirm = true);
            } else if (_confirm) {
              setState(() => _confirm = false);
            } else {
              _set(false);
            }
          },
        ),
        SizedBox(width: t.sp * 1.25),
        Flexible(
          child: Text(
            'Allow this session to start agents',
            style: agentSans(t),
          ),
        ),
      ],
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        row,
        SizedBox(height: t.sp * 0.5),
        Text(
          spawnSwitchNote(sw, providerLabel(s.provider)),
          key: const Key('spawn-switch-note'),
          maxLines: 3,
          overflow: TextOverflow.ellipsis,
          style: agentMono(t),
        ),
        if (_confirm && !sw.enabled) ...[
          SizedBox(height: t.sp),
          _Banner(
            key: const Key('spawn-on-confirm'),
            title: spawnOnConfirmText(s.displayName, widget.host),
            action: 'Allow',
            accent: true,
            onAction: _busy ? null : () => _set(true),
            onCancel: () => setState(() => _confirm = false),
          ),
        ],
      ],
    );
  }
}

// ---- banners --------------------------------------------------------------

/// The inline banner every confirmation here uses (4p, 4t): a title, groups
/// of names, an optional tick, Cancel and the action.
class _Banner extends StatelessWidget {
  const _Banner({
    super.key,
    required this.title,
    required this.action,
    required this.onAction,
    required this.onCancel,
    this.groups = const [],
    this.note,
    this.tick,
    this.accent = false,
  });

  final String title;
  final String action;
  final VoidCallback? onAction;
  final VoidCallback onCancel;
  final List<(String, List<String>)> groups;
  final String? note;
  final ({bool value, ValueChanged<bool> onChanged, String label})? tick;
  final bool accent;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final fill = accent ? t.accent : t.danger;
    return Container(
      padding: EdgeInsets.symmetric(horizontal: t.sp * 2, vertical: t.sp * 1.5),
      decoration: BoxDecoration(
        color: t.surface2,
        borderRadius: BorderRadius.circular(t.rControl),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            title,
            key: const Key('banner-title'),
            style: agentSans(t, weight: FontWeight.w600),
          ),
          for (final (label, names) in groups)
            if (names.isNotEmpty)
              Padding(
                padding: EdgeInsets.only(top: t.sp * 0.75),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    SizedBox(
                      width: t.sp * 16,
                      child: Text(label.toUpperCase(), style: agentKey(t)),
                    ),
                    Expanded(
                      child: Text(names.join(', '), style: agentSans(t)),
                    ),
                  ],
                ),
              ),
          if (note != null)
            Padding(
              padding: EdgeInsets.only(top: t.sp * 0.75),
              child: Text(
                note!,
                key: const Key('banner-note'),
                style: agentSans(t, color: t.text2),
              ),
            ),
          SizedBox(height: t.sp * 1.25),
          Wrap(
            alignment: WrapAlignment.end,
            crossAxisAlignment: WrapCrossAlignment.center,
            spacing: t.sp,
            runSpacing: t.sp * 0.5,
            children: [
              if (tick case final tick?)
                InkWell(
                  key: const Key('report-first'),
                  onTap: () => tick.onChanged(!tick.value),
                  borderRadius: BorderRadius.circular(t.rControl),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Checkbox(
                        value: tick.value,
                        onChanged: (v) => tick.onChanged(v ?? false),
                        activeColor: t.accent,
                        checkColor: t.onAccent,
                        side: BorderSide(color: t.text3, width: t.bw),
                        visualDensity: VisualDensity.compact,
                      ),
                      Flexible(
                        child: Text(
                          tick.label,
                          style: agentSans(t, color: t.text2),
                        ),
                      ),
                    ],
                  ),
                ),
              StormButton.text(
                key: const Key('banner-cancel'),
                label: 'Cancel',
                onPressed: onCancel,
              ),
              if (onAction != null || action.isNotEmpty)
                Semantics(
                  button: true,
                  enabled: onAction != null,
                  label: action,
                  excludeSemantics: true,
                  onTap: onAction,
                  child: Material(
                    color: onAction == null ? t.surface : fill,
                    borderRadius: BorderRadius.circular(t.rControl),
                    child: InkWell(
                      key: const Key('banner-action'),
                      borderRadius: BorderRadius.circular(t.rControl),
                      onTap: onAction,
                      child: Padding(
                        padding: EdgeInsets.symmetric(
                          horizontal: t.sp * 1.25,
                          vertical: t.sp * 0.75,
                        ),
                        child: Text(
                          action,
                          style: agentSans(
                            t,
                            color: onAction == null ? t.text3 : t.onAccent,
                            weight: FontWeight.w600,
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
            ],
          ),
        ],
      ),
    );
  }
}

/// The tick a person's end offers (C9): children get the notice and 30 s.
const kReportFirst = 'Let children report first (waits 30 s)';

/// Ending a parent ends its children, after a person saw which (C1, 4p).
/// [children] true is End all children (C5): the parent keeps running.
class EndTreeBanner extends ConsumerStatefulWidget {
  const EndTreeBanner({
    super.key,
    required this.session,
    required this.onDone,
    this.children = false,
  });

  final AgentSession session;
  final VoidCallback onDone;
  final bool children;

  @override
  ConsumerState<EndTreeBanner> createState() => _EndTreeBannerState();
}

class _EndTreeBannerState extends ConsumerState<EndTreeBanner> {
  bool _reportFirst = false;
  bool _busy = false;

  Future<void> _go(Descendants d) async {
    final api = agentApi(ref);
    if (api == null) return;
    setState(() => _busy = true);
    final s = widget.session;
    final ids = [for (final r in d.running) r.id];
    final c = ProviderScope.containerOf(context, listen: false);
    try {
      if (widget.children) {
        await api.endChildren(
          s.id,
          descendants: ids,
          reportFirst: _reportFirst,
        );
      } else {
        await api.endConfirmed(
          s.id,
          descendants: ids,
          reportFirst: _reportFirst,
        );
      }
      if (mounted) widget.onDone();
    } catch (e) {
      if (!mounted) return;
      _say(context, switch (_code(e)) {
        'descendants_changed' || 'confirm_descendants' =>
          'What it started changed. Check the list again.',
        'no_live_descendants' => 'None of its children is running now.',
        _ => describeFailure(e),
      });
    } finally {
      api.dispose();
      spawnChanged(c, s.id);
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final s = widget.session;
    final d = ref.watch(sessionDescendantsProvider(s.id)).value;
    final running = <String>[
      for (final r in d?.running ?? const <DescendantRef>[]) r.name,
    ];
    final ended = <String>[
      for (final r in d?.ended ?? const <DescendantRef>[]) r.name,
    ];
    final n = running.length + (widget.children ? 0 : 1);
    return _Banner(
      key: Key(widget.children ? 'end-children-confirm' : 'end-tree-confirm'),
      title: widget.children
          ? 'End ${s.displayName}’s children? ${s.displayName} keeps running.'
          : 'End ${s.displayName}? Its children end with it.',
      groups: [('Running', running), ('Already ended', ended)],
      tick: running.isEmpty
          ? null
          : (
              value: _reportFirst,
              onChanged: (v) => setState(() => _reportFirst = v),
              label: kReportFirst,
            ),
      action: 'End $n ${n == 1 ? 'session' : 'sessions'}',
      onAction: d == null || _busy || n == 0 ? null : () => _go(d),
      onCancel: widget.onDone,
    );
  }
}

/// The warning a tree's dismissal carries (C8 with 137g).
const kTreeDismissWarning =
    'Each child’s own copy or scratch folder is deleted with it, and any of '
    'its work you haven’t applied goes too.';

/// A tree is dismissed only as a unit, once every member has ended (C8).
class DismissTreeBanner extends ConsumerStatefulWidget {
  const DismissTreeBanner({
    super.key,
    required this.session,
    required this.onDone,
    required this.onDismissed,
  });

  final AgentSession session;
  final VoidCallback onDone;
  final VoidCallback onDismissed;

  @override
  ConsumerState<DismissTreeBanner> createState() => _DismissTreeBannerState();
}

class _DismissTreeBannerState extends ConsumerState<DismissTreeBanner> {
  bool _busy = false;

  Future<void> _go() async {
    final api = agentApi(ref);
    if (api == null) return;
    setState(() => _busy = true);
    final c = ProviderScope.containerOf(context, listen: false);
    final before = ref.read(agentOverviewProvider).value;
    final messenger = ScaffoldMessenger.maybeOf(context);
    try {
      final removed = await api.dismiss(widget.session.id, tree: true);
      c.invalidate(agentOverviewProvider);
      announceDismissedCrews(c, before, removed, fallback: messenger);
      widget.onDismissed();
    } catch (e) {
      if (!mounted) return;
      _say(
        context,
        _code(e) == 'tree_live'
            ? 'Dismiss it once every session in it has ended.'
            : describeFailure(e),
      );
    } finally {
      api.dispose();
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final o = ref.watch(agentOverviewProvider).value;
    final s = widget.session;
    final members = o == null ? [s] : treeOf(o, s);
    final live = [
      for (final m in members)
        if (!m.ended) m.displayName,
    ];
    final root = o?.byId(s.spawn.rootSession ?? s.id) ?? s;
    final n = members.length;
    return _Banner(
      key: const Key('dismiss-tree-confirm'),
      title: live.isEmpty
          ? 'Dismiss ${root.displayName} and everything it started? '
                '$n sessions go together.'
          : 'Dismiss ${root.displayName}’s sessions together, once every one '
                'has ended.',
      groups: [
        ('Dismisses', [for (final m in members) m.displayName]),
        ('Still running', live),
      ],
      note: live.isEmpty ? kTreeDismissWarning : null,
      action: live.isEmpty ? 'Dismiss $n sessions' : '',
      onAction: live.isNotEmpty || _busy ? null : _go,
      onCancel: widget.onDone,
    );
  }
}

// ---- what the agent did ------------------------------------------------

/// Whether a session's view says its agent may have acts to show: its
/// switch is on its view (141e).
bool showsActs(AgentSession s) => s.spawn.switchState != null;

/// "Started poet · haiku", "Message to poet · round 2", "Ended poet": an
/// act in one line (145a). Rows from before 145 keep their outcome.
String actTitle(AgentAct r, AgentOverview? o) {
  String nameOf(String? id) =>
      id == null ? 'a session' : o?.byId(id)?.displayName ?? id;
  final child =
      r.name ?? (r.session == null ? null : nameOf(r.session)) ?? 'a child';
  final base = switch (r.kind) {
    'start' => [
      'Started $child',
      if (r.agent != null) providerLabel(r.agent!),
      ?r.model,
    ].join(' · '),
    'message' => [
      'Message to ${nameOf(r.target)}',
      if (r.round != null) 'round ${r.round}',
    ].join(' · '),
    'end' => 'Ended ${nameOf(r.target)}',
    _ => r.kind,
  };
  final how = switch (r.outcome) {
    'start_failed' => ' · couldn’t start',
    'denied' => ' · denied',
    'expired' => ' · expired',
    'refused' => ' · refused: ${requestRefusalText(r.reason, r.cap)}',
    _ => '',
  };
  return '$base$how';
}

/// What the act carried, exactly as the agent sent it: a start's task, a
/// message's text.
String? actText(AgentAct r) => switch (r.kind) {
  'start' => r.task,
  'message' => r.text,
  _ => null,
};

/// What a session's agent started, messaged and ended, newest first: events
/// for the person to read, never questions (145a).
class ActsPanel extends ConsumerWidget {
  const ActsPanel({super.key, required this.session, this.sheet = false});

  final AgentSession session;
  final bool sheet;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final list = ref.watch(sessionActsProvider(session.id)).value;
    final o = ref.watch(agentOverviewProvider).value;
    final denials = o == null
        ? const <(AgentSession, ChildEvent)>[]
        : deniedOf(o, session.id);
    if ((list == null || list.isEmpty) && denials.isEmpty) {
      return Padding(
        padding: EdgeInsets.all(sheet ? 0 : t.sp * 2),
        child: Text(
          'Nothing yet. Each agent this session starts, messages or ends '
          'shows here as it happens.',
          key: const Key('acts-empty'),
          style: agentSans(t, color: t.text3),
        ),
      );
    }
    return ListView(
      key: const Key('acts-list'),
      shrinkWrap: sheet,
      physics: sheet ? const NeverScrollableScrollPhysics() : null,
      padding: EdgeInsets.all(sheet ? 0 : t.sp * 2),
      children: [
        for (final (k, e) in denials)
          Container(
            key: Key('act-denied-${e.id}'),
            padding: EdgeInsets.symmetric(vertical: t.sp * 1.25),
            decoration: BoxDecoration(
              border: Border(
                bottom: BorderSide(color: t.border, width: t.bw),
              ),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  [
                    'DENIED',
                    if (DateTime.tryParse(e.at) case final at?) clockAt(at),
                  ].join(' · '),
                  style: agentKey(t, color: t.text3),
                ),
                SizedBox(height: t.sp * 0.5),
                Text(
                  '${k.displayName}: ${deniedLabel(e)}',
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: agentSans(t),
                ),
                if ((e.detail ?? '').isNotEmpty)
                  Padding(
                    padding: EdgeInsets.only(top: t.sp * 0.5),
                    child: DeniedDetail(event: e, lines: 4),
                  ),
              ],
            ),
          ),
        for (final r in list ?? const <AgentAct>[])
          Container(
            key: Key('act-${r.id}'),
            padding: EdgeInsets.symmetric(vertical: t.sp * 1.25),
            decoration: BoxDecoration(
              border: Border(
                bottom: BorderSide(color: t.border, width: t.bw),
              ),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(_actWhen(r), style: agentKey(t, color: t.text3)),
                SizedBox(height: t.sp * 0.5),
                Text(
                  actTitle(r, o),
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: agentSans(t),
                ),
                if (actText(r) case final text?)
                  Padding(
                    padding: EdgeInsets.only(top: t.sp * 0.5),
                    child: Text(
                      text,
                      maxLines: 4,
                      overflow: TextOverflow.ellipsis,
                      style: agentMono(t, color: t.text2),
                    ),
                  ),
              ],
            ),
          ),
      ],
    );
  }
}

/// What [parent]'s children were refused (decision 148b), newest first:
/// a child never asks the person, so its denials come to its parent.
List<(AgentSession, ChildEvent)> deniedOf(AgentOverview o, String parent) {
  final out = [
    for (final k in childrenOf(o, parent))
      for (final e in k.spawn.recentEvents)
        if (e.kind == 'denied') (k, e),
  ];
  out.sort((a, b) => b.$2.at.compareTo(a.$2.at));
  return out;
}

/// A denial's command or path: the child's own text, shown only as plain
/// data.
class DeniedDetail extends StatelessWidget {
  const DeniedDetail({super.key, required this.event, this.lines = 1});

  final ChildEvent event;
  final int lines;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Text(
      event.detail ?? '',
      maxLines: lines,
      overflow: TextOverflow.ellipsis,
      style: agentMono(t, color: t.text2),
    );
  }
}

String _actWhen(AgentAct r) {
  final when = DateTime.tryParse(r.decidedAt ?? r.createdAt);
  final word = switch (r.kind) {
    'start' => 'START',
    'message' => 'MESSAGE',
    'end' => 'END',
    _ => r.kind.toUpperCase(),
  };
  return when == null ? word : '$word · ${clockAt(when)}';
}

// ---- About ----------------------------------------------------------------

/// The session header's meta line, with a child's own workspace named for
/// what it is rather than its record's `children/<id>`.
String spawnMeta(String meta, AgentSession s, AgentOverview? o) {
  final parent = s.spawn.parentSession;
  if (parent == null) return meta;
  final ws = o?.byId(parent)?.workspace ?? 'its parent’s workspace';
  return meta.replaceFirst(
    s.workspace,
    childWorkspaceLabel(s.spawn.childWorkspace, parent: ws),
  );
}

/// A child's workspace as About says it (137b, 137c): where its work is, and
/// the commit a copy starts from.
String childWorkspaceText(AgentSession s, String parentWorkspace, String host) {
  final w = s.spawn.childWorkspace;
  if (w == null) return 'Its own workspace on $host, made when it starts';
  if (w.clone) {
    final from = w.commit == null ? '' : ' from ${shortCommit(w.commit)}';
    final limits = [
      if (w.submodules) 'submodules',
      if (w.largeFiles) 'large-file pointers',
    ];
    final missing = limits.isEmpty
        ? ''
        : '. Its ${limits.join(' and ')} weren’t copied';
    return 'Its own copy of $parentWorkspace$from on $host (committed work '
        'only)$missing';
  }
  return 'A scratch folder on $host; it reads $parentWorkspace read-only';
}

/// About's spawning rows: who started it, and a resumed child's history
/// (C7).
List<(String, String)> spawnAboutRows(AgentSession s, AgentOverview? o) {
  final parent = s.spawn.parentSession;
  final from = s.resumedFrom == null ? null : o?.byId(s.resumedFrom!);
  final oldParent = from?.spawn.parentSession;
  return [
    if (parent != null)
      (
        'Started by',
        '${o?.byId(parent)?.displayName ?? parent}. It runs with no more '
            'access than it, and ends when it is ended.',
      ),
    if (parent == null && oldParent != null)
      (
        'History',
        'Was started by ${o?.byId(oldParent)?.displayName ?? oldParent} '
            '(history). Resumed by you, so it runs on its own now.',
      ),
  ];
}

/// About's children, with End all children while any runs (C5).
class SpawnChildrenSection extends ConsumerStatefulWidget {
  const SpawnChildrenSection({super.key, required this.session});

  final AgentSession session;

  @override
  ConsumerState<SpawnChildrenSection> createState() =>
      _SpawnChildrenSectionState();
}

class _SpawnChildrenSectionState extends ConsumerState<SpawnChildrenSection> {
  bool _confirm = false;

  @override
  Widget build(BuildContext context) {
    final o = ref.watch(agentOverviewProvider).value;
    final s = widget.session;
    final kids = o == null ? const <AgentSession>[] : childrenOf(o, s.id);
    if (kids.isEmpty) return const SizedBox.shrink();
    final t = context.tokens;
    final running = kids.where((k) => !k.ended).toList();
    return Column(
      key: const Key('spawn-children'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SizedBox(height: t.sp * 2),
        Text('CHILDREN', style: agentKey(t)),
        SizedBox(height: t.sp * 0.5),
        Text(
          [
            for (final k in running) '${k.displayName} (running)',
            for (final k in kids)
              if (k.ended) '${k.displayName} (ended)',
          ].join(' · '),
          style: agentSans(t),
        ),
        for (final k in kids)
          if (k.spawn.childActivity != null ||
              k.spawn.recentEvents.isNotEmpty ||
              (k.spawn.pendingEvents ?? 0) > 0)
            ChildEventsTile(child: k),
        if (running.isNotEmpty) ...[
          SizedBox(height: t.sp),
          if (_confirm)
            EndTreeBanner(
              session: s,
              children: true,
              onDone: () => setState(() => _confirm = false),
            )
          else
            Align(
              alignment: Alignment.centerLeft,
              child: StormButton.outline(
                key: const Key('end-children'),
                label: 'End all children…',
                onPressed: () {
                  ref.invalidate(sessionDescendantsProvider(s.id));
                  setState(() => _confirm = true);
                },
              ),
            ),
        ],
      ],
    );
  }
}

/// One child's health and latest events (144a, 144b): its activity as
/// evidence says it ("Unknown" until a gated signal says more, never
/// working), how many events its parent has not read, and each event with
/// where it came from when that is less than certain.
class ChildEventsTile extends StatelessWidget {
  const ChildEventsTile({super.key, required this.child});

  final AgentSession child;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final k = child;
    final activity = k.ended ? null : activityLabel(k.spawn.childActivity);
    final pending = k.spawn.pendingEvents ?? 0;
    final events = k.spawn.recentEvents.reversed.toList();
    return Padding(
      key: Key('child-events-${k.id}'),
      padding: EdgeInsets.only(top: t.sp),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Wrap(
            spacing: t.sp,
            runSpacing: t.sp * 0.5,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              Text(k.displayName, style: agentSans(t, weight: FontWeight.w600)),
              if (activity != null)
                _Pill(
                  key: Key('child-activity-${k.id}'),
                  text: activity,
                  color: activity == 'Working' ? t.accent : t.text3,
                  fill: activity == 'Working' ? t.accentSoft : t.surface2,
                ),
              if (pending > 0)
                _Pill(
                  key: Key('child-pending-${k.id}'),
                  text: pending == 1
                      ? '1 unread event'
                      : '$pending unread events',
                  color: t.accent,
                  fill: t.accentSoft,
                ),
            ],
          ),
          for (final e in events)
            Padding(
              key: Key('child-event-${e.id}'),
              padding: EdgeInsets.only(top: t.sp * 0.5),
              child: Wrap(
                spacing: t.sp * 0.75,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  Text(
                    [
                      childEventLabel(e),
                      if (DateTime.tryParse(e.at) case final at?) clockAt(at),
                    ].join(' · '),
                    style: agentMono(t),
                  ),
                  if (evidenceLabel(e) case final evidence?)
                    _Pill(
                      key: Key('event-evidence-${e.id}'),
                      text: evidence,
                      color: t.amber,
                      fill: t.amberSoft,
                    ),
                  if (e.kind == 'denied' && (e.detail ?? '').isNotEmpty)
                    DeniedDetail(key: Key('event-detail-${e.id}'), event: e),
                ],
              ),
            ),
        ],
      ),
    );
  }
}

class _Pill extends StatelessWidget {
  const _Pill({
    super.key,
    required this.text,
    required this.color,
    required this.fill,
  });

  final String text;
  final Color color;
  final Color fill;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 0.75,
        vertical: t.sp * 0.125,
      ),
      decoration: BoxDecoration(
        color: fill,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Text(text, maxLines: 1, style: agentMono(t, color: color)),
    );
  }
}

/// A child's "↳ started by gateway-impl ›" and any attention, under the
/// session's name; nothing for a session nobody started that waits for
/// nobody.
class SpawnHeaderLine extends ConsumerWidget {
  const SpawnHeaderLine({super.key, required this.session, this.onParent});

  final AgentSession session;
  final ValueChanged<String>? onParent;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final s = session;
    final parent = s.spawn.parentSession;
    final attention = s.ended ? null : s.spawn.needsAttention;
    if (parent == null && attention == null) return const SizedBox.shrink();
    final t = context.tokens;
    final o = ref.watch(agentOverviewProvider).value;
    return Padding(
      padding: EdgeInsets.only(top: t.sp * 0.5),
      child: Wrap(
        spacing: t.sp,
        runSpacing: t.sp * 0.5,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          if (parent != null)
            InkWell(
              key: const Key('started-by'),
              onTap: onParent == null ? null : () => onParent!(parent),
              borderRadius: BorderRadius.circular(t.rControl),
              child: Container(
                padding: EdgeInsets.symmetric(
                  horizontal: t.sp * 0.75,
                  vertical: t.sp * 0.125,
                ),
                decoration: BoxDecoration(
                  borderRadius: BorderRadius.circular(t.rControl),
                  border: Border.all(color: t.accent, width: t.bw),
                ),
                child: Text(
                  '↳ started by ${o?.byId(parent)?.displayName ?? 'its parent'} ›',
                  style: agentMono(t, color: t.accent),
                ),
              ),
            ),
          AttentionTag(kind: attention),
        ],
      ),
    );
  }
}

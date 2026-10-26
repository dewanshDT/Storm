import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart' show VaultInfo;
import '../state/app_state.dart' show vaultsProvider;
import '../ui/controls.dart';
import '../ui/states.dart' show describeFailure;
import '../ui/tokens.dart';
import 'agent_api.dart' show AgentRefusal;
import 'agent_models.dart';
import 'agent_state.dart';
import 'spawn_api.dart';
import 'spawn_models.dart';
import 'spawn_state.dart';

/// Decision 134's surfaces in a session (138p corrects the handoff's §6,
/// 4m–4w): the spawn switch, approval cards, the Requests list, ending a
/// parent or its children, dismissing a tree, lineage and attention. Each
/// shows only what the server's view says, so on a host that cannot offer
/// spawning there is nothing here to show.

TextStyle _mono(StormTokens t, [Color? color]) => TextStyle(
  fontFamily: StormTokens.monoFamily,
  fontSize: t.labelSize,
  color: color ?? t.text3,
);

TextStyle _sans(StormTokens t, {Color? color, FontWeight? weight}) => TextStyle(
  fontFamily: StormTokens.sansFamily,
  fontSize: t.codeSize,
  height: 1.4,
  fontWeight: weight,
  color: color ?? t.text,
);

TextStyle _key(StormTokens t, [Color? color]) => TextStyle(
  fontFamily: StormTokens.monoFamily,
  fontSize: t.labelSize,
  letterSpacing: t.labelSize * 0.08,
  color: color ?? t.text3,
);

String _two(int n) => n.toString().padLeft(2, '0');

String _at(String? iso) {
  final t = iso == null ? null : DateTime.tryParse(iso)?.toLocal();
  return t == null ? '' : '${_two(t.hour)}:${_two(t.minute)}';
}

void _say(BuildContext context, String text) {
  if (!context.mounted) return;
  ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
}

String? _code(Object e) => e is AgentRefusal ? e.code : null;

// ---- attention and lineage --------------------------------------------

/// "Needs approval" beside a session's activity (134p); nothing when it
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
        color: kind == 'blocked' ? t.amberSoft : t.accentSoft,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Text(
        label,
        maxLines: 1,
        style: _mono(t, kind == 'blocked' ? t.amber : t.accent),
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
  final mode = switch (s.spawn.childWorkspace?.mode) {
    'clone' => 'own copy of $ws',
    'scratch' => 'scratch beside $ws',
    _ => 'own workspace',
  };
  return '↳ from ${parent?.displayName ?? 'its parent'} · $mode';
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
        ? 'On · you approve each start, message and end · Turn off any time'
        : 'On, but not offered now: $why';
  }
  return why == null ? 'Off' : 'Unavailable · $why';
}

/// The confirmation for turning it on (4t, as 138p point 6 corrects it).
String spawnOnConfirmText(String name, String host) =>
    'Allow $name to start agents? It can then ask to start, message and end '
    'sessions on $host. Each request waits for you, and children never get '
    'more access than $name. You can turn this off at any time: requests '
    'still waiting are refused then, and children keep running.';

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
    try {
      final refused = await api.setSpawn(s.id, enabled);
      if (!enabled && mounted) {
        _say(
          context,
          refused == 0
              ? 'Starting agents is off for ${s.displayName}.'
              : 'Starting agents is off for ${s.displayName}. $refused '
                    'waiting ${refused == 1 ? 'request was' : 'requests were'} '
                    'refused.',
        );
      }
      if (mounted) setState(() => _confirm = false);
    } catch (e) {
      if (mounted) {
        final code = _code(e);
        _say(
          context,
          code == null
              ? describeFailure(e)
              : "Couldn't allow it: "
                    '${spawnRefusalText(code, providerLabel(s.provider))}.',
        );
      }
    } finally {
      api.dispose();
      spawnChanged(ref, s.id);
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
        Text('Allow this session to start agents', style: _sans(t)),
        SizedBox(width: t.sp * 1.25),
        Flexible(
          child: Text(
            spawnSwitchNote(sw, providerLabel(s.provider)),
            key: const Key('spawn-switch-note'),
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: _mono(t),
          ),
        ),
      ],
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        row,
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
            style: _sans(t, weight: FontWeight.w600),
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
                      child: Text(label.toUpperCase(), style: _key(t)),
                    ),
                    Expanded(child: Text(names.join(', '), style: _sans(t))),
                  ],
                ),
              ),
          if (note != null)
            Padding(
              padding: EdgeInsets.only(top: t.sp * 0.75),
              child: Text(
                note!,
                key: const Key('banner-note'),
                style: _sans(t, color: t.text2),
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
                          style: _sans(t, color: t.text2),
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
                          style: _sans(
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
      widget.onDone();
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
      spawnChanged(ref, s.id);
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
    try {
      await api.dismissTree(widget.session.id);
      ref.invalidate(agentOverviewProvider);
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

// ---- approval cards -------------------------------------------------------

/// Who and what a card is about, read from the overview.
class ApprovalFacts {
  const ApprovalFacts({
    required this.request,
    required this.requester,
    required this.host,
    this.targetName,
    this.targetDescendants = const [],
    this.writeVault,
  });

  final SpawnRequest request;
  final AgentSession requester;
  final String host;
  final String? targetName;

  /// For an end: what the target started, which ends with it (134-7).
  final List<String> targetDescendants;

  /// The requester's own write vault, the only one D2 lets a child have.
  final String? writeVault;
}

/// The card's heading: who asks for what.
String approvalTitle(ApprovalFacts f) {
  final who = f.requester.displayName;
  final target = f.targetName ?? 'one of its sessions';
  return switch (f.request.kind) {
    'start' => '$who wants to start a session',
    'message' => '$who wants to message $target',
    'end' => '$who wants to end $target',
    _ => '$who is asking',
  };
}

/// The card's rows (134o): who asks, what exactly, where it runs and the
/// host's identity and isolation states. The bool marks the exact text,
/// drawn as the agent sent it.
List<(String, String, bool)> approvalRows(ApprovalFacts f) {
  final r = f.request;
  final s = f.requester;
  final agent = providerLabel(s.provider);
  final ws = s.workspace;
  final asker = ('Asked by', '${s.displayName} · $agent · ${f.host}', false);
  final host = (
    'Host',
    '${f.host} · ${hostStatesLine(s.identityState, s.isolationState)}',
    false,
  );
  final target = f.targetName ?? r.target ?? '';
  return switch (r.kind) {
    'start' => [
      asker,
      ('Task', r.task ?? '', true),
      if (r.name != null) ('Name', r.name!, false),
      if (r.contextTitle != null) ('Note', r.contextTitle!, false),
      (
        'Runs as',
        '$agent on ${f.host}, in a workspace of its own: a copy of $ws’s '
            'last commit (uncommitted changes are not included), or $ws '
            'read-only with a scratch folder if it isn’t a git repository',
        false,
      ),
      host,
    ],
    'message' => [
      asker,
      ('To', '$target, which it started', false),
      ('Message', r.text ?? '', true),
      host,
    ],
    'end' => [
      asker,
      (
        'Ends',
        f.targetDescendants.isEmpty
            ? '$target, which it started'
            : '$target, which it started, and what $target started: '
                  '${f.targetDescendants.join(', ')}',
        false,
      ),
      host,
    ],
    _ => [asker, host],
  };
}

/// The write toggle on a start's card (D2): off by default.
String writeToggleLabel(String vault) => 'Let the child write to $vault';

/// The card's facts for a request, from the overview, or null while the
/// requester is unknown.
ApprovalFacts? approvalFactsOf(
  SpawnRequest r,
  AgentOverview? o,
  List<VaultInfo> vaults, {
  AgentSession? requester,
  Descendants? targetDescendants,
}) {
  final s = requester ?? o?.byId(r.requester);
  if (s == null) return null;
  final target = r.target == null ? null : o?.byId(r.target!);
  return ApprovalFacts(
    request: r,
    requester: s,
    host: o?.hostName(s.hostId) ?? 'host',
    targetName: target?.displayName ?? r.target,
    targetDescendants: [
      for (final d in targetDescendants?.running ?? const <DescendantRef>[])
        d.name,
    ],
    writeVault: s.isShell || s.writeVaultId == null
        ? null
        : vaults.where((v) => v.id == s.writeVaultId).firstOrNull?.name ??
              'its write vault',
  );
}

/// What a person sees once they have decided: the outcome, never a line in
/// any terminal (138p point 2).
String decidedText(SpawnRequest r, AgentOverview? o) {
  String nameOf(String? id) =>
      id == null ? 'the session' : o?.byId(id)?.displayName ?? id;
  return switch (r.outcome) {
    'started' => 'Approved: ${r.name ?? 'its child'} is starting.',
    'start_failed' => 'Approved, but it couldn’t start.',
    'delivered' => 'Approved: the message is in ${nameOf(r.target)}’s inbox.',
    'ended' => 'Approved: ${nameOf(r.target)} is ending.',
    'denied' => 'Denied.',
    'expired' => 'It expired before it was decided.',
    'refused' => 'Refused: ${requestRefusalText(r.reason, r.cap)}.',
    _ => 'Decided.',
  };
}

/// One waiting request, decided here by this signed-in device (A5). Desk:
/// a card over the session's terminal (4m). Phone: [sheet] lays it out as
/// 4w, with full-width Approve and Deny. There is no other way to decide,
/// and no notification carries Approve or Deny (138p point 7).
class ApprovalCard extends ConsumerStatefulWidget {
  const ApprovalCard({
    super.key,
    required this.request,
    this.requester,
    this.sheet = false,
    this.onDecided,
  });

  final SpawnRequest request;
  final AgentSession? requester;
  final bool sheet;
  final VoidCallback? onDecided;

  @override
  ConsumerState<ApprovalCard> createState() => _ApprovalCardState();
}

class _ApprovalCardState extends ConsumerState<ApprovalCard> {
  bool _write = false;
  bool _busy = false;
  Timer? _tick;

  @override
  void initState() {
    super.initState();
    _tick = Timer.periodic(const Duration(seconds: 1), (_) {
      if (mounted) setState(() {});
    });
  }

  @override
  void dispose() {
    _tick?.cancel();
    super.dispose();
  }

  Future<void> _decide(bool approve) async {
    final api = agentApi(ref);
    if (api == null) return;
    setState(() => _busy = true);
    final r = widget.request;
    try {
      final decided = approve
          ? await api.approve(r.id, writeVault: _write)
          : await api.deny(r.id);
      if (mounted) {
        _say(
          context,
          decidedText(decided, ref.read(agentOverviewProvider).value),
        );
      }
      widget.onDecided?.call();
    } catch (e) {
      if (mounted) {
        _say(
          context,
          _code(e) == 'already_decided'
              ? 'Already decided, on another device or by its expiry.'
              : describeFailure(e),
        );
      }
    } finally {
      api.dispose();
      spawnChanged(ref, r.requester);
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final r = widget.request;
    final o = ref.watch(agentOverviewProvider).value;
    final vaults = ref.watch(vaultsProvider).value ?? const <VaultInfo>[];
    final ends = r.kind == 'end' && r.target != null
        ? ref.watch(sessionDescendantsProvider(r.target!)).value
        : null;
    final f = approvalFactsOf(
      r,
      o,
      vaults,
      requester: widget.requester,
      targetDescendants: ends,
    );
    if (f == null) return const SizedBox.shrink();
    final left = r.left(DateTime.now());
    final expired = left == Duration.zero;
    final kind = r.kind.toUpperCase();
    final title = approvalTitle(f);
    final rows = approvalRows(f);
    final writeVault = r.kind == 'start' ? f.writeVault : null;

    Widget value(String text, bool exact) => exact
        ? Container(
            width: double.infinity,
            padding: EdgeInsets.symmetric(
              horizontal: t.sp,
              vertical: t.sp * 0.75,
            ),
            decoration: BoxDecoration(
              color: t.surface2,
              borderRadius: BorderRadius.circular(t.rControl),
            ),
            child: SelectableText(
              text,
              key: const Key('approval-exact'),
              style: TextStyle(
                fontFamily: StormTokens.monoFamily,
                fontSize: t.labelSize * 1.09,
                height: 1.45,
                color: t.text,
              ),
            ),
          )
        : Text(text, style: _sans(t));

    final toggle = writeVault == null
        ? null
        : Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              StormToggle(
                key: const Key('approval-write'),
                value: _write,
                onChanged: _busy ? null : (v) => setState(() => _write = v),
              ),
              SizedBox(width: t.sp),
              Flexible(
                child: Text(
                  writeToggleLabel(writeVault),
                  style: _sans(t, color: t.text2),
                ),
              ),
            ],
          );
    final deny = StormButton.outline(
      key: const Key('approval-deny'),
      label: 'Deny',
      expand: widget.sheet,
      onPressed: _busy || expired ? null : () => _decide(false),
    );
    final approve = StormButton.primary(
      key: const Key('approval-approve'),
      label: 'Approve',
      expand: widget.sheet,
      onPressed: _busy || expired ? null : () => _decide(true),
    );
    final heading = Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(kind, style: _key(t, t.accent)),
        SizedBox(width: t.sp),
        Expanded(
          child: Text(
            title,
            key: const Key('approval-title'),
            style: _sans(t, weight: FontWeight.w600),
          ),
        ),
        SizedBox(width: t.sp),
        Text(
          expired ? 'expired' : 'expires in ${clockLeft(left)}',
          key: const Key('approval-left'),
          style: _mono(t, expired ? t.danger : t.text3),
        ),
      ],
    );
    final body = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        heading,
        if (widget.sheet) ...[
          SizedBox(height: t.sp * 0.75),
          Text(
            'Nothing happens until you decide here.',
            style: _sans(t, color: t.text2),
          ),
        ],
        for (final (k, v, exact) in rows)
          Padding(
            padding: EdgeInsets.only(top: t.sp * 1.25),
            child: widget.sheet
                ? Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(k.toUpperCase(), style: _key(t)),
                      SizedBox(height: t.sp * 0.5),
                      value(v, exact),
                    ],
                  )
                : Row(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      SizedBox(
                        width: t.sp * 12,
                        child: Text(k.toUpperCase(), style: _key(t)),
                      ),
                      Expanded(child: value(v, exact)),
                    ],
                  ),
          ),
        SizedBox(height: t.sp * 2),
        if (widget.sheet) ...[
          ?toggle,
          SizedBox(height: t.sp * 2),
          approve,
          SizedBox(height: t.sp),
          deny,
        ] else
          Row(
            children: [
              if (toggle != null) Expanded(child: toggle) else const Spacer(),
              SizedBox(width: t.sp),
              deny,
              SizedBox(width: t.sp),
              approve,
            ],
          ),
      ],
    );
    return Semantics(
      container: true,
      label: 'Request: $title',
      child: Container(
        key: Key('approval-${r.id}'),
        padding: EdgeInsets.all(t.sp * 2),
        decoration: widget.sheet
            ? null
            : BoxDecoration(
                color: t.surface,
                borderRadius: BorderRadius.circular(t.rCard),
                border: Border.all(color: t.accent, width: t.bw),
              ),
        child: body,
      ),
    );
  }
}

/// The phone's way to a waiting request (4w): its sheet, opened from the
/// session's "Needs approval".
Future<void> showApprovalSheet(
  BuildContext context,
  SpawnRequest r,
  AgentSession requester,
) {
  final t = context.tokens;
  return showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    backgroundColor: t.surface,
    barrierColor: const Color(0x59000000),
    shape: RoundedRectangleBorder(
      borderRadius: BorderRadius.vertical(top: Radius.circular(t.rCard)),
      side: BorderSide(color: t.border, width: t.bw),
    ),
    builder: (sheet) => ConstrainedBox(
      constraints: BoxConstraints(
        maxHeight: MediaQuery.sizeOf(sheet).height * 0.85,
      ),
      child: SafeArea(
        top: false,
        child: SingleChildScrollView(
          child: ApprovalCard(
            request: r,
            requester: requester,
            sheet: true,
            onDecided: () => Navigator.of(sheet).maybePop(),
          ),
        ),
      ),
    ),
  );
}

/// Whether a session's view says it could have requests to show: its switch
/// is on its view, or one of its requests waits.
bool showsRequests(AgentSession s) =>
    s.spawn.switchState != null || s.spawn.needsAttention == 'approval';

/// The oldest request of [s] still waiting for a person.
SpawnRequest? waitingRequest(WidgetRef ref, AgentSession s) {
  if (!showsRequests(s)) return null;
  final list = ref.watch(sessionRequestsProvider(s.id)).value;
  final now = DateTime.now();
  return list?.reversed.where((r) => r.waiting(now)).firstOrNull;
}

/// The phone's line above the terminal while a request waits: a tap opens
/// its sheet.
class ApprovalBanner extends ConsumerWidget {
  const ApprovalBanner({super.key, required this.session});

  final AgentSession session;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final r = waitingRequest(ref, session);
    if (r == null) return const SizedBox.shrink();
    final t = context.tokens;
    final what = switch (r.kind) {
      'start' => 'start a session',
      'message' => 'send a message',
      'end' => 'end a session',
      _ => 'do something',
    };
    return Material(
      color: t.accentSoft,
      child: InkWell(
        key: const Key('approval-banner'),
        onTap: () => showApprovalSheet(context, r, session),
        child: Padding(
          padding: EdgeInsets.symmetric(
            horizontal: t.sp * 2,
            vertical: t.sp * 1.25,
          ),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  '${session.displayName} is asking to $what',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: _sans(t, color: t.accent),
                ),
              ),
              Text(
                'Review ›',
                style: _sans(t, color: t.accent, weight: FontWeight.w600),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

// ---- the Requests list ----------------------------------------------------

/// "Waiting · 8:42 left", "Refused · 12:55": a request's state, as the
/// Requests tab lists it (4m), shown to the person and nowhere else.
String requestStateLine(SpawnRequest r, DateTime now) {
  if (r.waiting(now)) return 'Waiting · ${clockLeft(r.left(now))} left';
  final at = _at(r.decidedAt ?? r.expiresAt);
  final word = switch (r.outcome) {
    'started' => 'Started',
    'start_failed' => 'Couldn’t start',
    'delivered' => 'Delivered',
    'ended' => 'Ended',
    'denied' => 'Denied',
    'refused' => 'Refused',
    _ => 'Expired',
  };
  return at.isEmpty ? word : '$word · $at';
}

/// What a request asked for, and how it ended, in one line.
String requestSummary(SpawnRequest r, AgentOverview? o) {
  String nameOf(String? id) =>
      id == null ? 'a session' : o?.byId(id)?.displayName ?? id;
  final what = switch (r.kind) {
    'start' => 'Start: “${r.task ?? ''}”',
    'message' => 'Message to ${nameOf(r.target)}: “${r.text ?? ''}”',
    'end' => 'End ${nameOf(r.target)}',
    _ => r.kind,
  };
  final how = switch (r.outcome) {
    'started' => ' · ${nameOf(r.session)}',
    'refused' => ' · ${requestRefusalText(r.reason, r.cap)}',
    'expired' => ' · no answer in 10 min',
    _ =>
      r.outcome == null && !r.waiting(DateTime.now())
          ? ' · no answer in 10 min'
          : '',
  };
  return '$what$how';
}

/// The Requests tab: every request the session made and its outcome,
/// newest first, with the waiting one's card.
class RequestsPanel extends ConsumerWidget {
  const RequestsPanel({super.key, required this.session, this.sheet = false});

  final AgentSession session;
  final bool sheet;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final list = ref.watch(sessionRequestsProvider(session.id)).value;
    final o = ref.watch(agentOverviewProvider).value;
    final now = DateTime.now();
    if (list == null || list.isEmpty) {
      return Padding(
        padding: EdgeInsets.all(sheet ? 0 : t.sp * 2),
        child: Text(
          'No requests yet. Each start, message and end this session asks '
          'for waits here for you.',
          key: const Key('requests-empty'),
          style: _sans(t, color: t.text3),
        ),
      );
    }
    return ListView(
      key: const Key('requests-list'),
      shrinkWrap: sheet,
      physics: sheet ? const NeverScrollableScrollPhysics() : null,
      padding: EdgeInsets.all(sheet ? 0 : t.sp * 2),
      children: [
        for (final r in list)
          Container(
            key: Key('request-${r.id}'),
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
                  requestStateLine(r, now).toUpperCase(),
                  style: _key(t, r.waiting(now) ? t.accent : t.text3),
                ),
                SizedBox(height: t.sp * 0.5),
                Text(
                  requestSummary(r, o),
                  maxLines: 3,
                  overflow: TextOverflow.ellipsis,
                  style: _sans(t),
                ),
                if (r.waiting(now) && sheet)
                  Padding(
                    padding: EdgeInsets.only(top: t.sp * 0.75),
                    child: StormButton.soft(
                      key: Key('review-${r.id}'),
                      label: 'Review',
                      onPressed: () => showApprovalSheet(context, r, session),
                    ),
                  ),
              ],
            ),
          ),
      ],
    );
  }
}

// ---- About ----------------------------------------------------------------

/// The session header's meta line, with a child's own workspace named for
/// what it is rather than its record's `children/<id>`.
String spawnMeta(String meta, AgentSession s, AgentOverview? o) {
  final parent = s.spawn.parentSession;
  if (parent == null) return meta;
  final ws = o?.byId(parent)?.workspace ?? 'its parent’s workspace';
  final own = switch (s.spawn.childWorkspace?.mode) {
    'clone' => 'own copy of $ws',
    'scratch' => 'scratch beside $ws',
    _ => 'own workspace',
  };
  return meta.replaceFirst(s.workspace, own);
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
        Text('CHILDREN', style: _key(t)),
        SizedBox(height: t.sp * 0.5),
        Text(
          [
            for (final k in running) '${k.displayName} (running)',
            for (final k in kids)
              if (k.ended) '${k.displayName} (ended)',
          ].join(' · '),
          style: _sans(t),
        ),
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
                  style: _mono(t, t.accent),
                ),
              ),
            ),
          AttentionTag(kind: attention),
        ],
      ),
    );
  }
}

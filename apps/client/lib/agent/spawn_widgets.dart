import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import '../ui/breakpoints.dart';
import '../ui/controls.dart';
import '../ui/decision_strip.dart';
import '../ui/states.dart' show describeFailure;
import '../ui/surfaces.dart' show StormPopover, showStormPopover;
import '../ui/tokens.dart';
import '../ui/widgets.dart' show PopoverItem, PopoverTone;
import 'agent_api.dart' show AgentRefusal;
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart' show agentMono, agentSans;
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

/// A child's workspace named for what it is rather than its record's
/// `children/<id>`, with its [parent]'s workspace where the line has room.
/// A child without one of its own runs in a workspace by name (151 N6).
String childWorkspaceLabel(AgentSession s, {String? parent}) {
  if (!s.hasOwnWorkspace) return 'in ${s.workspace}';
  return switch (s.spawn.childWorkspace?.mode) {
    'clone' => parent == null ? 'own copy' : 'own copy of $parent',
    'scratch' => parent == null ? 'scratch folder' : 'scratch beside $parent',
    _ => 'own workspace',
  };
}

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

/// The confirmation for turning it on (4t, as 138p point 6 corrects it).
String spawnOnConfirmText(String name, String host) =>
    'Allow $name to start agents? It can then start, message and end '
    'sessions on $host without asking, and each shows here as it happens. '
    'Children never get more access than $name. You can turn this off at '
    'any time; children keep running.';

/// Whether a session shows the spawn switch, now the header's "Starts
/// agents ▾" chip and About's Starts agents (153j): only where the
/// server's view carries it, while it runs.
bool showsSpawnSwitch(AgentSession s) =>
    s.spawn.switchState != null && !s.ended;

/// Whether turning it on is waiting for the person's Allow, asked from the
/// header chip or About at desk width, and shown in the strip under the
/// header. Device-local.
final spawnOnAskProvider = StateProvider.autoDispose.family<bool, String>(
  (ref, id) => false,
);

/// About's Starts agents, in words (153j): on with the host's limits, or
/// why it is off.
String startsAgentsText(AgentSession s, AgentOverview? o) {
  final sw = s.spawn.switchState;
  if (sw == null) return 'Off';
  final agent = providerLabel(s.provider);
  final why = sw.refused == null ? null : spawnRefusalText(sw.refused!, agent);
  if (sw.enabled) {
    if (why != null) return 'On, but not offered now: $why.';
    final l = o?.hosts.where((h) => h.id == s.hostId).firstOrNull?.limits;
    final limits = l == null
        ? ''
        : ', up to ${l.maxChildren} at once and ${l.maxTree} in its tree';
    final write = sw.launchedOn
        ? (sw.childrenWrite
              ? ' They write to the same vault as this session.'
              : ' They are read only.')
        : '';
    return 'On. It starts, messages and ends its own agents$limits.$write';
  }
  if (sw.refused == 'launched_without_spawn') return kSpawnAtLaunch;
  return why == null ? 'Off.' : 'Off. $why.';
}

/// Settings › hosts, where the limits are set (153-7 renames it Agents &
/// hosts).
void openSpawnLimits(BuildContext context) =>
    GoRouter.of(context).go(Routes.settingsPage('hosts'));

/// Turns a session's switch on or off, and says why it could not.
Future<void> setSpawnSwitch(
  BuildContext context,
  WidgetRef ref,
  AgentSession s,
  bool enabled,
) async {
  final api = agentApi(ref);
  if (api == null) return;
  final c = ProviderScope.containerOf(context, listen: false);
  try {
    await api.setSpawn(s.id, enabled);
    if (!enabled && context.mounted) {
      _say(context, 'Starting agents is off for ${s.displayName}.');
    }
    if (enabled) c.read(spawnOnAskProvider(s.id).notifier).state = false;
  } catch (e) {
    if (context.mounted) {
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
  }
}

/// Asks for the switch's Allow: in the desk strip, through
/// [spawnOnAskProvider], or [inPlace] where there is no strip.
void askSpawnOn(
  BuildContext context,
  WidgetRef ref,
  AgentSession s, {
  VoidCallback? inPlace,
}) {
  if (context.isExpanded || inPlace == null) {
    ref.read(spawnOnAskProvider(s.id).notifier).state = true;
  } else {
    inPlace();
  }
}

/// The Allow for turning the switch on, with what it allows (4t).
class SpawnOnConfirm extends ConsumerStatefulWidget {
  const SpawnOnConfirm({
    super.key,
    required this.session,
    required this.host,
    required this.onDone,
  });

  final AgentSession session;
  final String host;
  final VoidCallback onDone;

  @override
  ConsumerState<SpawnOnConfirm> createState() => _SpawnOnConfirmState();
}

class _SpawnOnConfirmState extends ConsumerState<SpawnOnConfirm> {
  bool _busy = false;

  Future<void> _allow() async {
    setState(() => _busy = true);
    await setSpawnSwitch(context, ref, widget.session, true);
    if (!mounted) return;
    setState(() => _busy = false);
    if (widget.session.spawn.switchState?.enabled != true) widget.onDone();
  }

  @override
  Widget build(BuildContext context) => _Banner(
    key: const Key('spawn-on-confirm'),
    title: spawnOnConfirmText(widget.session.displayName, widget.host),
    action: 'Allow',
    accent: true,
    onAction: _busy ? null : _allow,
    onCancel: widget.onDone,
  );
}

/// "Starts agents ▾" in the session header (153j): an accent dot while on,
/// and a menu to turn it on or off and to change the limits.
class SpawnChip extends ConsumerStatefulWidget {
  const SpawnChip({super.key, required this.session});

  final AgentSession session;

  @override
  ConsumerState<SpawnChip> createState() => _SpawnChipState();
}

class _SpawnChipState extends ConsumerState<SpawnChip> {
  final _anchor = GlobalKey();

  Future<void> _menu() async {
    final t = context.tokens;
    final s = widget.session;
    final sw = s.spawn.switchState!;
    final canTurnOn = sw.refused == null;
    final picked = await showStormPopover<String>(
      context: context,
      anchorKey: _anchor,
      width: t.sp * 34,
      builder: (popover) => StormPopover(
        key: const Key('spawn-chip-menu'),
        padding: t.sp * 0.75,
        children: [
          if (sw.enabled)
            PopoverItem(
              key: const Key('spawn-chip-toggle'),
              label: 'Turn off',
              onTap: () => Navigator.of(popover).pop('off'),
            )
          else
            PopoverItem(
              key: const Key('spawn-chip-toggle'),
              label: 'Turn on',
              subtitle: canTurnOn
                  ? null
                  : spawnRefusalText(sw.refused!, providerLabel(s.provider)),
              tone: canTurnOn ? PopoverTone.normal : PopoverTone.muted,
              onTap: canTurnOn ? () => Navigator.of(popover).pop('on') : null,
            ),
          PopoverItem(
            key: const Key('spawn-chip-limits'),
            label: 'Change limits in Agents & hosts ›',
            tone: PopoverTone.accent,
            onTap: () => Navigator.of(popover).pop('limits'),
          ),
        ],
      ),
    );
    if (!mounted) return;
    switch (picked) {
      case 'off':
        await setSpawnSwitch(context, ref, s, false);
      case 'on':
        askSpawnOn(context, ref, s);
      case 'limits':
        openSpawnLimits(context);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final on = widget.session.spawn.switchState?.enabled ?? false;
    return Semantics(
      button: true,
      label: 'Starts agents, ${on ? 'on' : 'off'}',
      excludeSemantics: true,
      onTap: _menu,
      child: InkWell(
        key: _anchor,
        onTap: _menu,
        borderRadius: BorderRadius.circular(999),
        child: Container(
          key: const Key('spawn-chip'),
          padding: EdgeInsets.symmetric(
            horizontal: t.sp * 1.125,
            vertical: t.sp * 0.25,
          ),
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(999),
            border: Border.all(color: t.border, width: t.bw),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Container(
                key: const Key('spawn-chip-dot'),
                width: t.sp * 0.75,
                height: t.sp * 0.75,
                decoration: BoxDecoration(
                  color: on ? t.accent : t.text3,
                  shape: BoxShape.circle,
                ),
              ),
              SizedBox(width: t.sp * 0.75),
              Text(
                'Starts agents',
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  fontSize: t.labelSize,
                  color: t.text2,
                  height: 1.3,
                ),
              ),
              SizedBox(width: t.sp * 0.5),
              Icon(LucideIcons.chevron_down, size: t.labelSize, color: t.text3),
            ],
          ),
        ),
      ),
    );
  }
}

/// About's Starts agents (153j): the state and limits, then Turn on or off
/// and the limits' page. On a phone its Allow asks in place.
class StartsAgentsAbout extends ConsumerStatefulWidget {
  const StartsAgentsAbout({
    super.key,
    required this.session,
    required this.host,
  });

  final AgentSession session;
  final String host;

  @override
  ConsumerState<StartsAgentsAbout> createState() => _StartsAgentsAboutState();
}

class _StartsAgentsAboutState extends ConsumerState<StartsAgentsAbout> {
  bool _confirm = false;
  bool _busy = false;
  final _confirmKey = GlobalKey();

  /// Its Allow sits below the row, often past the bottom of the sheet.
  void _askHere() {
    setState(() => _confirm = true);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final at = _confirmKey.currentContext;
      if (at == null || !at.mounted) return;
      Scrollable.ensureVisible(
        at,
        duration: const Duration(milliseconds: 200),
        alignmentPolicy: ScrollPositionAlignmentPolicy.keepVisibleAtEnd,
      );
    });
  }

  Future<void> _off() async {
    setState(() => _busy = true);
    await setSpawnSwitch(context, ref, widget.session, false);
    if (mounted) setState(() => _busy = false);
  }

  @override
  Widget build(BuildContext context) {
    final s = widget.session;
    final sw = s.spawn.switchState;
    if (sw == null) return const SizedBox.shrink();
    final t = context.tokens;
    final live = !s.ended;
    final canTurnOn = sw.refused == null;
    final o = ref.watch(agentOverviewProvider).value;
    return Column(
      key: const Key('about-starts-agents'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          startsAgentsText(s, o),
          key: const Key('spawn-switch-note'),
          style: agentSans(t),
        ),
        SizedBox(height: t.sp * 0.5),
        Wrap(
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            if (live && sw.enabled)
              AccentLink(
                key: const Key('spawn-switch'),
                label: 'Turn off',
                onTap: _busy ? null : _off,
              )
            else if (live && canTurnOn)
              AccentLink(
                key: const Key('spawn-switch'),
                label: 'Turn on',
                onTap: () => askSpawnOn(context, ref, s, inPlace: _askHere),
              ),
            if (live && (sw.enabled || canTurnOn))
              Text(' · ', style: agentSans(t, color: t.text3)),
            AccentLink(
              key: const Key('spawn-limits'),
              label: 'Change limits in Agents & hosts ›',
              onTap: () => openSpawnLimits(context),
            ),
          ],
        ),
        if (_confirm && !sw.enabled && live) ...[
          SizedBox(height: t.sp),
          KeyedSubtree(
            key: _confirmKey,
            child: SpawnOnConfirm(
              session: s,
              host: widget.host,
              onDone: () => setState(() => _confirm = false),
            ),
          ),
        ],
      ],
    );
  }
}

/// An accent text link in About and the Agents tab; [danger] for End all.
class AccentLink extends StatelessWidget {
  const AccentLink({
    super.key,
    required this.label,
    required this.onTap,
    this.danger = false,
  });

  final String label;
  final VoidCallback? onTap;
  final bool danger;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Semantics(
      button: true,
      enabled: onTap != null,
      label: label,
      excludeSemantics: true,
      onTap: onTap,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(t.rControl),
        child: Padding(
          padding: EdgeInsets.symmetric(vertical: t.sp * 0.25),
          child: Text(
            label,
            style: agentSans(
              t,
              color: onTap == null
                  ? t.text3
                  : danger
                  ? t.danger
                  : t.accent,
            ),
          ),
        ),
      ),
    );
  }
}

// ---- banners --------------------------------------------------------------

/// The inline confirmation for turning the spawn switch on (4t). Ending
/// and dismissing ask in the decision strip (153g).
class _Banner extends StatelessWidget {
  const _Banner({
    super.key,
    required this.title,
    required this.action,
    required this.onAction,
    required this.onCancel,
    this.accent = false,
  });

  final String title;
  final String action;
  final VoidCallback? onAction;
  final VoidCallback onCancel;
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
          SizedBox(height: t.sp * 1.25),
          Wrap(
            alignment: WrapAlignment.end,
            crossAxisAlignment: WrapCrossAlignment.center,
            spacing: t.sp,
            runSpacing: t.sp * 0.5,
            children: [
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
const kReportFirst = 'Let them report first (waits 30 s)';

/// What a session's strip is confirming (153g): the person asked for it,
/// so it shows in place of a waiting request until it is answered.
enum SessionConfirm { endTree, endChildren, dismissTree }

/// The confirmation open on a session's desk strip, if any. Device-local.
final sessionConfirmProvider = StateProvider.autoDispose
    .family<SessionConfirm?, String>((ref, id) => null);

/// "spike-notes (2 commits not applied stay in its Work tab until you
/// dismiss it)": an ended child's work the person has not taken (153d).
String endedWithWork(String name, AgentSession? s) {
  if (s == null || s.spawn.workToTake != true) return name;
  final what = switch (s.spawn.unappliedCommits) {
    1 => '1 commit not applied stays',
    final n? when n > 1 => '$n commits not applied stay',
    _ => 'work not applied stays',
  };
  return '$name ($what in its Work tab until you dismiss it)';
}

/// Ending a parent ends its children, after a person saw which (C1, 4p), as
/// a confirmation strip (153g). [children] true is End all children (C5):
/// the parent keeps running.
class EndTreeStrip extends ConsumerStatefulWidget {
  const EndTreeStrip({
    super.key,
    required this.session,
    required this.onDone,
    this.children = false,
    this.layout = DecisionLayout.strip,
  });

  final AgentSession session;
  final VoidCallback onDone;
  final bool children;
  final DecisionLayout layout;

  @override
  ConsumerState<EndTreeStrip> createState() => _EndTreeStripState();
}

class _EndTreeStripState extends ConsumerState<EndTreeStrip> {
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
    final o = ref.watch(agentOverviewProvider).value;
    final running = [
      for (final r in d?.running ?? const <DescendantRef>[]) r.name,
    ];
    final ended = [
      for (final r in d?.ended ?? const <DescendantRef>[])
        endedWithWork(r.name, o?.byId(r.id)),
    ];
    final n = running.length + (widget.children ? 0 : 1);
    final agents = running.length == 1 ? '1 agent' : '${running.length} agents';
    return DecisionStrip(
      key: Key(widget.children ? 'end-children-confirm' : 'end-tree-confirm'),
      label: 'CONFIRM',
      layout: widget.layout,
      title: widget.children
          ? 'End the $agents ${s.displayName} started? '
                '${s.displayName} keeps running.'
          : 'End ${s.displayName} and the $agents it started?',
      rows: [
        if (running.isNotEmpty) DecisionRow('Running', running.join(' · ')),
        if (ended.isNotEmpty) DecisionRow('Already ended', ended.join(' · ')),
      ],
      tick: running.isEmpty
          ? null
          : (
              value: _reportFirst,
              onChanged: (v) => setState(() => _reportFirst = v),
              label: kReportFirst,
            ),
      onEscape: widget.onDone,
      actions: [
        DecisionAction(
          key: const Key('banner-cancel'),
          label: 'Cancel',
          kind: DecisionActionKind.text,
          onPressed: widget.onDone,
        ),
        DecisionAction(
          key: const Key('banner-action'),
          label: 'End $n ${n == 1 ? 'session' : 'sessions'}',
          kind: DecisionActionKind.danger,
          onPressed: d == null || _busy || n == 0 ? null : () => _go(d),
        ),
      ],
    );
  }
}

/// The warning a tree's dismissal carries (C8 with 137g).
const kTreeDismissWarning =
    'Each child’s own copy or scratch folder is deleted with it, and any of '
    'its work you haven’t applied goes too.';

/// A tree is dismissed only as a unit, once every member has ended (C8), as
/// a confirmation strip (153g).
class DismissTreeStrip extends ConsumerStatefulWidget {
  const DismissTreeStrip({
    super.key,
    required this.session,
    required this.onDone,
    required this.onDismissed,
    this.layout = DecisionLayout.strip,
  });

  final AgentSession session;
  final VoidCallback onDone;
  final VoidCallback onDismissed;
  final DecisionLayout layout;

  @override
  ConsumerState<DismissTreeStrip> createState() => _DismissTreeStripState();
}

class _DismissTreeStripState extends ConsumerState<DismissTreeStrip> {
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
    return DecisionStrip(
      key: const Key('dismiss-tree-confirm'),
      label: 'CONFIRM',
      layout: widget.layout,
      title: live.isEmpty
          ? 'Dismiss ${root.displayName} and everything it started? '
                '$n sessions go together.'
          : 'Dismiss ${root.displayName}’s sessions together, once every one '
                'has ended.',
      rows: [
        DecisionRow(
          'Dismisses',
          [for (final m in members) m.displayName].join(' · '),
        ),
        if (live.isNotEmpty) DecisionRow('Still running', live.join(' · ')),
      ],
      note: live.isEmpty ? kTreeDismissWarning : null,
      onEscape: widget.onDone,
      actions: [
        DecisionAction(
          key: const Key('banner-cancel'),
          label: 'Cancel',
          kind: DecisionActionKind.text,
          onPressed: widget.onDone,
        ),
        if (live.isEmpty)
          DecisionAction(
            key: const Key('banner-action'),
            label: 'Dismiss $n sessions',
            kind: DecisionActionKind.danger,
            onPressed: _busy ? null : _go,
          ),
      ],
    );
  }
}

// ---- what the agent did ------------------------------------------------

/// What the act carried, exactly as the agent sent it: a start's task, a
/// message's text.
String? actText(AgentAct r) => switch (r.kind) {
  'start' => r.task,
  'message' => r.text,
  _ => null,
};

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

// ---- About ----------------------------------------------------------------

/// The session header's meta line, with a child's own workspace named for
/// what it is rather than its record's `children/<id>`.
String spawnMeta(String meta, AgentSession s, AgentOverview? o) {
  final parent = s.spawn.parentSession;
  if (parent == null) return meta;
  final ws = o?.byId(parent)?.workspace ?? 'its parent’s workspace';
  return meta.replaceFirst(s.workspace, childWorkspaceLabel(s, parent: ws));
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

import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart' show clockAt;

/// The Agents sidebar's rules (decision 153e, handoff §2 Sidebar Rules A):
/// which tier a session is in, the order rows take from it, and the words
/// a row's sub-line says. Pure, so every rule is tested without a widget.

/// Most urgent first. Only the first four move a row (§2 Sorting).
enum SidebarTier {
  needsYou,
  blocked,
  failed,
  unknown,
  outOfUsage,
  working,
  idle,
  parked,
  rest;

  bool get urgent => index <= SidebarTier.unknown.index;
}

/// What waits for the person to answer: a permission prompt, a question or
/// a delete approval. The only state outlined, and counted on the rail.
const kNeedsYouKinds = {
  'permission_prompt',
  'delete_approval',
  'approval',
  'question',
};

/// How long a failed session stays in the failed tier after it ended.
const kFailedUrgentFor = Duration(hours: 1);

/// Why a live session waits, from the reasoned attention or, on an older
/// server, the bare kind.
String? attentionKindOf(AgentSession s) =>
    s.ended ? null : (s.spawn.attention?.kind ?? s.spawn.needsAttention);

bool needsYou(AgentSession s) {
  if (s.ended) return false;
  return kNeedsYouKinds.contains(attentionKindOf(s)) ||
      (s.deleteRequest?.pending ?? false) ||
      s.pendingDecisions.isNotEmpty;
}

DateTime? _at(String? iso) => iso == null ? null : DateTime.tryParse(iso);

SidebarTier tierOf(AgentSession s, DateTime now) {
  if (needsYou(s)) return SidebarTier.needsYou;
  final kind = attentionKindOf(s);
  if (kind == 'blocked') return SidebarTier.blocked;
  if (s.status == 'failed') {
    final ended = _at(s.endedAt) ?? _at(s.startedAt) ?? _at(s.createdAt);
    final recent = ended != null && now.difference(ended) < kFailedUrgentFor;
    return recent ? SidebarTier.failed : SidebarTier.rest;
  }
  if (s.status == 'unknown') return SidebarTier.unknown;
  if (kind == 'usage_limit') return SidebarTier.outOfUsage;
  return switch (s.status) {
    'running' when s.working => SidebarTier.working,
    'running' || 'creating' || 'starting' => SidebarTier.idle,
    'parked' => SidebarTier.parked,
    _ => SidebarTier.rest,
  };
}

/// Every session's tier, as the sidebar holds them while the pointer is in
/// it: rows keep the place they had when it came in.
Map<String, SidebarTier> tiersOf(
  Iterable<AgentSession> sessions,
  DateTime now,
) => {for (final s in sessions) s.id: tierOf(s, now)};

typedef TierOf = SidebarTier Function(AgentSession s);

/// [now]'s tiers, except where [held] already has one.
TierOf tierLookup(DateTime now, [Map<String, SidebarTier>? held]) =>
    (s) => held?[s.id] ?? tierOf(s, now);

/// The sort key: an urgent tier's index, or one key for every other tier,
/// so below the top four the person's own order holds.
int _rank(SidebarTier t) => t.urgent ? t.index : SidebarTier.values.length;

/// The most urgent tier of [sessions], or [SidebarTier.rest] for none.
SidebarTier worstTier(Iterable<AgentSession> sessions, TierOf tier) {
  var worst = SidebarTier.rest;
  for (final s in sessions) {
    final t = tier(s);
    if (t.index < worst.index) worst = t;
  }
  return worst;
}

/// [items] with the urgent first by tier; the rest, and ties, keep their
/// order.
List<T> _urgentFirst<T>(List<T> items, SidebarTier Function(T) tier) {
  final keyed = [for (final (i, x) in items.indexed) (i, _rank(tier(x)), x)];
  keyed.sort((a, b) => a.$2 != b.$2 ? a.$2 - b.$2 : a.$1 - b.$1);
  return [for (final k in keyed) k.$3];
}

/// [list] with each child after its parent and its siblings sorted, and
/// trees sorted among themselves by their most urgent member; a tree moves
/// as one. A child whose parent is not in [list] is a tree of its own.
List<AgentSession> sidebarOrder(List<AgentSession> list, TierOf tier) {
  final ids = {for (final s in list) s.id};
  final under = <String, List<AgentSession>>{};
  final tops = <AgentSession>[];
  for (final s in list) {
    final p = s.spawn.parentSession;
    if (p != null && p != s.id && ids.contains(p)) {
      (under[p] ??= []).add(s);
    } else {
      tops.add(s);
    }
  }
  final worst = <String, SidebarTier>{};
  SidebarTier treeTier(AgentSession s, Set<String> seen) {
    if (worst[s.id] case final w?) return w;
    var w = tier(s);
    if (seen.add(s.id)) {
      for (final c in under[s.id] ?? const <AgentSession>[]) {
        final cw = treeTier(c, seen);
        if (cw.index < w.index) w = cw;
      }
    }
    return worst[s.id] = w;
  }

  final out = <AgentSession>[];
  final placed = <String>{};
  void walk(AgentSession s) {
    if (!placed.add(s.id)) return;
    out.add(s);
    final kids = under[s.id] ?? const <AgentSession>[];
    for (final c in _urgentFirst(kids, (k) => treeTier(k, {}))) {
      walk(c);
    }
  }

  for (final s in _urgentFirst(tops, (t) => treeTier(t, {}))) {
    walk(s);
  }
  return out;
}

/// Crews by their most urgent member, otherwise in the person's order.
List<Crew> crewOrder(
  List<Crew> crews,
  List<AgentSession> Function(Crew c) members,
  TierOf tier,
) => _urgentFirst(crews, (c) => worstTier(members(c), tier));

/// Every session under [id] in [list], at any depth.
List<AgentSession> descendantsIn(List<AgentSession> list, String id) {
  final out = <AgentSession>[];
  final seen = {id};
  var frontier = {id};
  while (frontier.isNotEmpty) {
    final next = <String>{};
    for (final s in list) {
      final p = s.spawn.parentSession;
      if (p != null && frontier.contains(p) && seen.add(s.id)) {
        out.add(s);
        next.add(s.id);
      }
    }
    frontier = next;
  }
  return out;
}

/// [order] less the rows under a collapsed tree.
List<AgentSession> withoutCollapsed(
  List<AgentSession> order,
  Set<String> collapsed,
) {
  if (collapsed.isEmpty) return order;
  final hidden = <String>{};
  for (final id in collapsed) {
    if (order.any((s) => s.id == id)) {
      hidden.addAll(descendantsIn(order, id).map((s) => s.id));
    }
  }
  return [
    for (final s in order)
      if (!hidden.contains(s.id)) s,
  ];
}

/// A folded crew's or tree's most urgent state: "1 needs you", "2 failed".
class RollUp {
  const RollUp(this.tier, this.count);

  final SidebarTier tier;
  final int count;

  /// Outlined like the row that needs you.
  bool get outlined => tier == SidebarTier.needsYou;

  /// In `danger`: what needs you, or failed.
  bool get loud => tier == SidebarTier.needsYou || tier == SidebarTier.failed;

  String get words =>
      '$count ${switch (tier) {
        SidebarTier.needsYou => 'needs you',
        SidebarTier.blocked => 'blocked',
        SidebarTier.failed => 'failed',
        SidebarTier.unknown => 'host offline',
        _ => 'out of usage',
      }}';
}

/// The roll-up of [sessions], or null when none of them waits or failed.
RollUp? rollUpOf(Iterable<AgentSession> sessions, TierOf tier) {
  final worst = worstTier(sessions, tier);
  if (!worst.urgent && worst != SidebarTier.outOfUsage) return null;
  return RollUp(worst, sessions.where((s) => tier(s) == worst).length);
}

/// How a sub-line is coloured.
enum SubTone {
  /// `text3`: the plain "workspace · age".
  plain,

  /// `text`: something waits for you.
  loud,

  /// `text2`: blocked, out of usage, host offline.
  quiet,

  /// `danger`: failed, or a folded tree with someone waiting.
  danger,

  /// `accent`: work to apply.
  accent,
}

class SubLine {
  const SubLine(this.text, this.tone);

  final String text;
  final SubTone tone;

  @override
  bool operator ==(Object other) =>
      other is SubLine && other.text == text && other.tone == tone;

  @override
  int get hashCode => Object.hash(text, tone);

  @override
  String toString() => 'SubLine($text, $tone)';
}

/// "now", "3m", "2h", "1d".
String agoWords(DateTime then, DateTime now) {
  final d = now.difference(then);
  if (d.inMinutes < 1) return 'now';
  if (d.inHours < 1) return '${d.inMinutes}m';
  if (d.inDays < 1) return '${d.inHours}h';
  return '${d.inDays}d';
}

String _needsYouWords(AgentSession s) {
  final kind = attentionKindOf(s);
  return switch (kind) {
    'permission_prompt' => 'permission prompt',
    'delete_approval' => 'delete request',
    'question' => 'question',
    'approval' => 'approval',
    _ when s.deleteRequest?.pending ?? false => 'delete request',
    _ => switch (s.pendingDecisions.firstOrNull?.kind) {
      'delete_request' => 'delete request',
      _ => 'decision',
    },
  };
}

/// "commits to apply": an ended child whose work waits to be applied or
/// deleted (153d). Null when there is none.
String? workToApply(AgentSession s) {
  if (!s.ended || s.spawn.workToTake != true) return null;
  return switch (s.spawn.unappliedCommits) {
    1 => '1 commit to apply',
    final n? when n > 1 => '$n commits to apply',
    _ => 'work to apply',
  };
}

/// Why, in words (§2 Sub-line), or null for the plain "workspace · age".
/// [tier] is the row's tier; [o] names whoever it is blocked on.
SubLine? reasonLine(
  AgentSession s, {
  required DateTime now,
  required SidebarTier tier,
  AgentOverview? o,
}) {
  final a = s.spawn.attention;
  String withAge(String words, DateTime? since) =>
      since == null ? words : '$words · ${agoWords(since, now)}';
  switch (tier) {
    case SidebarTier.needsYou:
      final since = a?.since ?? s.pendingDecisions.firstOrNull?.createdAt;
      return SubLine(withAge(_needsYouWords(s), since), SubTone.loud);
    case SidebarTier.blocked:
      final on = a?.blockedOn == null ? null : o?.byId(a!.blockedOn!);
      final detail = on != null ? 'waiting on ${on.displayName}' : a?.detail;
      final words = detail == null || detail.trim().isEmpty
          ? 'blocked'
          : 'blocked · ${detail.trim()}';
      return SubLine(words, SubTone.quiet);
    case SidebarTier.unknown:
      final seen = s.hostLastSeen;
      return SubLine(
        seen == null
            ? 'host offline'
            : 'host offline · last seen ${agoWords(seen, now)}',
        SubTone.quiet,
      );
    case SidebarTier.outOfUsage:
      final resets = a?.resetsAt;
      return SubLine(
        resets == null
            ? 'out of usage'
            : 'out of usage · resets ${clockAt(resets)}',
        SubTone.quiet,
      );
    case SidebarTier.parked:
      return const SubLine('parked until its next message', SubTone.plain);
    case SidebarTier.failed || SidebarTier.rest when s.status == 'failed':
      final ended = _at(s.endedAt);
      final work = workToApply(s);
      final words = [
        'failed',
        ?s.endDetail,
        if (work == null && ended != null) agoWords(ended, now),
        ?work,
      ].join(' · ');
      return SubLine(
        words,
        tier == SidebarTier.failed
            ? SubTone.danger
            : (work != null ? SubTone.accent : SubTone.plain),
      );
    default:
      if (workToApply(s) case final work?) {
        return SubLine(
          '${s.statusLabel.toLowerCase()} · $work',
          SubTone.accent,
        );
      }
      return null;
  }
}

/// The parent a child is listed apart from: in another crew or section,
/// where it is not indented under it. Null when it is, or is no child.
AgentSession? parentApart(
  AgentSession s,
  List<AgentSession> listed,
  AgentOverview? o,
) {
  final p = s.spawn.parentSession;
  if (p == null || listed.any((m) => m.id == p)) return null;
  return o?.byId(p);
}

/// Whether rows name their host: only when sessions run on more than one.
bool namesHosts(AgentOverview? o) =>
    o != null && {for (final s in o.sessions) s.hostId}.length > 1;

/// How many sessions wait for the person: the rail's count (153e), the
/// only place it is counted on a desk.
final needsYouCountProvider = Provider<int>(
  (ref) => ref.watch(
    agentOverviewProvider.select(
      (o) => o.value?.sessions.where(needsYou).length ?? 0,
    ),
  ),
);

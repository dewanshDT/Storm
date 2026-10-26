import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'agent_models.dart';
import 'agent_state.dart';
import 'spawn_api.dart';
import 'spawn_models.dart';

/// How often a session with a waiting request re-reads its requests. Status
/// events carry no `needs_attention` (134-9), so the client re-reads.
const spawnRequestsRefresh = Duration(seconds: 4);

/// What a session asked a person for, newest first; null while unknown or
/// unreachable. Re-read while one waits.
final sessionRequestsProvider = FutureProvider.autoDispose
    .family<List<SpawnRequest>?, String>((ref, id) async {
      // A new request shows on the session's view first (134-9), which is
      // re-read while it is live: read the list again when that moves.
      await ref.watch(
        sessionDetailProvider(id).selectAsync((s) => s?.spawn.needsAttention),
      );
      final api = ref.watch(agentApiFactoryProvider)?.call();
      if (api == null) return null;
      try {
        final list = await api.requests(id);
        if (list.any((r) => r.waiting(DateTime.now()))) {
          refreshProviderEvery(ref, spawnRequestsRefresh);
        }
        return list;
      } catch (_) {
        return null;
      } finally {
        api.dispose();
      }
    });

/// The descendants ending a session would stop, running apart from ended.
final sessionDescendantsProvider = FutureProvider.autoDispose
    .family<Descendants?, String>((ref, id) async {
      final api = ref.watch(agentApiFactoryProvider)?.call();
      if (api == null) return null;
      try {
        return await api.descendants(id);
      } catch (_) {
        return null;
      } finally {
        api.dispose();
      }
    });

/// An ended child's work, read once; a refusal is the error.
final childWorkProvider = FutureProvider.autoDispose.family<ChildWork, String>((
  ref,
  id,
) async {
  final api = ref.watch(agentApiFactoryProvider)?.call();
  if (api == null) throw StateError('signed out');
  try {
    return await api.childWork(id);
  } finally {
    api.dispose();
  }
});

/// After a person decides, switches or ends: everything that shows it.
void spawnChanged(WidgetRef ref, String sessionId) {
  ref.invalidate(sessionRequestsProvider(sessionId));
  ref.invalidate(sessionDetailProvider(sessionId));
  ref.invalidate(sessionDescendantsProvider(sessionId));
  ref.invalidate(agentOverviewProvider);
}

/// A session's tree as the overview knows it: its root and every session
/// under that root, the root first, then nearest first.
List<AgentSession> treeOf(AgentOverview o, AgentSession s) {
  final root = s.spawn.rootSession ?? s.id;
  final members = [
    for (final m in o.sessions)
      if ((m.spawn.rootSession ?? m.id) == root) m,
  ]..sort((a, b) => a.spawn.depth.compareTo(b.spawn.depth));
  return members;
}

/// Its own children, as the overview knows them.
List<AgentSession> childrenOf(AgentOverview o, String id) => [
  for (final m in o.sessions)
    if (m.spawn.parentSession == id) m,
];

/// Sessions with children directly after their parent, depth first, in the
/// list's own order otherwise. A child whose parent is not in [list] keeps
/// its place.
List<AgentSession> lineageOrder(List<AgentSession> list) {
  final ids = {for (final s in list) s.id};
  final under = <String, List<AgentSession>>{};
  final tops = <AgentSession>[];
  for (final s in list) {
    final p = s.spawn.parentSession;
    if (p != null && ids.contains(p)) {
      (under[p] ??= []).add(s);
    } else {
      tops.add(s);
    }
  }
  final out = <AgentSession>[];
  void walk(AgentSession s) {
    out.add(s);
    for (final c in under[s.id] ?? const <AgentSession>[]) {
      walk(c);
    }
  }

  tops.forEach(walk);
  return out;
}

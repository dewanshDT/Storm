import 'dart:convert';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../agent/agent_state.dart' show agentOverviewProvider;
import '../api/models.dart';
import 'app_state.dart';

/// Every session's `wrote_count` added up, from the overview the rail and
/// the bubbles already poll: when it moves, an agent wrote something.
final agentWroteSignalProvider = Provider<int>(
  (ref) => ref.watch(
    agentOverviewProvider.select(
      (o) => o.value?.sessions.fold<int>(0, (n, s) => n + s.wroteCount) ?? 0,
    ),
  ),
);

/// Note id → its latest agent write in [vaultId], re-read with the vault's
/// sync and whenever a session writes. Empty when the server cannot say.
final agentWritesProvider = FutureProvider.autoDispose
    .family<Map<String, LatestAgentWrite>, String>((ref, vaultId) async {
      ref.watch(vaultRevisionProvider);
      ref.watch(agentWroteSignalProvider);
      final api = ref.watch(apiProvider);
      if (api == null || vaultId.isEmpty) return const {};
      try {
        return await api.agentWrites(vaultId);
      } catch (_) {
        return const {};
      }
    });

/// The newest version of each note this device has opened, as
/// `{"<vault>/<note>": version}`. Device-local by design: what one device
/// has seen says nothing about another.
class SeenVersions extends AsyncNotifier<Map<String, int>> {
  static const key = 'storm.seen';

  /// Enough for a long reading history; the oldest opens fall off first.
  static const limit = 2000;

  @override
  Future<Map<String, int>> build() async {
    try {
      final raw = (await SharedPreferences.getInstance()).getString(key);
      if (raw == null) return {};
      final json = jsonDecode(raw) as Map<String, dynamic>;
      return {for (final e in json.entries) e.key: (e.value as num).toInt()};
    } catch (_) {
      return {};
    }
  }

  /// Records that [version] of a note is on screen here. Never goes back.
  Future<void> markSeen(String vaultId, String noteId, int version) async {
    final current = state.value ?? await future;
    final id = '$vaultId/$noteId';
    if ((current[id] ?? 0) >= version) return;
    final next = Map<String, int>.of(current)
      ..remove(id)
      ..[id] = version;
    while (next.length > limit) {
      next.remove(next.keys.first);
    }
    state = AsyncData(next);
    try {
      await (await SharedPreferences.getInstance()).setString(
        key,
        jsonEncode(next),
      );
    } catch (_) {
      // No preferences store (tests, previews): memory still works.
    }
  }
}

final seenVersionsProvider =
    AsyncNotifierProvider<SeenVersions, Map<String, int>>(SeenVersions.new);

/// The notes of [vaultId] an agent changed since this device last opened
/// them: latest agent write version above the last opened one.
final unseenNotesProvider = Provider.autoDispose.family<Set<String>, String>((
  ref,
  vaultId,
) {
  final seen = ref.watch(seenVersionsProvider).value;
  if (seen == null) return const {};
  final writes = ref.watch(agentWritesProvider(vaultId)).value ?? const {};
  return {
    for (final e in writes.entries)
      if (e.value.version > (seen['$vaultId/${e.key}'] ?? 0)) e.key,
  };
});

/// A note's provenance (`agent_write`), fetched only for a note an agent
/// wrote, and again only when its latest agent write moves.
final noteProvenanceProvider = FutureProvider.autoDispose
    .family<AgentWrite?, ({String vaultId, String noteId})>((ref, at) async {
      final latest = ref.watch(
        agentWritesProvider(
          at.vaultId,
        ).select((w) => w.value?[at.noteId]?.version),
      );
      if (latest == null) return null;
      final api = ref.watch(apiProvider);
      if (api == null) return null;
      try {
        return (await api.note(at.vaultId, at.noteId)).agentWrite;
      } catch (_) {
        return null;
      }
    });

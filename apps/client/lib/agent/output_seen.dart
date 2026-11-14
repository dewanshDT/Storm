import 'dart:async';
import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'agent_models.dart';
import 'agent_state.dart';

/// The output end (153c) each session had when this device last showed it,
/// as `{"<session>": end}`. Device-local, like the notes' seen versions:
/// what the laptop has read says nothing about the phone.
///
/// The `""` key marks a device that has baselined: the sessions it found on
/// its first look count as read, so a new device does not dot every one.
class SeenOutput extends AsyncNotifier<Map<String, int>> {
  static const key = 'storm.agents.seen_output';

  /// Enough for a long history; the oldest fall off first.
  static const limit = 500;

  static const marker = '';

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

  /// [end] of [sessionId]'s output is on screen here. Never goes back.
  Future<void> markSeen(String sessionId, int end) async {
    final current = state.value ?? await future;
    if ((current[sessionId] ?? -1) >= end) return;
    final next = Map<String, int>.of(current)
      ..remove(sessionId)
      ..[sessionId] = end;
    await _save(next);
  }

  /// On this device's first look, takes [ends] as read.
  Future<void> baseline(Map<String, int> ends) async {
    final current = state.value ?? await future;
    if (current.containsKey(marker)) return;
    final next = Map<String, int>.of(current)..[marker] = 1;
    for (final e in ends.entries) {
      if ((next[e.key] ?? -1) < e.value) next[e.key] = e.value;
    }
    await _save(next);
  }

  Future<void> _save(Map<String, int> next) async {
    final ids = next.keys.where((k) => k != marker).toList();
    ids.take(math.max(0, ids.length - limit)).forEach(next.remove);
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

final seenOutputProvider = AsyncNotifierProvider<SeenOutput, Map<String, int>>(
  SeenOutput.new,
);

/// Whether [s] has output past what [seen] says this device last showed.
/// Nothing is unread before the device has baselined, and a session with no
/// output end (an older server, or nothing yet) never is.
bool outputUnread(Map<String, int>? seen, AgentSession s) {
  if (seen == null || !seen.containsKey(SeenOutput.marker)) return false;
  final end = s.outputEnd;
  return end != null && end > (seen[s.id] ?? 0);
}

/// Sessions with output past what this device last showed ([outputUnread]).
final unreadSessionsProvider = Provider<Set<String>>((ref) {
  final seen = ref.watch(seenOutputProvider).value;
  final o = ref.watch(agentOverviewProvider).value;
  if (seen == null || o == null || o.unreachable) return const {};
  if (!seen.containsKey(SeenOutput.marker)) {
    final notifier = ref.read(seenOutputProvider.notifier);
    final ends = {for (final s in o.sessions) s.id: ?s.outputEnd};
    scheduleMicrotask(() => notifier.baseline(ends));
    return const {};
  }
  return {
    for (final s in o.sessions)
      if (outputUnread(seen, s)) s.id,
  };
});

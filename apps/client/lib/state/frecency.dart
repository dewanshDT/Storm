import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// How often and how lately each palette item was used, on this device only
/// (decision 156i, 153q): never synced, so the laptop and the phone learn
/// separately.
///
/// Each item keeps one number, a projected timestamp: the moment a single
/// use would have to happen to score what all its uses score now, with a
/// use's weight halving every [halfLife]. A higher number is a stronger
/// item, comparing two needs no clock, and a new use folds in without
/// keeping a history.
class Frecency {
  const Frecency([this._at = const {}]);

  final Map<String, double> _at;

  static const halfLife = Duration(days: 30);

  /// What a pick from the palette counts, against a plain open's 1.
  static const pickWeight = 2.0;

  static const _keep = 400;

  /// [id]'s projected timestamp in ms, or null when it was never used.
  double? of(String id) => _at[id];

  /// Higher first; never-used items tie.
  int compare(String a, String b) {
    final x = _at[a];
    final y = _at[b];
    if (x == y) return 0;
    if (x == null) return 1;
    if (y == null) return -1;
    return y.compareTo(x);
  }

  Frecency used(String id, {double weight = 1, DateTime? now}) {
    final h = halfLife.inMilliseconds.toDouble();
    final t =
        (now ?? DateTime.now()).millisecondsSinceEpoch +
        h * math.log(weight) / math.ln2;
    final old = _at[id];
    // log2(2^(old/h) + 2^(t/h)) * h, kept in range by factoring out the
    // larger term.
    final next = old == null
        ? t
        : math.max(old, t) +
              h * math.log(1 + math.pow(2, -(old - t).abs() / h)) / math.ln2;
    final out = {..._at, id: next};
    if (out.length > _keep) {
      final weakest = out.entries.reduce((a, b) => a.value <= b.value ? a : b);
      out.remove(weakest.key);
    }
    return Frecency(out);
  }

  static const _key = 'storm.palette.frecency';

  static Frecency read(SharedPreferences prefs) {
    try {
      final raw = prefs.getString(_key);
      if (raw == null) return const Frecency();
      final map = jsonDecode(raw) as Map<String, dynamic>;
      return Frecency({
        for (final e in map.entries) e.key: (e.value as num).toDouble(),
      });
    } catch (_) {
      return const Frecency();
    }
  }

  Future<void> write(SharedPreferences prefs) =>
      prefs.setString(_key, jsonEncode(_at));
}

class FrecencyNotifier extends AsyncNotifier<Frecency> {
  @override
  Future<Frecency> build() async {
    try {
      return Frecency.read(await SharedPreferences.getInstance());
    } catch (_) {
      return const Frecency();
    }
  }

  /// A palette pick, which weighs more than [opened].
  Future<void> picked(String id) => _use(id, Frecency.pickWeight);

  /// The item was opened some other way: a link, the sidebar, a recent.
  Future<void> opened(String id) => _use(id, 1);

  Future<void> _use(String id, double weight) async {
    final now = (state.value ?? await future).used(id, weight: weight);
    state = AsyncData(now);
    try {
      await now.write(await SharedPreferences.getInstance());
    } catch (_) {
      // Ranking is a convenience; a device that can't keep it ranks without.
    }
  }
}

final frecencyProvider = AsyncNotifierProvider<FrecencyNotifier, Frecency>(
  FrecencyNotifier.new,
);

/// The palette item a location shows, for counting plain opens: a note, a
/// session or a Settings page. Null for anywhere else.
String? frecencyIdOf(String location) {
  final seg = Uri.parse(location).pathSegments;
  if (seg.length >= 4 && seg[0] == 'v' && seg[2] == 'note') {
    return noteItemId(seg[1], seg[3]);
  }
  if (seg.length >= 3 && seg[0] == 'agents' && seg[1] == 's') {
    return 'session:${seg[2]}';
  }
  if (seg.length == 2 && seg[0] == 'settings') return 'settings:${seg[1]}';
  return null;
}

String noteItemId(String vaultId, String noteId) => 'note:$vaultId/$noteId';

import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// Which crews, spawned trees and sections are collapsed on this device
/// (decision 138m, 153e). Per device and kept across restarts; never sent to the
/// server, so the laptop and the phone fold differently.
@immutable
class CrewPrefs {
  const CrewPrefs({
    this.section = false,
    this.crews = const {},
    this.trees = const {},
    this.ended = true,
  });

  /// The CREWS section is folded to "CREWS · n".
  final bool section;

  /// Collapsed crews, by id.
  final Set<String> crews;

  /// Folded trees, by their parent's id: the parent's row rolls them up.
  final Set<String> trees;

  /// The sidebar's ENDED is folded to its count and Dismiss all: folded
  /// until the person opens it (§2: "ENDED stays collapsed").
  final bool ended;

  bool collapsed(String crewId) => crews.contains(crewId);

  bool treeCollapsed(String sessionId) => trees.contains(sessionId);

  CrewPrefs copyWith({
    bool? section,
    Set<String>? crews,
    Set<String>? trees,
    bool? ended,
  }) => CrewPrefs(
    section: section ?? this.section,
    crews: crews ?? this.crews,
    trees: trees ?? this.trees,
    ended: ended ?? this.ended,
  );

  static const _kSection = 'storm.crews.collapsed.section';
  static const _kCrews = 'storm.crews.collapsed.crews';
  static const _kTrees = 'storm.agents.collapsed.trees';
  static const _kEnded = 'storm.agents.collapsed.ended';

  static CrewPrefs read(SharedPreferences prefs) => CrewPrefs(
    section: prefs.getBool(_kSection) ?? false,
    crews: {...?prefs.getStringList(_kCrews)},
    trees: {...?prefs.getStringList(_kTrees)},
    ended: prefs.getBool(_kEnded) ?? true,
  );

  Future<void> write(SharedPreferences prefs) async {
    await prefs.setBool(_kSection, section);
    await prefs.setStringList(_kCrews, crews.toList()..sort());
    await prefs.setStringList(_kTrees, trees.toList()..sort());
    await prefs.setBool(_kEnded, ended);
  }
}

/// Loaded before the first crew is drawn, so a collapsed crew never opens
/// for a frame on launch.
class CrewPrefsNotifier extends AsyncNotifier<CrewPrefs> {
  @override
  Future<CrewPrefs> build() async {
    try {
      return CrewPrefs.read(await SharedPreferences.getInstance());
    } catch (_) {
      return const CrewPrefs();
    }
  }

  CrewPrefs get _now => state.value ?? const CrewPrefs();

  void toggleSection() => _set(_now.copyWith(section: !_now.section));

  void toggleCrew(String id) {
    final crews = {..._now.crews};
    if (!crews.remove(id)) crews.add(id);
    _set(_now.copyWith(crews: crews));
  }

  void toggleTree(String id) {
    final trees = {..._now.trees};
    if (!trees.remove(id)) trees.add(id);
    _set(_now.copyWith(trees: trees));
  }

  void toggleEnded() => _set(_now.copyWith(ended: !_now.ended));

  void _set(CrewPrefs next) {
    state = AsyncData(next);
    _persist(next);
  }

  Future<void> _persist(CrewPrefs p) async {
    try {
      await p.write(await SharedPreferences.getInstance());
    } catch (_) {
      // No preferences store (a preview); memory still works.
    }
  }
}

final crewPrefsProvider = AsyncNotifierProvider<CrewPrefsNotifier, CrewPrefs>(
  CrewPrefsNotifier.new,
);

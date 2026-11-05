import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// Which crews, and whether the CREWS section, are collapsed on this device
/// (decision 138m). Per device and kept across restarts; never sent to the
/// server, so the laptop and the phone fold differently.
@immutable
class CrewPrefs {
  const CrewPrefs({this.section = false, this.crews = const {}});

  /// The CREWS section is folded to "CREWS · n".
  final bool section;

  /// Collapsed crews, by id.
  final Set<String> crews;

  bool collapsed(String crewId) => crews.contains(crewId);

  static const _kSection = 'storm.crews.collapsed.section';
  static const _kCrews = 'storm.crews.collapsed.crews';

  static CrewPrefs read(SharedPreferences prefs) => CrewPrefs(
    section: prefs.getBool(_kSection) ?? false,
    crews: {...?prefs.getStringList(_kCrews)},
  );

  Future<void> write(SharedPreferences prefs) async {
    await prefs.setBool(_kSection, section);
    await prefs.setStringList(_kCrews, crews.toList()..sort());
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

  void toggleSection() =>
      _set(CrewPrefs(section: !_now.section, crews: _now.crews));

  void toggleCrew(String id) {
    final crews = {..._now.crews};
    if (!crews.remove(id)) crews.add(id);
    _set(CrewPrefs(section: _now.section, crews: crews));
  }

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

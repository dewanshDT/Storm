import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'app_state.dart';

enum Activity { notes, agents, settings }

/// Which activity a location belongs to, or null outside the shell.
Activity? activityOf(String path) {
  if (path.startsWith('/v/') || path == '/notes') return Activity.notes;
  if (path == '/agents' || path.startsWith('/agents/')) return Activity.agents;
  if (path == '/settings' || path.startsWith('/settings/')) {
    return Activity.settings;
  }
  return null;
}

/// The last location per activity on this device (handoff §1.5).
@immutable
class NavMemory {
  const NavMemory({
    this.activity = Activity.notes,
    this.notes = '',
    this.agents = '',
    this.settings = '',
  });

  final Activity activity;
  final String notes;
  final String agents;
  final String settings;

  String lastOf(Activity a) => switch (a) {
    Activity.notes => notes,
    Activity.agents => agents,
    Activity.settings => settings,
  };

  /// Where an activity opens: its last location, else its entry.
  String entryOf(Activity a) {
    final last = lastOf(a);
    if (last.isNotEmpty) return last;
    return switch (a) {
      Activity.notes => '/notes',
      Activity.agents => '/agents',
      Activity.settings => '/settings',
    };
  }

  /// Where `/` goes on launch.
  String get launch => entryOf(activity);

  NavMemory visit(String location) {
    final a = activityOf(Uri.parse(location).path);
    if (a == null) return this;
    return NavMemory(
      activity: a,
      notes: a == Activity.notes && location != '/notes' ? location : notes,
      agents: a == Activity.agents ? location : agents,
      settings: a == Activity.settings ? location : settings,
    );
  }

  static const _kActivity = 'storm.nav.activity';
  static const _kNotes = 'storm.nav.notes';
  static const _kAgents = 'storm.nav.agents';
  static const _kSettings = 'storm.nav.settings';

  static NavMemory read(SharedPreferences prefs) => NavMemory(
    activity: Activity.values.firstWhere(
      (a) => a.name == prefs.getString(_kActivity),
      orElse: () => Activity.notes,
    ),
    notes: prefs.getString(_kNotes) ?? '',
    agents: prefs.getString(_kAgents) ?? '',
    settings: prefs.getString(_kSettings) ?? '',
  );

  Future<void> write(SharedPreferences prefs) async {
    await prefs.setString(_kActivity, activity.name);
    await prefs.setString(_kNotes, notes);
    await prefs.setString(_kAgents, agents);
    await prefs.setString(_kSettings, settings);
  }

  @override
  bool operator ==(Object other) =>
      other is NavMemory &&
      other.activity == activity &&
      other.notes == notes &&
      other.agents == agents &&
      other.settings == settings;

  @override
  int get hashCode => Object.hash(activity, notes, agents, settings);
}

/// Seeded from what [SettingsNotifier] read at launch, then kept in memory
/// and written through. Separate from settings so a navigation never
/// re-saves (and re-broadcasts) every setting.
class NavMemoryNotifier extends Notifier<NavMemory> {
  @override
  NavMemory build() =>
      ref.read(settingsProvider).value?.nav ?? const NavMemory();

  void visit(String location) {
    final next = state.visit(location);
    if (next == state) return;
    state = next;
    _persist(next);
  }

  Future<void> _persist(NavMemory m) async {
    try {
      await m.write(await SharedPreferences.getInstance());
    } catch (_) {
      // Tests and previews have no preferences store; memory still works.
    }
  }
}

final navMemoryProvider = NotifierProvider<NavMemoryNotifier, NavMemory>(
  NavMemoryNotifier.new,
);

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../api/models.dart';
import '../api/storm_api.dart';
import '../cache/cache_db.dart';
import '../sync/sync_engine.dart';
import 'note_session.dart';

/// Connection and appearance settings.
///
/// v1 stores the token in plain shared_preferences. That is acceptable only
/// because the server is LAN-only; moving to `flutter_secure_storage` on
/// native platforms is a prerequisite for any wider exposure.
@immutable
class Settings {
  const Settings({
    this.baseUrl = '',
    this.token = '',
    this.darkMode = false,
    this.fontSize = 16,
  });

  final String baseUrl;
  final String token;
  final bool darkMode;
  final double fontSize;

  bool get isConfigured => baseUrl.isNotEmpty && token.isNotEmpty;

  Settings copyWith({
    String? baseUrl,
    String? token,
    bool? darkMode,
    double? fontSize,
  }) => Settings(
    baseUrl: baseUrl ?? this.baseUrl,
    token: token ?? this.token,
    darkMode: darkMode ?? this.darkMode,
    fontSize: fontSize ?? this.fontSize,
  );
}

class SettingsNotifier extends AsyncNotifier<Settings> {
  static const _kUrl = 'storm.baseUrl';
  static const _kToken = 'storm.token';
  static const _kDark = 'storm.darkMode';
  static const _kFont = 'storm.fontSize';

  @override
  Future<Settings> build() async {
    final prefs = await SharedPreferences.getInstance();
    return Settings(
      baseUrl: prefs.getString(_kUrl) ?? '',
      token: prefs.getString(_kToken) ?? '',
      darkMode: prefs.getBool(_kDark) ?? false,
      fontSize: prefs.getDouble(_kFont) ?? 16,
    );
  }

  Future<void> save(Settings next) async {
    // Normalise here rather than at every call site: a trailing slash produces
    // `//v1/tree`, which some proxies reject.
    final cleaned = next.copyWith(baseUrl: normalizeUrl(next.baseUrl));
    state = AsyncData(cleaned);

    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_kUrl, cleaned.baseUrl);
    await prefs.setString(_kToken, cleaned.token);
    await prefs.setBool(_kDark, cleaned.darkMode);
    await prefs.setDouble(_kFont, cleaned.fontSize);
  }
}

/// Trims whitespace, strips trailing slashes, and defaults to `http://`.
String normalizeUrl(String raw) {
  var url = raw.trim();
  if (url.isEmpty) return '';
  if (!url.startsWith('http://') && !url.startsWith('https://')) {
    url = 'http://$url';
  }
  while (url.endsWith('/')) {
    url = url.substring(0, url.length - 1);
  }
  return url;
}

final settingsProvider = AsyncNotifierProvider<SettingsNotifier, Settings>(
  SettingsNotifier.new,
);

/// The API client, rebuilt whenever the connection settings change.
final apiProvider = Provider<StormApi?>((ref) {
  final settings = ref.watch(settingsProvider).value;
  if (settings == null || !settings.isConfigured) return null;

  final api = StormApi(baseUrl: settings.baseUrl, token: settings.token);
  ref.onDispose(api.dispose);
  return api;
});

/// The local cache. One instance for the app's lifetime.
final cacheProvider = Provider<CacheDb>((ref) {
  final db = CacheDb();
  ref.onDispose(db.close);
  return db;
});

/// Owns the cache, the outbox and the server connection.
///
/// Everything above this reads and writes through it rather than touching
/// [StormApi], so offline behaviour lives in one place.
/// Owns the cache, the outbox and the server connection.
///
/// **Watch this only to render status** (online, syncing, unsent count).
/// `ChangeNotifierProvider` rebuilds its watchers on every `notifyListeners()`,
/// and the engine notifies on every status tick — so anything that needs the
/// *instance* rather than the notifications must `ref.read` it, or it gets
/// torn down and rebuilt several times a second.
final syncEngineProvider = ChangeNotifierProvider<SyncEngine>((ref) {
  final api = ref.watch(apiProvider);
  final engine = SyncEngine(
    api: api ?? StormApi(baseUrl: '', token: ''),
    cache: ref.watch(cacheProvider),
  );
  if (api != null) engine.start();
  // No `ref.onDispose(engine.dispose)`: ChangeNotifierProvider already
  // disposes what it holds, and disposing twice trips ChangeNotifier's
  // "used after being disposed" assert.
  return engine;
});

/// Bumped when a sync actually changed data.
///
/// Data providers watch this instead of the engine, so a status tick doesn't
/// trigger a refetch of the tree, the tags and every open backlinks panel.
final vaultRevisionProvider = StateProvider<int>((ref) => 0);

/// Keeps the open note and the vault views in step with incoming changes.
///
/// Instantiated by the shell; nothing else depends on its value.
final syncListenerProvider = Provider<void>((ref) {
  // Rebuild only when the connection itself changes, never on a status tick.
  ref.watch(apiProvider);
  final engine = ref.read(syncEngineProvider);

  final sub = engine.changes.listen((ids) {
    final session = ref.read(noteSessionProvider);
    if (session.noteId != null && ids.contains(session.noteId)) {
      // The session decides whether to adopt it — a local edit always wins.
      session.onRemoteChange();
    }
    ref.read(vaultRevisionProvider.notifier).state++;
  });

  ref.onDispose(sub.cancel);
});

/// The vault tree — server when reachable, cache when not.
final treeProvider = FutureProvider<List<NoteMeta>>((ref) async {
  ref.watch(apiProvider);
  ref.watch(vaultRevisionProvider);
  return ref.read(syncEngineProvider).tree();
});

/// The currently open note, if any.
final openNoteIdProvider = StateProvider<String?>((ref) => null);

final noteSessionProvider = ChangeNotifierProvider<NoteSession>((ref) {
  // Deliberately `watch(apiProvider)` + `read(syncEngineProvider)`: the
  // session must be rebuilt when the server connection changes, but must
  // survive the engine's status notifications. Watching the engine here reset
  // the editor mid-open and left the tree showing a selected note with an
  // empty pane.
  ref.watch(apiProvider);
  // Disposal is ChangeNotifierProvider's job — see syncEngineProvider.
  return NoteSession(ref.read(syncEngineProvider));
});

final searchQueryProvider = StateProvider<String>((ref) => '');

final searchResultsProvider = FutureProvider<List<SearchHit>>((ref) async {
  final api = ref.watch(apiProvider);
  final query = ref.watch(searchQueryProvider);
  if (api == null || query.trim().isEmpty) return const [];
  return api.search(query);
});

/// Backlinks for a note — the "linked mentions" panel.
///
/// Server-only: resolving these needs the whole vault's link index, which the
/// client deliberately does not hold. Offline it returns empty rather than
/// pretending there are none.
final backlinksProvider = FutureProvider.family<List<NoteMeta>, String>((
  ref,
  id,
) async {
  final api = ref.watch(apiProvider);
  if (api == null) return const [];
  // Re-resolve whenever sync pulls something in, so the panel isn't stale.
  ref.watch(syncEngineProvider);
  return api.backlinks(id);
});

final tagsProvider = FutureProvider<List<TagCount>>((ref) async {
  final api = ref.watch(apiProvider);
  if (api == null) return const [];
  ref.watch(syncEngineProvider);
  return api.tags();
});

final selectedTagProvider = StateProvider<String?>((ref) => null);

final notesWithTagProvider = FutureProvider.family<List<NoteMeta>, String>((
  ref,
  tag,
) async {
  final api = ref.watch(apiProvider);
  if (api == null) return const [];
  return api.notesWithTag(tag);
});

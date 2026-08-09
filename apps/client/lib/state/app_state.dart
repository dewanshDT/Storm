import 'package:drift/drift.dart' show Value;
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../api/models.dart';
import '../api/storm_api.dart';
import '../cache/cache_db.dart';
import '../ui/theme.dart';
import '../ui/tokens.dart';
import '../sync/sync_engine.dart';
import 'note_session.dart';
import '../ui/accents.dart';
import 'vault_config.dart';

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
    // Dark-first, matching the design. Someone who turns it off has `false`
    // stored, so this default never overrides a deliberate choice.
    this.theme = StormPreset.stormDark,
    this.fontSize = 16,
    this.activeVault = '',
    this.bodyFont = BodyFont.serif,
    this.showNoteId = false,
  });

  final String baseUrl;
  final String token;

  /// Which visual identity the app wears.
  ///
  /// Replaces the old `darkMode` bool, which could only express two of the
  /// three shipped presets. The stored bool is migrated on load rather than
  /// dropped — see [SettingsNotifier.build].
  final StormPreset theme;

  /// Kept so the rest of the app can still ask the simple question.
  bool get darkMode => theme.inputs.dark;
  final double fontSize;

  /// The face note bodies are set in.
  ///
  /// Only families that ship with the app or the platform: a runtime download
  /// is wrong for something that has to work offline, and it would make the
  /// editor's metrics depend on the network.
  final BodyFont bodyFont;

  /// Whether the properties list shows a note's `id`.
  ///
  /// Off by default. It is a UUID Storm assigned and nobody reads it, so it
  /// costs a row at the top of every note for information that only matters
  /// when something has gone wrong. Available rather than removed, because
  /// the list is the only place frontmatter is visible at all.
  final bool showNoteId;

  /// Which vault the note-level providers are serving.
  ///
  /// The **route** is the source of truth for which vault is open; this is a
  /// persisted mirror that `apiProvider` reads, which is what makes switching
  /// vaults reuse the existing teardown — the engine is disposed, its socket
  /// closed and the session rebuilt, all by machinery that already existed.
  /// `VaultGate` keeps the two in agreement and refuses to render a vault's
  /// screens until they are.
  ///
  /// Persisting it also reopens the last vault on launch, for free.
  final String activeVault;

  bool get isConfigured => baseUrl.isNotEmpty && token.isNotEmpty;

  Settings copyWith({
    String? baseUrl,
    String? token,
    StormPreset? theme,
    double? fontSize,
    String? activeVault,
    BodyFont? bodyFont,
    bool? showNoteId,
  }) => Settings(
    baseUrl: baseUrl ?? this.baseUrl,
    token: token ?? this.token,
    theme: theme ?? this.theme,
    fontSize: fontSize ?? this.fontSize,
    activeVault: activeVault ?? this.activeVault,
    bodyFont: bodyFont ?? this.bodyFont,
    showNoteId: showNoteId ?? this.showNoteId,
  );
}

class SettingsNotifier extends AsyncNotifier<Settings> {
  static const _kUrl = 'storm.baseUrl';
  static const _kToken = 'storm.token';
  static const _kDark = 'storm.darkMode';
  static const _kTheme = 'storm.theme';
  static const _kFont = 'storm.fontSize';
  static const _kVault = 'storm.activeVault';
  static const _kBodyFont = 'storm.bodyFont';
  static const _kShowId = 'storm.showNoteId';

  @override
  Future<Settings> build() async {
    final prefs = await SharedPreferences.getInstance();
    return Settings(
      baseUrl: prefs.getString(_kUrl) ?? '',
      token: prefs.getString(_kToken) ?? '',
      theme: _themeFrom(prefs),
      fontSize: prefs.getDouble(_kFont) ?? 16,
      activeVault: prefs.getString(_kVault) ?? '',
      bodyFont: BodyFont.fromName(prefs.getString(_kBodyFont)),
      showNoteId: prefs.getBool(_kShowId) ?? false,
    );
  }

  /// The stored theme, migrating an installation that only knew dark/light.
  ///
  /// Someone upgrading has `storm.darkMode` and no `storm.theme`, and losing
  /// their choice would silently flip the app to dark on the first launch after
  /// an update. The old key is read but never written again, so the migration
  /// happens once and then stops mattering.
  static StormPreset _themeFrom(SharedPreferences prefs) {
    final stored = prefs.getString(_kTheme);
    if (stored != null) return StormPreset.parse(stored);

    final wasDark = prefs.getBool(_kDark);
    if (wasDark == null) return StormPreset.stormDark;
    return wasDark ? StormPreset.stormDark : StormPreset.stormLight;
  }

  Future<void> save(Settings next) async {
    // Normalise here rather than at every call site: a trailing slash produces
    // `//v1/tree`, which some proxies reject.
    final cleaned = next.copyWith(baseUrl: normalizeUrl(next.baseUrl));
    state = AsyncData(cleaned);

    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_kUrl, cleaned.baseUrl);
    await prefs.setString(_kToken, cleaned.token);
    await prefs.setString(_kTheme, cleaned.theme.wire);
    await prefs.setDouble(_kFont, cleaned.fontSize);
    await prefs.setString(_kVault, cleaned.activeVault);
    await prefs.setString(_kBodyFont, cleaned.bodyFont.name);
    await prefs.setBool(_kShowId, cleaned.showNoteId);
  }
}

/// The faces a note body can be set in.
///
/// Three, not a font list: every extra family is another megabyte in the APK,
/// and these cover the distinction that actually matters — prose, interface,
/// and fixed-width.
enum BodyFont {
  /// The bundled serif. Newsreader, shipped in `assets/fonts/`.
  serif('serif', 'Serif', StormTheme.bodyFamily),

  /// Whatever the platform uses for its own interface.
  sans('sans', 'Sans', null),

  /// The platform's fixed-width face.
  mono('mono', 'Monospace', 'monospace');

  const BodyFont(this.name, this.label, this.family);

  final String name;
  final String label;

  /// `null` means "the platform default", which is what Flutter does with an
  /// unset `fontFamily`.
  final String? family;

  static BodyFont fromName(String? name) {
    for (final font in BodyFont.values) {
      if (font.name == name) return font;
    }
    return BodyFont.serif;
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
///
/// One client per *server*. The vault is a parameter of each call, so this
/// does not rebuild on a vault switch — [syncEngineProvider] does.
final apiProvider = Provider<StormApi?>((ref) {
  final settings = ref.watch(settingsProvider).value;
  if (settings == null || !settings.isConfigured) return null;

  final api = StormApi(baseUrl: settings.baseUrl, token: settings.token);
  ref.onDispose(api.dispose);
  return api;
});

/// The vault the note-level providers are serving, or `''` when none is open
/// (on the dashboard, say).
///
/// Derived from settings rather than held separately, so there is exactly one
/// answer and no second copy to keep in step.
final activeVaultProvider = Provider<String>((ref) {
  return ref.watch(settingsProvider).value?.activeVault ?? '';
});

/// Every vault on the server. Powers the dashboard grid and the switcher.
final vaultsProvider = FutureProvider<List<VaultInfo>>((ref) async {
  ref.watch(vaultRevisionProvider);
  final api = ref.watch(apiProvider);
  if (api == null) return const [];
  final vaults = await api.vaults();

  // Rows cached before multi-vault carry no vault id. With exactly one vault
  // there is only one thing they can belong to; with more, guessing would
  // attach one vault's queued edits to another, so they are left for
  // `legacyRowCount` to report instead.
  if (vaults.length == 1) {
    await ref.read(cacheProvider).adoptLegacyRows(vaults.first.id);
  }
  return vaults;
});

/// Each vault's accent, read from its own `_storm/vault.md`.
///
/// The dashboard shows every vault at once, so it cannot use
/// [vaultConfigProvider], which only serves the *active* vault.
final vaultAccentsProvider = FutureProvider<Map<String, Accent>>((ref) async {
  ref.watch(vaultRevisionProvider);
  final api = ref.watch(apiProvider);
  final vaults = ref.watch(vaultsProvider).value ?? const <VaultInfo>[];
  if (api == null) return const {};

  final out = <String, Accent>{};
  for (final vault in vaults) {
    if (vault.missing) continue;
    try {
      final tree = await api.tree(vault.id);
      final config = tree.notes
          .where((n) => n.path == kVaultConfigPath)
          .firstOrNull;
      if (config == null) continue;
      final note = await api.note(vault.id, config.id);
      out[vault.id] = VaultConfig.parse(note.content).accent;
    } catch (_) {
      // A vault whose colour cannot be read is simply uncoloured.
    }
  }
  return out;
});

/// Recently opened notes across every vault — server first, cache when offline.
final recentsProvider = FutureProvider<List<RecentNote>>((ref) async {
  ref.watch(vaultRevisionProvider);
  final api = ref.watch(apiProvider);
  final cache = ref.read(cacheProvider);
  if (api == null) return const [];

  try {
    final fresh = (await api.recents(
      limit: 20,
    )).where((r) => !isVaultConfigPath(r.path)).toList();
    await cache.replaceRecents([
      for (final r in fresh)
        RecentsCompanion.insert(
          vaultId: r.vaultId,
          noteId: r.noteId,
          vaultName: Value(r.vaultName),
          path: Value(r.path),
          title: Value(r.title),
          openedAt: DateTime.tryParse(r.openedAt) ?? DateTime.now(),
        ),
    ]);
    return fresh;
  } catch (_) {
    // Same server-then-cache shape as `SyncEngine.tree()`: the dashboard is
    // the home screen and must render without a connection.
    final held = await cache.recentNotes();
    return [
      for (final r in held)
        RecentNote(
          vaultId: r.vaultId,
          vaultName: r.vaultName,
          noteId: r.noteId,
          path: r.path,
          title: r.title,
          modified: '',
          openedAt: r.openedAt.toIso8601String(),
        ),
    ];
  }
});

/// The server's own settings, chiefly where vaults are stored.
final serverConfigProvider = FutureProvider<ServerConfig?>((ref) async {
  final api = ref.watch(apiProvider);
  if (api == null) return null;
  return api.config();
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
  final vaultId = ref.watch(activeVaultProvider);
  final engine = SyncEngine(
    api: api ?? StormApi(baseUrl: '', token: ''),
    cache: ref.watch(cacheProvider),
    vaultId: vaultId,
  );
  // No vault open means nothing to sync — the dashboard reads the vault list
  // and the recents endpoint, neither of which needs an engine.
  if (api != null && vaultId.isNotEmpty) engine.start();
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
  // Watched, not read: switching vaults has to refetch, or the browser paints
  // the previous vault's notes under the new vault's name.
  ref.watch(activeVaultProvider);
  return ref.read(syncEngineProvider).tree();
});

/// Folders that exist in the vault, including empty ones.
///
/// Derived from the same fetch as [treeProvider] rather than its own request:
/// the server returns both in one response.
final vaultFoldersProvider = Provider<List<String>>((ref) {
  ref.watch(treeProvider);
  return ref.read(syncEngineProvider).folders;
});

/// The currently open note, if any.
final openNoteIdProvider = StateProvider<String?>((ref) => null);

/// Whether the properties drawer is open beside the note, at desk width.
///
/// A provider rather than the note screen's own state, which is where it
/// started: opening a second note built a second `_NoteScreenState` and the
/// drawer shut. Properties are a *pane* at that width — you open the column
/// once and it stays while you read around it, the way the sidebar does.
///
/// Not read on a phone. There properties are a modal sheet, which has to be
/// dismissed before anything else can be touched, so "is it open" is the
/// navigator's question rather than ours.
final propertiesOpenProvider = StateProvider<bool>((ref) => false);

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

/// Which notes are kept available offline.
final pinnedNotesProvider = FutureProvider<Set<String>>((ref) async {
  ref.watch(vaultRevisionProvider);
  ref.watch(activeVaultProvider);
  return ref.read(syncEngineProvider).pinnedIds();
});

final searchQueryProvider = StateProvider<String>((ref) => '');

final searchResultsProvider = FutureProvider<List<SearchHit>>((ref) async {
  final api = ref.watch(apiProvider);
  final vaultId = ref.watch(activeVaultProvider);
  final query = ref.watch(searchQueryProvider);
  if (api == null || vaultId.isEmpty || query.trim().isEmpty) return const [];
  final hits = await api.search(vaultId, query);
  // Storm's own config note is not a search result.
  return hits.where((h) => !isVaultConfigPath(h.path)).toList();
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
  final vaultId = ref.watch(activeVaultProvider);
  if (api == null || vaultId.isEmpty) return const [];
  // Re-resolve whenever sync pulls something in, so the panel isn't stale.
  ref.watch(syncEngineProvider);
  return api.backlinks(vaultId, id);
});

final tagsProvider = FutureProvider<List<TagCount>>((ref) async {
  final api = ref.watch(apiProvider);
  final vaultId = ref.watch(activeVaultProvider);
  if (api == null || vaultId.isEmpty) return const [];
  ref.watch(syncEngineProvider);
  return api.tags(vaultId);
});

final selectedTagProvider = StateProvider<String?>((ref) => null);

final notesWithTagProvider = FutureProvider.family<List<NoteMeta>, String>((
  ref,
  tag,
) async {
  final api = ref.watch(apiProvider);
  final vaultId = ref.watch(activeVaultProvider);
  if (api == null || vaultId.isEmpty) return const [];
  return api.notesWithTag(vaultId, tag);
});

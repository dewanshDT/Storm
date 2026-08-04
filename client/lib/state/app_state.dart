import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../api/models.dart';
import '../api/storm_api.dart';
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
  }) =>
      Settings(
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

final settingsProvider =
    AsyncNotifierProvider<SettingsNotifier, Settings>(SettingsNotifier.new);

/// The API client, rebuilt whenever the connection settings change.
final apiProvider = Provider<StormApi?>((ref) {
  final settings = ref.watch(settingsProvider).value;
  if (settings == null || !settings.isConfigured) return null;

  final api = StormApi(baseUrl: settings.baseUrl, token: settings.token);
  ref.onDispose(api.dispose);
  return api;
});

/// The vault tree. Invalidate this after any structural change.
final treeProvider = FutureProvider<VaultTree>((ref) async {
  final api = ref.watch(apiProvider);
  if (api == null) throw StormApiException(0, 'Not connected');
  return api.tree();
});

/// The currently open note, if any.
final openNoteIdProvider = StateProvider<String?>((ref) => null);

final noteSessionProvider = ChangeNotifierProvider<NoteSession>((ref) {
  final api = ref.watch(apiProvider);
  final session = NoteSession(
    api ?? StormApi(baseUrl: '', token: ''),
  );
  ref.onDispose(session.dispose);
  return session;
});

final searchQueryProvider = StateProvider<String>((ref) => '');

final searchResultsProvider = FutureProvider<List<SearchHit>>((ref) async {
  final api = ref.watch(apiProvider);
  final query = ref.watch(searchQueryProvider);
  if (api == null || query.trim().isEmpty) return const [];
  return api.search(query);
});

final backlinksProvider =
    FutureProvider.family<List<NoteMeta>, String>((ref, id) async {
  final api = ref.watch(apiProvider);
  if (api == null) return const [];
  return api.backlinks(id);
});

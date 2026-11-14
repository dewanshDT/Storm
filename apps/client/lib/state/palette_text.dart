import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../api/storm_api.dart';
import 'app_state.dart';
import 'vault_config.dart' show isVaultConfigPath;

/// The palette's In note text search (decision 156l): one query, every
/// vault.
typedef PaletteTextSearch = Future<CrossSearchResult> Function(String query);

/// Null while signed out. A provider so tests can hold or fail a response.
final paletteTextSearchProvider = Provider<PaletteTextSearch?>((ref) {
  final api = ref.watch(apiProvider);
  if (api == null) return null;
  return (query) => searchNoteText(api, query);
});

/// What a vault answers on the per-vault fallback.
const kPerVaultTextLimit = 20;

/// `GET /v1/search`, or against a server older than decision 156 (a 404),
/// the per-vault route over each vault. Throws on any other failure.
Future<CrossSearchResult> searchNoteText(StormApi api, String query) async {
  CrossSearchResult result;
  try {
    result = await api.searchAll(query);
  } on StormApiException catch (e) {
    if (!e.isNotFound) rethrow;
    result = await _searchEachVault(api, query);
  }
  return CrossSearchResult(
    hits: [
      for (final h in result.hits)
        if (!isVaultConfigPath(h.path)) h,
    ],
    skipped: result.skipped,
  );
}

Future<CrossSearchResult> _searchEachVault(StormApi api, String query) async {
  final vaults = [
    for (final v in await api.vaults())
      if (!v.missing) v,
  ];
  final answers = await Future.wait([
    for (final v in vaults)
      api
          .search(v.id, query, limit: kPerVaultTextLimit)
          .then<List<SearchHit>?>((h) => h, onError: (_) => null),
  ]);
  final want = query.trim().toLowerCase();
  final hits = <CrossSearchHit>[];
  final skipped = <SkippedVault>[];
  for (final (i, v) in vaults.indexed) {
    final answer = answers[i];
    if (answer == null) {
      skipped.add(
        SkippedVault(vaultId: v.id, vaultName: v.name, reason: 'failed'),
      );
      continue;
    }
    for (final h in answer) {
      hits.add(
        CrossSearchHit(
          vaultId: v.id,
          vaultName: v.name,
          id: h.id,
          path: h.path,
          title: h.title,
          snippet: h.snippet,
          tier: _tier(h.title.trim().toLowerCase(), want),
        ),
      );
    }
  }
  // Stable: within a tier, each vault's own order and the vault order stay.
  final ordered = [for (final (i, h) in hits.indexed) (i, h)]
    ..sort((a, b) {
      final d = a.$2.tier - b.$2.tier;
      return d != 0 ? d : a.$1 - b.$1;
    });
  return CrossSearchResult(
    hits: [for (final (_, h) in ordered.take(50)) h],
    skipped: skipped,
  );
}

/// The server's tiers (156k): exact title, title prefix, title contains,
/// then anything else.
int _tier(String title, String want) {
  if (want.isEmpty) return 3;
  if (title == want) return 0;
  if (title.startsWith(want)) return 1;
  if (title.contains(want)) return 2;
  return 3;
}

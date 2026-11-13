import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import 'app_state.dart';
import 'vault_config.dart' show isVaultConfigPath;
import 'wikilinks.dart' show wikilinkDisplayName;

/// A note the palette can find by title, in any vault.
class PaletteNote {
  const PaletteNote({
    required this.vaultId,
    required this.vaultName,
    required this.meta,
  });

  final String vaultId;
  final String vaultName;
  final NoteMeta meta;

  String get name => wikilinkDisplayName(meta);
}

class PaletteFolder {
  const PaletteFolder({
    required this.vaultId,
    required this.vaultName,
    required this.path,
  });

  final String vaultId;
  final String vaultName;
  final String path;
}

/// Every vault's note titles and folders, for the palette's instant matches
/// (decision 156j).
class PaletteTitles {
  const PaletteTitles({
    required this.notes,
    required this.folders,
    required this.fetchedAt,
    this.offline = false,
  });

  static final empty = PaletteTitles(
    notes: const [],
    folders: const [],
    fetchedAt: DateTime.fromMillisecondsSinceEpoch(0),
  );

  final List<PaletteNote> notes;
  final List<PaletteFolder> folders;
  final DateTime fetchedAt;

  /// The server could not be asked, so these are the notes this device
  /// holds.
  final bool offline;
}

/// How old [paletteTitlesProvider] may be when the palette reopens.
const kPaletteTitlesMaxAge = Duration(seconds: 60);

/// Vault names from the last time the server answered, for tagging cached
/// notes while it doesn't.
final _knownVaultNamesProvider = Provider<Map<String, String>>((ref) => {});

/// Built when the palette first reads it, and kept: refetched when the
/// active vault's revision moves, and by [refreshPaletteTitlesIfStale]
/// when the palette reopens on a list older than [kPaletteTitlesMaxAge].
/// Never throws: offline it is the cached notes (drift `CachedNotes`).
final paletteTitlesProvider = FutureProvider<PaletteTitles>((ref) async {
  final revision = ref.watch(vaultRevisionProvider);
  final api = ref.watch(apiProvider);
  final trees = ref.watch(vaultTreeFetcherProvider);
  final cache = ref.read(cacheProvider);
  final names = ref.read(_knownVaultNamesProvider);
  if (api == null || trees == null) return PaletteTitles.empty;

  final notes = <PaletteNote>[];
  final folders = <PaletteFolder>[];
  var offline = false;

  List<VaultInfo> vaults;
  try {
    // Asked directly: offline, `vaultsProvider` retries on its own schedule
    // and its future would not settle until the server is back.
    vaults = await api.vaults();
  } catch (_) {
    vaults = const [];
    offline = true;
  }
  for (final v in vaults) {
    names[v.id] = v.name;
  }

  void add(String vaultId, List<NoteMeta> metas, List<String> dirs) {
    final name = names[vaultId] ?? '';
    for (final m in metas) {
      if (isVaultConfigPath(m.path)) continue;
      notes.add(PaletteNote(vaultId: vaultId, vaultName: name, meta: m));
    }
    for (final d in dirs) {
      if (d == '_storm' || isVaultConfigPath('$d/')) continue;
      folders.add(PaletteFolder(vaultId: vaultId, vaultName: name, path: d));
    }
  }

  Future<void> cached(String vaultId) async {
    try {
      final held = await cache.allNotes(vaultId);
      add(vaultId, [
        for (final n in held)
          NoteMeta(
            id: n.id,
            path: n.path,
            title: n.title,
            version: n.version,
            modified: n.modified,
            size: n.content.length,
          ),
      ], const []);
    } catch (_) {}
  }

  if (offline) {
    final ids = {...names.keys, ref.read(activeVaultProvider)}..remove('');
    for (final id in ids) {
      await cached(id);
    }
  } else {
    var reached = 0;
    final live = vaults.where((v) => !v.missing).toList();
    final fetched = await Future.wait([
      for (final v in live)
        trees
            .tree(v.id, revision: revision, maxAge: kPaletteTitlesMaxAge)
            .then<VaultTree?>((t) => t, onError: (Object _) => null),
    ]);
    for (final (i, v) in live.indexed) {
      final tree = fetched[i];
      if (tree == null) {
        await cached(v.id);
        continue;
      }
      reached++;
      add(v.id, tree.notes, tree.folders);
    }
    offline = live.isNotEmpty && reached == 0;
  }
  return PaletteTitles(
    notes: notes,
    folders: folders,
    fetchedAt: DateTime.now(),
    offline: offline,
  );
});

/// Called as the palette opens: a list older than [kPaletteTitlesMaxAge]
/// is fetched again, and the old one shown meanwhile.
void refreshPaletteTitlesIfStale(WidgetRef ref) {
  final held = ref.read(paletteTitlesProvider).value;
  if (held == null) return;
  if (DateTime.now().difference(held.fetchedAt) > kPaletteTitlesMaxAge) {
    ref.invalidate(paletteTitlesProvider);
  }
}

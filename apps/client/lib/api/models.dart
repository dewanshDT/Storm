/// Wire types shared with `storm-server`.
///
/// Kept deliberately thin — these mirror the server's JSON exactly rather than
/// modelling anything the client invents, so a protocol change shows up as a
/// compile error here rather than a silent misparse.
library;

/// A note's metadata. Content is fetched separately.
class NoteMeta {
  const NoteMeta({
    required this.id,
    required this.path,
    required this.title,
    required this.version,
    required this.modified,
    required this.size,
  });

  final String id;
  final String path;
  final String title;
  final int version;
  final String modified;
  final int size;

  /// The folder this note lives in, or `''` at the vault root.
  String get folder {
    final i = path.lastIndexOf('/');
    return i < 0 ? '' : path.substring(0, i);
  }

  String get fileName {
    final i = path.lastIndexOf('/');
    return i < 0 ? path : path.substring(i + 1);
  }

  factory NoteMeta.fromJson(Map<String, dynamic> j) => NoteMeta(
    id: j['id'] as String,
    path: j['path'] as String,
    title: j['title'] as String? ?? '',
    version: (j['version'] as num).toInt(),
    modified: j['modified'] as String? ?? '',
    size: (j['size'] as num?)?.toInt() ?? 0,
  );
}

/// A note with its content.
class Note {
  const Note({required this.meta, required this.content});

  final NoteMeta meta;
  final String content;

  /// The server flattens metadata alongside `content` in one object.
  factory Note.fromJson(Map<String, dynamic> j) =>
      Note(meta: NoteMeta.fromJson(j), content: j['content'] as String? ?? '');

  Note copyWith({NoteMeta? meta, String? content}) =>
      Note(meta: meta ?? this.meta, content: content ?? this.content);
}

/// The vault tree plus the change-log position it was read at.
class VaultTree {
  const VaultTree({
    required this.notes,
    required this.folders,
    required this.seq,
  });

  final List<NoteMeta> notes;
  final List<String> folders;
  final int seq;

  factory VaultTree.fromJson(Map<String, dynamic> j) => VaultTree(
    notes: (j['notes'] as List)
        .map((n) => NoteMeta.fromJson(n as Map<String, dynamic>))
        .toList(),
    folders: (j['folders'] as List).cast<String>(),
    seq: (j['seq'] as num).toInt(),
  );
}

/// The result of a write.
///
/// [merged] and [conflict] are the important fields: when either is set the
/// client **must** adopt [content], because the server reconciled against a
/// version this client never saw. Ignoring it means the next save races.
class WriteResult {
  const WriteResult({
    required this.meta,
    required this.content,
    required this.seq,
    required this.merged,
    required this.conflict,
  });

  final NoteMeta meta;
  final String content;
  final int seq;
  final bool merged;
  final bool conflict;

  factory WriteResult.fromJson(Map<String, dynamic> j) => WriteResult(
    meta: NoteMeta.fromJson(j['note'] as Map<String, dynamic>),
    content: j['content'] as String? ?? '',
    seq: (j['seq'] as num).toInt(),
    merged: j['merged'] as bool? ?? false,
    conflict: j['conflict'] as bool? ?? false,
  );
}

/// One entry from the server's change log.
///
/// Carries metadata only — the client decides what content to fetch. That
/// keeps the log cheap and means a client can catch up on thousands of changes
/// without downloading a vault it doesn't cache.
class Change {
  const Change({
    required this.seq,
    required this.vaultId,
    required this.noteId,
    required this.kind,
    required this.version,
    required this.at,
  });

  final int seq;

  /// Which vault this happened in. One WebSocket carries every vault's
  /// changes, so a listener must filter on this rather than assume.
  final String vaultId;
  final String noteId;

  /// `created`, `updated`, `moved` or `deleted`.
  final String kind;
  final int version;
  final String at;

  factory Change.fromJson(Map<String, dynamic> j) => Change(
    seq: (j['seq'] as num).toInt(),
    vaultId: j['vault_id'] as String? ?? '',
    noteId: j['note_id'] as String,
    kind: j['kind'] as String,
    version: (j['version'] as num?)?.toInt() ?? 0,
    at: j['at'] as String? ?? '',
  );
}

/// One vault on the server.
class VaultInfo {
  const VaultInfo({
    required this.id,
    required this.name,
    required this.dir,
    required this.noteCount,
    required this.missing,
  });

  final String id;
  final String name;

  /// Directory name under the storage root.
  final String dir;
  final int noteCount;

  /// The directory is gone. Kept in the list rather than hidden — a vault that
  /// silently disappears looks identical to one that never existed.
  final bool missing;

  factory VaultInfo.fromJson(Map<String, dynamic> j) => VaultInfo(
    id: j['id'] as String,
    name: j['name'] as String? ?? '',
    dir: j['dir'] as String? ?? '',
    noteCount: (j['note_count'] as num?)?.toInt() ?? 0,
    missing: j['missing'] as bool? ?? false,
  );
}

/// A recently opened note, from any vault.
class RecentNote {
  const RecentNote({
    required this.vaultId,
    required this.vaultName,
    required this.noteId,
    required this.path,
    required this.title,
    required this.modified,
    required this.openedAt,
  });

  final String vaultId;
  final String vaultName;
  final String noteId;
  final String path;
  final String title;
  final String modified;
  final String openedAt;

  /// The folder holding it, or `''` at the vault root.
  String get folder {
    final i = path.lastIndexOf('/');
    return i < 0 ? '' : path.substring(0, i);
  }

  String get fileName {
    final i = path.lastIndexOf('/');
    final name = i < 0 ? path : path.substring(i + 1);
    return name.endsWith('.md') ? name.substring(0, name.length - 3) : name;
  }

  /// What to show: the note's heading when it has one, else its file name.
  String get displayTitle => title.isEmpty ? fileName : title;

  factory RecentNote.fromJson(Map<String, dynamic> j) => RecentNote(
    vaultId: j['vault_id'] as String,
    vaultName: j['vault_name'] as String? ?? '',
    noteId: j['note_id'] as String,
    path: j['path'] as String? ?? '',
    title: j['title'] as String? ?? '',
    modified: j['modified'] as String? ?? '',
    openedAt: j['opened_at'] as String? ?? '',
  );
}

/// Server-level settings, chiefly where vaults are stored.
class ServerConfig {
  const ServerConfig({
    required this.vaultRoot,
    required this.stateDir,
    required this.vaultCount,
    required this.mcpEnabled,
  });

  final String vaultRoot;
  final String stateDir;
  final int vaultCount;

  /// Whether the server answers on `/mcp`.
  ///
  /// Defaults to false when the key is absent, which is what an older server
  /// sends — the safe direction, since the switch would otherwise read as on
  /// against a server that cannot serve it.
  final bool mcpEnabled;

  factory ServerConfig.fromJson(Map<String, dynamic> j) => ServerConfig(
    vaultRoot: j['vault_root'] as String? ?? '',
    stateDir: j['state_dir'] as String? ?? '',
    vaultCount: (j['vault_count'] as num?)?.toInt() ?? 0,
    mcpEnabled: j['mcp_enabled'] as bool? ?? false,
  );
}

/// A page of changes plus the server's current log position.
///
/// [seq] is what the client stores to resume from; it is the server's latest,
/// not the last item in [changes], so an empty batch still advances nothing
/// incorrectly.
class SyncBatch {
  const SyncBatch({required this.changes, required this.seq});

  final List<Change> changes;
  final int seq;

  factory SyncBatch.fromJson(Map<String, dynamic> j) => SyncBatch(
    changes: (j['changes'] as List)
        .map((c) => Change.fromJson(c as Map<String, dynamic>))
        .toList(),
    seq: (j['seq'] as num).toInt(),
  );
}

class SearchHit {
  const SearchHit({
    required this.id,
    required this.path,
    required this.title,
    required this.snippet,
  });

  final String id;
  final String path;
  final String title;
  final String snippet;

  factory SearchHit.fromJson(Map<String, dynamic> j) => SearchHit(
    id: j['id'] as String,
    path: j['path'] as String,
    title: j['title'] as String? ?? '',
    snippet: j['snippet'] as String? ?? '',
  );
}

/// A tag and how many notes carry it.
class TagCount {
  const TagCount({required this.tag, required this.count});

  final String tag;
  final int count;

  /// Tags are hierarchical in Obsidian (`proj/sub`), so the browser groups on
  /// the segment before the first slash.
  String get topLevel => tag.split('/').first;

  factory TagCount.fromJson(Map<String, dynamic> j) =>
      TagCount(tag: j['tag'] as String, count: (j['count'] as num).toInt());
}

/// Raised for any non-2xx response, carrying the server's message.
class StormApiException implements Exception {
  StormApiException(this.statusCode, this.message);

  final int statusCode;
  final String message;

  bool get isUnauthorized => statusCode == 401;
  bool get isNotFound => statusCode == 404;

  @override
  String toString() => 'StormApiException($statusCode): $message';
}

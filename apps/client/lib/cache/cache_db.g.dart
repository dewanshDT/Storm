// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'cache_db.dart';

// ignore_for_file: type=lint
class $CachedNotesTable extends CachedNotes
    with TableInfo<$CachedNotesTable, CachedNote> {
  @override
  final GeneratedDatabase attachedDatabase;
  final String? _alias;
  $CachedNotesTable(this.attachedDatabase, [this._alias]);
  static const VerificationMeta _vaultIdMeta = const VerificationMeta(
    'vaultId',
  );
  @override
  late final GeneratedColumn<String> vaultId = GeneratedColumn<String>(
    'vault_id',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
    defaultValue: const Constant(kLegacyVault),
  );
  static const VerificationMeta _idMeta = const VerificationMeta('id');
  @override
  late final GeneratedColumn<String> id = GeneratedColumn<String>(
    'id',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _pathMeta = const VerificationMeta('path');
  @override
  late final GeneratedColumn<String> path = GeneratedColumn<String>(
    'path',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _titleMeta = const VerificationMeta('title');
  @override
  late final GeneratedColumn<String> title = GeneratedColumn<String>(
    'title',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
    defaultValue: const Constant(''),
  );
  static const VerificationMeta _versionMeta = const VerificationMeta(
    'version',
  );
  @override
  late final GeneratedColumn<int> version = GeneratedColumn<int>(
    'version',
    aliasedName,
    false,
    type: DriftSqlType.int,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _contentMeta = const VerificationMeta(
    'content',
  );
  @override
  late final GeneratedColumn<String> content = GeneratedColumn<String>(
    'content',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _modifiedMeta = const VerificationMeta(
    'modified',
  );
  @override
  late final GeneratedColumn<String> modified = GeneratedColumn<String>(
    'modified',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
    defaultValue: const Constant(''),
  );
  static const VerificationMeta _cachedAtMeta = const VerificationMeta(
    'cachedAt',
  );
  @override
  late final GeneratedColumn<DateTime> cachedAt = GeneratedColumn<DateTime>(
    'cached_at',
    aliasedName,
    false,
    type: DriftSqlType.dateTime,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _pinnedMeta = const VerificationMeta('pinned');
  @override
  late final GeneratedColumn<bool> pinned = GeneratedColumn<bool>(
    'pinned',
    aliasedName,
    false,
    type: DriftSqlType.bool,
    requiredDuringInsert: false,
    defaultConstraints: GeneratedColumn.constraintIsAlways(
      'CHECK ("pinned" IN (0, 1))',
    ),
    defaultValue: const Constant(false),
  );
  @override
  List<GeneratedColumn> get $columns => [
    vaultId,
    id,
    path,
    title,
    version,
    content,
    modified,
    cachedAt,
    pinned,
  ];
  @override
  String get aliasedName => _alias ?? actualTableName;
  @override
  String get actualTableName => $name;
  static const String $name = 'cached_notes';
  @override
  VerificationContext validateIntegrity(
    Insertable<CachedNote> instance, {
    bool isInserting = false,
  }) {
    final context = VerificationContext();
    final data = instance.toColumns(true);
    if (data.containsKey('vault_id')) {
      context.handle(
        _vaultIdMeta,
        vaultId.isAcceptableOrUnknown(data['vault_id']!, _vaultIdMeta),
      );
    }
    if (data.containsKey('id')) {
      context.handle(_idMeta, id.isAcceptableOrUnknown(data['id']!, _idMeta));
    } else if (isInserting) {
      context.missing(_idMeta);
    }
    if (data.containsKey('path')) {
      context.handle(
        _pathMeta,
        path.isAcceptableOrUnknown(data['path']!, _pathMeta),
      );
    } else if (isInserting) {
      context.missing(_pathMeta);
    }
    if (data.containsKey('title')) {
      context.handle(
        _titleMeta,
        title.isAcceptableOrUnknown(data['title']!, _titleMeta),
      );
    }
    if (data.containsKey('version')) {
      context.handle(
        _versionMeta,
        version.isAcceptableOrUnknown(data['version']!, _versionMeta),
      );
    } else if (isInserting) {
      context.missing(_versionMeta);
    }
    if (data.containsKey('content')) {
      context.handle(
        _contentMeta,
        content.isAcceptableOrUnknown(data['content']!, _contentMeta),
      );
    } else if (isInserting) {
      context.missing(_contentMeta);
    }
    if (data.containsKey('modified')) {
      context.handle(
        _modifiedMeta,
        modified.isAcceptableOrUnknown(data['modified']!, _modifiedMeta),
      );
    }
    if (data.containsKey('cached_at')) {
      context.handle(
        _cachedAtMeta,
        cachedAt.isAcceptableOrUnknown(data['cached_at']!, _cachedAtMeta),
      );
    } else if (isInserting) {
      context.missing(_cachedAtMeta);
    }
    if (data.containsKey('pinned')) {
      context.handle(
        _pinnedMeta,
        pinned.isAcceptableOrUnknown(data['pinned']!, _pinnedMeta),
      );
    }
    return context;
  }

  @override
  Set<GeneratedColumn> get $primaryKey => {vaultId, id};
  @override
  CachedNote map(Map<String, dynamic> data, {String? tablePrefix}) {
    final effectivePrefix = tablePrefix != null ? '$tablePrefix.' : '';
    return CachedNote(
      vaultId: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}vault_id'],
      )!,
      id: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}id'],
      )!,
      path: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}path'],
      )!,
      title: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}title'],
      )!,
      version: attachedDatabase.typeMapping.read(
        DriftSqlType.int,
        data['${effectivePrefix}version'],
      )!,
      content: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}content'],
      )!,
      modified: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}modified'],
      )!,
      cachedAt: attachedDatabase.typeMapping.read(
        DriftSqlType.dateTime,
        data['${effectivePrefix}cached_at'],
      )!,
      pinned: attachedDatabase.typeMapping.read(
        DriftSqlType.bool,
        data['${effectivePrefix}pinned'],
      )!,
    );
  }

  @override
  $CachedNotesTable createAlias(String alias) {
    return $CachedNotesTable(attachedDatabase, alias);
  }
}

class CachedNote extends DataClass implements Insertable<CachedNote> {
  /// Which vault this note belongs to.
  ///
  /// Part of the primary key: note ids are unique per vault, not per server,
  /// and two vaults routinely hold the same *path*.
  final String vaultId;
  final String id;
  final String path;
  final String title;

  /// The server version this content came from. Sent as `base_version` on the
  /// next save, which is what lets the server merge rather than clobber.
  final int version;
  final String content;
  final String modified;
  final DateTime cachedAt;

  /// User asked to keep this available offline, so eviction must skip it.
  final bool pinned;
  const CachedNote({
    required this.vaultId,
    required this.id,
    required this.path,
    required this.title,
    required this.version,
    required this.content,
    required this.modified,
    required this.cachedAt,
    required this.pinned,
  });
  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    map['vault_id'] = Variable<String>(vaultId);
    map['id'] = Variable<String>(id);
    map['path'] = Variable<String>(path);
    map['title'] = Variable<String>(title);
    map['version'] = Variable<int>(version);
    map['content'] = Variable<String>(content);
    map['modified'] = Variable<String>(modified);
    map['cached_at'] = Variable<DateTime>(cachedAt);
    map['pinned'] = Variable<bool>(pinned);
    return map;
  }

  CachedNotesCompanion toCompanion(bool nullToAbsent) {
    return CachedNotesCompanion(
      vaultId: Value(vaultId),
      id: Value(id),
      path: Value(path),
      title: Value(title),
      version: Value(version),
      content: Value(content),
      modified: Value(modified),
      cachedAt: Value(cachedAt),
      pinned: Value(pinned),
    );
  }

  factory CachedNote.fromJson(
    Map<String, dynamic> json, {
    ValueSerializer? serializer,
  }) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return CachedNote(
      vaultId: serializer.fromJson<String>(json['vaultId']),
      id: serializer.fromJson<String>(json['id']),
      path: serializer.fromJson<String>(json['path']),
      title: serializer.fromJson<String>(json['title']),
      version: serializer.fromJson<int>(json['version']),
      content: serializer.fromJson<String>(json['content']),
      modified: serializer.fromJson<String>(json['modified']),
      cachedAt: serializer.fromJson<DateTime>(json['cachedAt']),
      pinned: serializer.fromJson<bool>(json['pinned']),
    );
  }
  @override
  Map<String, dynamic> toJson({ValueSerializer? serializer}) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return <String, dynamic>{
      'vaultId': serializer.toJson<String>(vaultId),
      'id': serializer.toJson<String>(id),
      'path': serializer.toJson<String>(path),
      'title': serializer.toJson<String>(title),
      'version': serializer.toJson<int>(version),
      'content': serializer.toJson<String>(content),
      'modified': serializer.toJson<String>(modified),
      'cachedAt': serializer.toJson<DateTime>(cachedAt),
      'pinned': serializer.toJson<bool>(pinned),
    };
  }

  CachedNote copyWith({
    String? vaultId,
    String? id,
    String? path,
    String? title,
    int? version,
    String? content,
    String? modified,
    DateTime? cachedAt,
    bool? pinned,
  }) => CachedNote(
    vaultId: vaultId ?? this.vaultId,
    id: id ?? this.id,
    path: path ?? this.path,
    title: title ?? this.title,
    version: version ?? this.version,
    content: content ?? this.content,
    modified: modified ?? this.modified,
    cachedAt: cachedAt ?? this.cachedAt,
    pinned: pinned ?? this.pinned,
  );
  CachedNote copyWithCompanion(CachedNotesCompanion data) {
    return CachedNote(
      vaultId: data.vaultId.present ? data.vaultId.value : this.vaultId,
      id: data.id.present ? data.id.value : this.id,
      path: data.path.present ? data.path.value : this.path,
      title: data.title.present ? data.title.value : this.title,
      version: data.version.present ? data.version.value : this.version,
      content: data.content.present ? data.content.value : this.content,
      modified: data.modified.present ? data.modified.value : this.modified,
      cachedAt: data.cachedAt.present ? data.cachedAt.value : this.cachedAt,
      pinned: data.pinned.present ? data.pinned.value : this.pinned,
    );
  }

  @override
  String toString() {
    return (StringBuffer('CachedNote(')
          ..write('vaultId: $vaultId, ')
          ..write('id: $id, ')
          ..write('path: $path, ')
          ..write('title: $title, ')
          ..write('version: $version, ')
          ..write('content: $content, ')
          ..write('modified: $modified, ')
          ..write('cachedAt: $cachedAt, ')
          ..write('pinned: $pinned')
          ..write(')'))
        .toString();
  }

  @override
  int get hashCode => Object.hash(
    vaultId,
    id,
    path,
    title,
    version,
    content,
    modified,
    cachedAt,
    pinned,
  );
  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      (other is CachedNote &&
          other.vaultId == this.vaultId &&
          other.id == this.id &&
          other.path == this.path &&
          other.title == this.title &&
          other.version == this.version &&
          other.content == this.content &&
          other.modified == this.modified &&
          other.cachedAt == this.cachedAt &&
          other.pinned == this.pinned);
}

class CachedNotesCompanion extends UpdateCompanion<CachedNote> {
  final Value<String> vaultId;
  final Value<String> id;
  final Value<String> path;
  final Value<String> title;
  final Value<int> version;
  final Value<String> content;
  final Value<String> modified;
  final Value<DateTime> cachedAt;
  final Value<bool> pinned;
  final Value<int> rowid;
  const CachedNotesCompanion({
    this.vaultId = const Value.absent(),
    this.id = const Value.absent(),
    this.path = const Value.absent(),
    this.title = const Value.absent(),
    this.version = const Value.absent(),
    this.content = const Value.absent(),
    this.modified = const Value.absent(),
    this.cachedAt = const Value.absent(),
    this.pinned = const Value.absent(),
    this.rowid = const Value.absent(),
  });
  CachedNotesCompanion.insert({
    this.vaultId = const Value.absent(),
    required String id,
    required String path,
    this.title = const Value.absent(),
    required int version,
    required String content,
    this.modified = const Value.absent(),
    required DateTime cachedAt,
    this.pinned = const Value.absent(),
    this.rowid = const Value.absent(),
  }) : id = Value(id),
       path = Value(path),
       version = Value(version),
       content = Value(content),
       cachedAt = Value(cachedAt);
  static Insertable<CachedNote> custom({
    Expression<String>? vaultId,
    Expression<String>? id,
    Expression<String>? path,
    Expression<String>? title,
    Expression<int>? version,
    Expression<String>? content,
    Expression<String>? modified,
    Expression<DateTime>? cachedAt,
    Expression<bool>? pinned,
    Expression<int>? rowid,
  }) {
    return RawValuesInsertable({
      if (vaultId != null) 'vault_id': vaultId,
      if (id != null) 'id': id,
      if (path != null) 'path': path,
      if (title != null) 'title': title,
      if (version != null) 'version': version,
      if (content != null) 'content': content,
      if (modified != null) 'modified': modified,
      if (cachedAt != null) 'cached_at': cachedAt,
      if (pinned != null) 'pinned': pinned,
      if (rowid != null) 'rowid': rowid,
    });
  }

  CachedNotesCompanion copyWith({
    Value<String>? vaultId,
    Value<String>? id,
    Value<String>? path,
    Value<String>? title,
    Value<int>? version,
    Value<String>? content,
    Value<String>? modified,
    Value<DateTime>? cachedAt,
    Value<bool>? pinned,
    Value<int>? rowid,
  }) {
    return CachedNotesCompanion(
      vaultId: vaultId ?? this.vaultId,
      id: id ?? this.id,
      path: path ?? this.path,
      title: title ?? this.title,
      version: version ?? this.version,
      content: content ?? this.content,
      modified: modified ?? this.modified,
      cachedAt: cachedAt ?? this.cachedAt,
      pinned: pinned ?? this.pinned,
      rowid: rowid ?? this.rowid,
    );
  }

  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    if (vaultId.present) {
      map['vault_id'] = Variable<String>(vaultId.value);
    }
    if (id.present) {
      map['id'] = Variable<String>(id.value);
    }
    if (path.present) {
      map['path'] = Variable<String>(path.value);
    }
    if (title.present) {
      map['title'] = Variable<String>(title.value);
    }
    if (version.present) {
      map['version'] = Variable<int>(version.value);
    }
    if (content.present) {
      map['content'] = Variable<String>(content.value);
    }
    if (modified.present) {
      map['modified'] = Variable<String>(modified.value);
    }
    if (cachedAt.present) {
      map['cached_at'] = Variable<DateTime>(cachedAt.value);
    }
    if (pinned.present) {
      map['pinned'] = Variable<bool>(pinned.value);
    }
    if (rowid.present) {
      map['rowid'] = Variable<int>(rowid.value);
    }
    return map;
  }

  @override
  String toString() {
    return (StringBuffer('CachedNotesCompanion(')
          ..write('vaultId: $vaultId, ')
          ..write('id: $id, ')
          ..write('path: $path, ')
          ..write('title: $title, ')
          ..write('version: $version, ')
          ..write('content: $content, ')
          ..write('modified: $modified, ')
          ..write('cachedAt: $cachedAt, ')
          ..write('pinned: $pinned, ')
          ..write('rowid: $rowid')
          ..write(')'))
        .toString();
  }
}

class $OutboxTable extends Outbox with TableInfo<$OutboxTable, OutboxData> {
  @override
  final GeneratedDatabase attachedDatabase;
  final String? _alias;
  $OutboxTable(this.attachedDatabase, [this._alias]);
  static const VerificationMeta _vaultIdMeta = const VerificationMeta(
    'vaultId',
  );
  @override
  late final GeneratedColumn<String> vaultId = GeneratedColumn<String>(
    'vault_id',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
    defaultValue: const Constant(kLegacyVault),
  );
  static const VerificationMeta _noteIdMeta = const VerificationMeta('noteId');
  @override
  late final GeneratedColumn<String> noteId = GeneratedColumn<String>(
    'note_id',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _baseVersionMeta = const VerificationMeta(
    'baseVersion',
  );
  @override
  late final GeneratedColumn<int> baseVersion = GeneratedColumn<int>(
    'base_version',
    aliasedName,
    false,
    type: DriftSqlType.int,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _contentMeta = const VerificationMeta(
    'content',
  );
  @override
  late final GeneratedColumn<String> content = GeneratedColumn<String>(
    'content',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _opMeta = const VerificationMeta('op');
  @override
  late final GeneratedColumn<String> op = GeneratedColumn<String>(
    'op',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
    defaultValue: const Constant('update'),
  );
  static const VerificationMeta _newPathMeta = const VerificationMeta(
    'newPath',
  );
  @override
  late final GeneratedColumn<String> newPath = GeneratedColumn<String>(
    'new_path',
    aliasedName,
    true,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
  );
  static const VerificationMeta _queuedAtMeta = const VerificationMeta(
    'queuedAt',
  );
  @override
  late final GeneratedColumn<DateTime> queuedAt = GeneratedColumn<DateTime>(
    'queued_at',
    aliasedName,
    false,
    type: DriftSqlType.dateTime,
    requiredDuringInsert: true,
  );
  @override
  List<GeneratedColumn> get $columns => [
    vaultId,
    noteId,
    baseVersion,
    content,
    op,
    newPath,
    queuedAt,
  ];
  @override
  String get aliasedName => _alias ?? actualTableName;
  @override
  String get actualTableName => $name;
  static const String $name = 'outbox';
  @override
  VerificationContext validateIntegrity(
    Insertable<OutboxData> instance, {
    bool isInserting = false,
  }) {
    final context = VerificationContext();
    final data = instance.toColumns(true);
    if (data.containsKey('vault_id')) {
      context.handle(
        _vaultIdMeta,
        vaultId.isAcceptableOrUnknown(data['vault_id']!, _vaultIdMeta),
      );
    }
    if (data.containsKey('note_id')) {
      context.handle(
        _noteIdMeta,
        noteId.isAcceptableOrUnknown(data['note_id']!, _noteIdMeta),
      );
    } else if (isInserting) {
      context.missing(_noteIdMeta);
    }
    if (data.containsKey('base_version')) {
      context.handle(
        _baseVersionMeta,
        baseVersion.isAcceptableOrUnknown(
          data['base_version']!,
          _baseVersionMeta,
        ),
      );
    } else if (isInserting) {
      context.missing(_baseVersionMeta);
    }
    if (data.containsKey('content')) {
      context.handle(
        _contentMeta,
        content.isAcceptableOrUnknown(data['content']!, _contentMeta),
      );
    } else if (isInserting) {
      context.missing(_contentMeta);
    }
    if (data.containsKey('op')) {
      context.handle(_opMeta, op.isAcceptableOrUnknown(data['op']!, _opMeta));
    }
    if (data.containsKey('new_path')) {
      context.handle(
        _newPathMeta,
        newPath.isAcceptableOrUnknown(data['new_path']!, _newPathMeta),
      );
    }
    if (data.containsKey('queued_at')) {
      context.handle(
        _queuedAtMeta,
        queuedAt.isAcceptableOrUnknown(data['queued_at']!, _queuedAtMeta),
      );
    } else if (isInserting) {
      context.missing(_queuedAtMeta);
    }
    return context;
  }

  @override
  Set<GeneratedColumn> get $primaryKey => {vaultId, noteId};
  @override
  OutboxData map(Map<String, dynamic> data, {String? tablePrefix}) {
    final effectivePrefix = tablePrefix != null ? '$tablePrefix.' : '';
    return OutboxData(
      vaultId: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}vault_id'],
      )!,
      noteId: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}note_id'],
      )!,
      baseVersion: attachedDatabase.typeMapping.read(
        DriftSqlType.int,
        data['${effectivePrefix}base_version'],
      )!,
      content: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}content'],
      )!,
      op: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}op'],
      )!,
      newPath: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}new_path'],
      ),
      queuedAt: attachedDatabase.typeMapping.read(
        DriftSqlType.dateTime,
        data['${effectivePrefix}queued_at'],
      )!,
    );
  }

  @override
  $OutboxTable createAlias(String alias) {
    return $OutboxTable(attachedDatabase, alias);
  }
}

class OutboxData extends DataClass implements Insertable<OutboxData> {
  final String vaultId;
  final String noteId;
  final int baseVersion;
  final String content;

  /// `update`, `move` or `delete`. Creates are sent immediately and never
  /// queued, because the client cannot invent a server-assigned id.
  final String op;

  /// Destination for a queued `move`. Null for every other op.
  final String? newPath;
  final DateTime queuedAt;
  const OutboxData({
    required this.vaultId,
    required this.noteId,
    required this.baseVersion,
    required this.content,
    required this.op,
    this.newPath,
    required this.queuedAt,
  });
  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    map['vault_id'] = Variable<String>(vaultId);
    map['note_id'] = Variable<String>(noteId);
    map['base_version'] = Variable<int>(baseVersion);
    map['content'] = Variable<String>(content);
    map['op'] = Variable<String>(op);
    if (!nullToAbsent || newPath != null) {
      map['new_path'] = Variable<String>(newPath);
    }
    map['queued_at'] = Variable<DateTime>(queuedAt);
    return map;
  }

  OutboxCompanion toCompanion(bool nullToAbsent) {
    return OutboxCompanion(
      vaultId: Value(vaultId),
      noteId: Value(noteId),
      baseVersion: Value(baseVersion),
      content: Value(content),
      op: Value(op),
      newPath: newPath == null && nullToAbsent
          ? const Value.absent()
          : Value(newPath),
      queuedAt: Value(queuedAt),
    );
  }

  factory OutboxData.fromJson(
    Map<String, dynamic> json, {
    ValueSerializer? serializer,
  }) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return OutboxData(
      vaultId: serializer.fromJson<String>(json['vaultId']),
      noteId: serializer.fromJson<String>(json['noteId']),
      baseVersion: serializer.fromJson<int>(json['baseVersion']),
      content: serializer.fromJson<String>(json['content']),
      op: serializer.fromJson<String>(json['op']),
      newPath: serializer.fromJson<String?>(json['newPath']),
      queuedAt: serializer.fromJson<DateTime>(json['queuedAt']),
    );
  }
  @override
  Map<String, dynamic> toJson({ValueSerializer? serializer}) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return <String, dynamic>{
      'vaultId': serializer.toJson<String>(vaultId),
      'noteId': serializer.toJson<String>(noteId),
      'baseVersion': serializer.toJson<int>(baseVersion),
      'content': serializer.toJson<String>(content),
      'op': serializer.toJson<String>(op),
      'newPath': serializer.toJson<String?>(newPath),
      'queuedAt': serializer.toJson<DateTime>(queuedAt),
    };
  }

  OutboxData copyWith({
    String? vaultId,
    String? noteId,
    int? baseVersion,
    String? content,
    String? op,
    Value<String?> newPath = const Value.absent(),
    DateTime? queuedAt,
  }) => OutboxData(
    vaultId: vaultId ?? this.vaultId,
    noteId: noteId ?? this.noteId,
    baseVersion: baseVersion ?? this.baseVersion,
    content: content ?? this.content,
    op: op ?? this.op,
    newPath: newPath.present ? newPath.value : this.newPath,
    queuedAt: queuedAt ?? this.queuedAt,
  );
  OutboxData copyWithCompanion(OutboxCompanion data) {
    return OutboxData(
      vaultId: data.vaultId.present ? data.vaultId.value : this.vaultId,
      noteId: data.noteId.present ? data.noteId.value : this.noteId,
      baseVersion: data.baseVersion.present
          ? data.baseVersion.value
          : this.baseVersion,
      content: data.content.present ? data.content.value : this.content,
      op: data.op.present ? data.op.value : this.op,
      newPath: data.newPath.present ? data.newPath.value : this.newPath,
      queuedAt: data.queuedAt.present ? data.queuedAt.value : this.queuedAt,
    );
  }

  @override
  String toString() {
    return (StringBuffer('OutboxData(')
          ..write('vaultId: $vaultId, ')
          ..write('noteId: $noteId, ')
          ..write('baseVersion: $baseVersion, ')
          ..write('content: $content, ')
          ..write('op: $op, ')
          ..write('newPath: $newPath, ')
          ..write('queuedAt: $queuedAt')
          ..write(')'))
        .toString();
  }

  @override
  int get hashCode =>
      Object.hash(vaultId, noteId, baseVersion, content, op, newPath, queuedAt);
  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      (other is OutboxData &&
          other.vaultId == this.vaultId &&
          other.noteId == this.noteId &&
          other.baseVersion == this.baseVersion &&
          other.content == this.content &&
          other.op == this.op &&
          other.newPath == this.newPath &&
          other.queuedAt == this.queuedAt);
}

class OutboxCompanion extends UpdateCompanion<OutboxData> {
  final Value<String> vaultId;
  final Value<String> noteId;
  final Value<int> baseVersion;
  final Value<String> content;
  final Value<String> op;
  final Value<String?> newPath;
  final Value<DateTime> queuedAt;
  final Value<int> rowid;
  const OutboxCompanion({
    this.vaultId = const Value.absent(),
    this.noteId = const Value.absent(),
    this.baseVersion = const Value.absent(),
    this.content = const Value.absent(),
    this.op = const Value.absent(),
    this.newPath = const Value.absent(),
    this.queuedAt = const Value.absent(),
    this.rowid = const Value.absent(),
  });
  OutboxCompanion.insert({
    this.vaultId = const Value.absent(),
    required String noteId,
    required int baseVersion,
    required String content,
    this.op = const Value.absent(),
    this.newPath = const Value.absent(),
    required DateTime queuedAt,
    this.rowid = const Value.absent(),
  }) : noteId = Value(noteId),
       baseVersion = Value(baseVersion),
       content = Value(content),
       queuedAt = Value(queuedAt);
  static Insertable<OutboxData> custom({
    Expression<String>? vaultId,
    Expression<String>? noteId,
    Expression<int>? baseVersion,
    Expression<String>? content,
    Expression<String>? op,
    Expression<String>? newPath,
    Expression<DateTime>? queuedAt,
    Expression<int>? rowid,
  }) {
    return RawValuesInsertable({
      if (vaultId != null) 'vault_id': vaultId,
      if (noteId != null) 'note_id': noteId,
      if (baseVersion != null) 'base_version': baseVersion,
      if (content != null) 'content': content,
      if (op != null) 'op': op,
      if (newPath != null) 'new_path': newPath,
      if (queuedAt != null) 'queued_at': queuedAt,
      if (rowid != null) 'rowid': rowid,
    });
  }

  OutboxCompanion copyWith({
    Value<String>? vaultId,
    Value<String>? noteId,
    Value<int>? baseVersion,
    Value<String>? content,
    Value<String>? op,
    Value<String?>? newPath,
    Value<DateTime>? queuedAt,
    Value<int>? rowid,
  }) {
    return OutboxCompanion(
      vaultId: vaultId ?? this.vaultId,
      noteId: noteId ?? this.noteId,
      baseVersion: baseVersion ?? this.baseVersion,
      content: content ?? this.content,
      op: op ?? this.op,
      newPath: newPath ?? this.newPath,
      queuedAt: queuedAt ?? this.queuedAt,
      rowid: rowid ?? this.rowid,
    );
  }

  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    if (vaultId.present) {
      map['vault_id'] = Variable<String>(vaultId.value);
    }
    if (noteId.present) {
      map['note_id'] = Variable<String>(noteId.value);
    }
    if (baseVersion.present) {
      map['base_version'] = Variable<int>(baseVersion.value);
    }
    if (content.present) {
      map['content'] = Variable<String>(content.value);
    }
    if (op.present) {
      map['op'] = Variable<String>(op.value);
    }
    if (newPath.present) {
      map['new_path'] = Variable<String>(newPath.value);
    }
    if (queuedAt.present) {
      map['queued_at'] = Variable<DateTime>(queuedAt.value);
    }
    if (rowid.present) {
      map['rowid'] = Variable<int>(rowid.value);
    }
    return map;
  }

  @override
  String toString() {
    return (StringBuffer('OutboxCompanion(')
          ..write('vaultId: $vaultId, ')
          ..write('noteId: $noteId, ')
          ..write('baseVersion: $baseVersion, ')
          ..write('content: $content, ')
          ..write('op: $op, ')
          ..write('newPath: $newPath, ')
          ..write('queuedAt: $queuedAt, ')
          ..write('rowid: $rowid')
          ..write(')'))
        .toString();
  }
}

class $MetaTable extends Meta with TableInfo<$MetaTable, MetaData> {
  @override
  final GeneratedDatabase attachedDatabase;
  final String? _alias;
  $MetaTable(this.attachedDatabase, [this._alias]);
  static const VerificationMeta _keyMeta = const VerificationMeta('key');
  @override
  late final GeneratedColumn<String> key = GeneratedColumn<String>(
    'key',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _valueMeta = const VerificationMeta('value');
  @override
  late final GeneratedColumn<String> value = GeneratedColumn<String>(
    'value',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  @override
  List<GeneratedColumn> get $columns => [key, value];
  @override
  String get aliasedName => _alias ?? actualTableName;
  @override
  String get actualTableName => $name;
  static const String $name = 'meta';
  @override
  VerificationContext validateIntegrity(
    Insertable<MetaData> instance, {
    bool isInserting = false,
  }) {
    final context = VerificationContext();
    final data = instance.toColumns(true);
    if (data.containsKey('key')) {
      context.handle(
        _keyMeta,
        key.isAcceptableOrUnknown(data['key']!, _keyMeta),
      );
    } else if (isInserting) {
      context.missing(_keyMeta);
    }
    if (data.containsKey('value')) {
      context.handle(
        _valueMeta,
        value.isAcceptableOrUnknown(data['value']!, _valueMeta),
      );
    } else if (isInserting) {
      context.missing(_valueMeta);
    }
    return context;
  }

  @override
  Set<GeneratedColumn> get $primaryKey => {key};
  @override
  MetaData map(Map<String, dynamic> data, {String? tablePrefix}) {
    final effectivePrefix = tablePrefix != null ? '$tablePrefix.' : '';
    return MetaData(
      key: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}key'],
      )!,
      value: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}value'],
      )!,
    );
  }

  @override
  $MetaTable createAlias(String alias) {
    return $MetaTable(attachedDatabase, alias);
  }
}

class MetaData extends DataClass implements Insertable<MetaData> {
  final String key;
  final String value;
  const MetaData({required this.key, required this.value});
  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    map['key'] = Variable<String>(key);
    map['value'] = Variable<String>(value);
    return map;
  }

  MetaCompanion toCompanion(bool nullToAbsent) {
    return MetaCompanion(key: Value(key), value: Value(value));
  }

  factory MetaData.fromJson(
    Map<String, dynamic> json, {
    ValueSerializer? serializer,
  }) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return MetaData(
      key: serializer.fromJson<String>(json['key']),
      value: serializer.fromJson<String>(json['value']),
    );
  }
  @override
  Map<String, dynamic> toJson({ValueSerializer? serializer}) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return <String, dynamic>{
      'key': serializer.toJson<String>(key),
      'value': serializer.toJson<String>(value),
    };
  }

  MetaData copyWith({String? key, String? value}) =>
      MetaData(key: key ?? this.key, value: value ?? this.value);
  MetaData copyWithCompanion(MetaCompanion data) {
    return MetaData(
      key: data.key.present ? data.key.value : this.key,
      value: data.value.present ? data.value.value : this.value,
    );
  }

  @override
  String toString() {
    return (StringBuffer('MetaData(')
          ..write('key: $key, ')
          ..write('value: $value')
          ..write(')'))
        .toString();
  }

  @override
  int get hashCode => Object.hash(key, value);
  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      (other is MetaData && other.key == this.key && other.value == this.value);
}

class MetaCompanion extends UpdateCompanion<MetaData> {
  final Value<String> key;
  final Value<String> value;
  final Value<int> rowid;
  const MetaCompanion({
    this.key = const Value.absent(),
    this.value = const Value.absent(),
    this.rowid = const Value.absent(),
  });
  MetaCompanion.insert({
    required String key,
    required String value,
    this.rowid = const Value.absent(),
  }) : key = Value(key),
       value = Value(value);
  static Insertable<MetaData> custom({
    Expression<String>? key,
    Expression<String>? value,
    Expression<int>? rowid,
  }) {
    return RawValuesInsertable({
      if (key != null) 'key': key,
      if (value != null) 'value': value,
      if (rowid != null) 'rowid': rowid,
    });
  }

  MetaCompanion copyWith({
    Value<String>? key,
    Value<String>? value,
    Value<int>? rowid,
  }) {
    return MetaCompanion(
      key: key ?? this.key,
      value: value ?? this.value,
      rowid: rowid ?? this.rowid,
    );
  }

  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    if (key.present) {
      map['key'] = Variable<String>(key.value);
    }
    if (value.present) {
      map['value'] = Variable<String>(value.value);
    }
    if (rowid.present) {
      map['rowid'] = Variable<int>(rowid.value);
    }
    return map;
  }

  @override
  String toString() {
    return (StringBuffer('MetaCompanion(')
          ..write('key: $key, ')
          ..write('value: $value, ')
          ..write('rowid: $rowid')
          ..write(')'))
        .toString();
  }
}

class $RecentsTable extends Recents with TableInfo<$RecentsTable, Recent> {
  @override
  final GeneratedDatabase attachedDatabase;
  final String? _alias;
  $RecentsTable(this.attachedDatabase, [this._alias]);
  static const VerificationMeta _vaultIdMeta = const VerificationMeta(
    'vaultId',
  );
  @override
  late final GeneratedColumn<String> vaultId = GeneratedColumn<String>(
    'vault_id',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _noteIdMeta = const VerificationMeta('noteId');
  @override
  late final GeneratedColumn<String> noteId = GeneratedColumn<String>(
    'note_id',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _vaultNameMeta = const VerificationMeta(
    'vaultName',
  );
  @override
  late final GeneratedColumn<String> vaultName = GeneratedColumn<String>(
    'vault_name',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
    defaultValue: const Constant(''),
  );
  static const VerificationMeta _pathMeta = const VerificationMeta('path');
  @override
  late final GeneratedColumn<String> path = GeneratedColumn<String>(
    'path',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
    defaultValue: const Constant(''),
  );
  static const VerificationMeta _titleMeta = const VerificationMeta('title');
  @override
  late final GeneratedColumn<String> title = GeneratedColumn<String>(
    'title',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
    defaultValue: const Constant(''),
  );
  static const VerificationMeta _openedAtMeta = const VerificationMeta(
    'openedAt',
  );
  @override
  late final GeneratedColumn<DateTime> openedAt = GeneratedColumn<DateTime>(
    'opened_at',
    aliasedName,
    false,
    type: DriftSqlType.dateTime,
    requiredDuringInsert: true,
  );
  @override
  List<GeneratedColumn> get $columns => [
    vaultId,
    noteId,
    vaultName,
    path,
    title,
    openedAt,
  ];
  @override
  String get aliasedName => _alias ?? actualTableName;
  @override
  String get actualTableName => $name;
  static const String $name = 'recents';
  @override
  VerificationContext validateIntegrity(
    Insertable<Recent> instance, {
    bool isInserting = false,
  }) {
    final context = VerificationContext();
    final data = instance.toColumns(true);
    if (data.containsKey('vault_id')) {
      context.handle(
        _vaultIdMeta,
        vaultId.isAcceptableOrUnknown(data['vault_id']!, _vaultIdMeta),
      );
    } else if (isInserting) {
      context.missing(_vaultIdMeta);
    }
    if (data.containsKey('note_id')) {
      context.handle(
        _noteIdMeta,
        noteId.isAcceptableOrUnknown(data['note_id']!, _noteIdMeta),
      );
    } else if (isInserting) {
      context.missing(_noteIdMeta);
    }
    if (data.containsKey('vault_name')) {
      context.handle(
        _vaultNameMeta,
        vaultName.isAcceptableOrUnknown(data['vault_name']!, _vaultNameMeta),
      );
    }
    if (data.containsKey('path')) {
      context.handle(
        _pathMeta,
        path.isAcceptableOrUnknown(data['path']!, _pathMeta),
      );
    }
    if (data.containsKey('title')) {
      context.handle(
        _titleMeta,
        title.isAcceptableOrUnknown(data['title']!, _titleMeta),
      );
    }
    if (data.containsKey('opened_at')) {
      context.handle(
        _openedAtMeta,
        openedAt.isAcceptableOrUnknown(data['opened_at']!, _openedAtMeta),
      );
    } else if (isInserting) {
      context.missing(_openedAtMeta);
    }
    return context;
  }

  @override
  Set<GeneratedColumn> get $primaryKey => {vaultId, noteId};
  @override
  Recent map(Map<String, dynamic> data, {String? tablePrefix}) {
    final effectivePrefix = tablePrefix != null ? '$tablePrefix.' : '';
    return Recent(
      vaultId: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}vault_id'],
      )!,
      noteId: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}note_id'],
      )!,
      vaultName: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}vault_name'],
      )!,
      path: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}path'],
      )!,
      title: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}title'],
      )!,
      openedAt: attachedDatabase.typeMapping.read(
        DriftSqlType.dateTime,
        data['${effectivePrefix}opened_at'],
      )!,
    );
  }

  @override
  $RecentsTable createAlias(String alias) {
    return $RecentsTable(attachedDatabase, alias);
  }
}

class Recent extends DataClass implements Insertable<Recent> {
  final String vaultId;
  final String noteId;
  final String vaultName;
  final String path;
  final String title;
  final DateTime openedAt;
  const Recent({
    required this.vaultId,
    required this.noteId,
    required this.vaultName,
    required this.path,
    required this.title,
    required this.openedAt,
  });
  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    map['vault_id'] = Variable<String>(vaultId);
    map['note_id'] = Variable<String>(noteId);
    map['vault_name'] = Variable<String>(vaultName);
    map['path'] = Variable<String>(path);
    map['title'] = Variable<String>(title);
    map['opened_at'] = Variable<DateTime>(openedAt);
    return map;
  }

  RecentsCompanion toCompanion(bool nullToAbsent) {
    return RecentsCompanion(
      vaultId: Value(vaultId),
      noteId: Value(noteId),
      vaultName: Value(vaultName),
      path: Value(path),
      title: Value(title),
      openedAt: Value(openedAt),
    );
  }

  factory Recent.fromJson(
    Map<String, dynamic> json, {
    ValueSerializer? serializer,
  }) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return Recent(
      vaultId: serializer.fromJson<String>(json['vaultId']),
      noteId: serializer.fromJson<String>(json['noteId']),
      vaultName: serializer.fromJson<String>(json['vaultName']),
      path: serializer.fromJson<String>(json['path']),
      title: serializer.fromJson<String>(json['title']),
      openedAt: serializer.fromJson<DateTime>(json['openedAt']),
    );
  }
  @override
  Map<String, dynamic> toJson({ValueSerializer? serializer}) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return <String, dynamic>{
      'vaultId': serializer.toJson<String>(vaultId),
      'noteId': serializer.toJson<String>(noteId),
      'vaultName': serializer.toJson<String>(vaultName),
      'path': serializer.toJson<String>(path),
      'title': serializer.toJson<String>(title),
      'openedAt': serializer.toJson<DateTime>(openedAt),
    };
  }

  Recent copyWith({
    String? vaultId,
    String? noteId,
    String? vaultName,
    String? path,
    String? title,
    DateTime? openedAt,
  }) => Recent(
    vaultId: vaultId ?? this.vaultId,
    noteId: noteId ?? this.noteId,
    vaultName: vaultName ?? this.vaultName,
    path: path ?? this.path,
    title: title ?? this.title,
    openedAt: openedAt ?? this.openedAt,
  );
  Recent copyWithCompanion(RecentsCompanion data) {
    return Recent(
      vaultId: data.vaultId.present ? data.vaultId.value : this.vaultId,
      noteId: data.noteId.present ? data.noteId.value : this.noteId,
      vaultName: data.vaultName.present ? data.vaultName.value : this.vaultName,
      path: data.path.present ? data.path.value : this.path,
      title: data.title.present ? data.title.value : this.title,
      openedAt: data.openedAt.present ? data.openedAt.value : this.openedAt,
    );
  }

  @override
  String toString() {
    return (StringBuffer('Recent(')
          ..write('vaultId: $vaultId, ')
          ..write('noteId: $noteId, ')
          ..write('vaultName: $vaultName, ')
          ..write('path: $path, ')
          ..write('title: $title, ')
          ..write('openedAt: $openedAt')
          ..write(')'))
        .toString();
  }

  @override
  int get hashCode =>
      Object.hash(vaultId, noteId, vaultName, path, title, openedAt);
  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      (other is Recent &&
          other.vaultId == this.vaultId &&
          other.noteId == this.noteId &&
          other.vaultName == this.vaultName &&
          other.path == this.path &&
          other.title == this.title &&
          other.openedAt == this.openedAt);
}

class RecentsCompanion extends UpdateCompanion<Recent> {
  final Value<String> vaultId;
  final Value<String> noteId;
  final Value<String> vaultName;
  final Value<String> path;
  final Value<String> title;
  final Value<DateTime> openedAt;
  final Value<int> rowid;
  const RecentsCompanion({
    this.vaultId = const Value.absent(),
    this.noteId = const Value.absent(),
    this.vaultName = const Value.absent(),
    this.path = const Value.absent(),
    this.title = const Value.absent(),
    this.openedAt = const Value.absent(),
    this.rowid = const Value.absent(),
  });
  RecentsCompanion.insert({
    required String vaultId,
    required String noteId,
    this.vaultName = const Value.absent(),
    this.path = const Value.absent(),
    this.title = const Value.absent(),
    required DateTime openedAt,
    this.rowid = const Value.absent(),
  }) : vaultId = Value(vaultId),
       noteId = Value(noteId),
       openedAt = Value(openedAt);
  static Insertable<Recent> custom({
    Expression<String>? vaultId,
    Expression<String>? noteId,
    Expression<String>? vaultName,
    Expression<String>? path,
    Expression<String>? title,
    Expression<DateTime>? openedAt,
    Expression<int>? rowid,
  }) {
    return RawValuesInsertable({
      if (vaultId != null) 'vault_id': vaultId,
      if (noteId != null) 'note_id': noteId,
      if (vaultName != null) 'vault_name': vaultName,
      if (path != null) 'path': path,
      if (title != null) 'title': title,
      if (openedAt != null) 'opened_at': openedAt,
      if (rowid != null) 'rowid': rowid,
    });
  }

  RecentsCompanion copyWith({
    Value<String>? vaultId,
    Value<String>? noteId,
    Value<String>? vaultName,
    Value<String>? path,
    Value<String>? title,
    Value<DateTime>? openedAt,
    Value<int>? rowid,
  }) {
    return RecentsCompanion(
      vaultId: vaultId ?? this.vaultId,
      noteId: noteId ?? this.noteId,
      vaultName: vaultName ?? this.vaultName,
      path: path ?? this.path,
      title: title ?? this.title,
      openedAt: openedAt ?? this.openedAt,
      rowid: rowid ?? this.rowid,
    );
  }

  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    if (vaultId.present) {
      map['vault_id'] = Variable<String>(vaultId.value);
    }
    if (noteId.present) {
      map['note_id'] = Variable<String>(noteId.value);
    }
    if (vaultName.present) {
      map['vault_name'] = Variable<String>(vaultName.value);
    }
    if (path.present) {
      map['path'] = Variable<String>(path.value);
    }
    if (title.present) {
      map['title'] = Variable<String>(title.value);
    }
    if (openedAt.present) {
      map['opened_at'] = Variable<DateTime>(openedAt.value);
    }
    if (rowid.present) {
      map['rowid'] = Variable<int>(rowid.value);
    }
    return map;
  }

  @override
  String toString() {
    return (StringBuffer('RecentsCompanion(')
          ..write('vaultId: $vaultId, ')
          ..write('noteId: $noteId, ')
          ..write('vaultName: $vaultName, ')
          ..write('path: $path, ')
          ..write('title: $title, ')
          ..write('openedAt: $openedAt, ')
          ..write('rowid: $rowid')
          ..write(')'))
        .toString();
  }
}

abstract class _$CacheDb extends GeneratedDatabase {
  _$CacheDb(QueryExecutor e) : super(e);
  $CacheDbManager get managers => $CacheDbManager(this);
  late final $CachedNotesTable cachedNotes = $CachedNotesTable(this);
  late final $OutboxTable outbox = $OutboxTable(this);
  late final $MetaTable meta = $MetaTable(this);
  late final $RecentsTable recents = $RecentsTable(this);
  @override
  Iterable<TableInfo<Table, Object?>> get allTables =>
      allSchemaEntities.whereType<TableInfo<Table, Object?>>();
  @override
  List<DatabaseSchemaEntity> get allSchemaEntities => [
    cachedNotes,
    outbox,
    meta,
    recents,
  ];
}

typedef $$CachedNotesTableCreateCompanionBuilder =
    CachedNotesCompanion Function({
      Value<String> vaultId,
      required String id,
      required String path,
      Value<String> title,
      required int version,
      required String content,
      Value<String> modified,
      required DateTime cachedAt,
      Value<bool> pinned,
      Value<int> rowid,
    });
typedef $$CachedNotesTableUpdateCompanionBuilder =
    CachedNotesCompanion Function({
      Value<String> vaultId,
      Value<String> id,
      Value<String> path,
      Value<String> title,
      Value<int> version,
      Value<String> content,
      Value<String> modified,
      Value<DateTime> cachedAt,
      Value<bool> pinned,
      Value<int> rowid,
    });

class $$CachedNotesTableFilterComposer
    extends Composer<_$CacheDb, $CachedNotesTable> {
  $$CachedNotesTableFilterComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnFilters<String> get vaultId => $composableBuilder(
    column: $table.vaultId,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get id => $composableBuilder(
    column: $table.id,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get path => $composableBuilder(
    column: $table.path,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get title => $composableBuilder(
    column: $table.title,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<int> get version => $composableBuilder(
    column: $table.version,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get content => $composableBuilder(
    column: $table.content,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get modified => $composableBuilder(
    column: $table.modified,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<DateTime> get cachedAt => $composableBuilder(
    column: $table.cachedAt,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<bool> get pinned => $composableBuilder(
    column: $table.pinned,
    builder: (column) => ColumnFilters(column),
  );
}

class $$CachedNotesTableOrderingComposer
    extends Composer<_$CacheDb, $CachedNotesTable> {
  $$CachedNotesTableOrderingComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnOrderings<String> get vaultId => $composableBuilder(
    column: $table.vaultId,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get id => $composableBuilder(
    column: $table.id,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get path => $composableBuilder(
    column: $table.path,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get title => $composableBuilder(
    column: $table.title,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<int> get version => $composableBuilder(
    column: $table.version,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get content => $composableBuilder(
    column: $table.content,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get modified => $composableBuilder(
    column: $table.modified,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<DateTime> get cachedAt => $composableBuilder(
    column: $table.cachedAt,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<bool> get pinned => $composableBuilder(
    column: $table.pinned,
    builder: (column) => ColumnOrderings(column),
  );
}

class $$CachedNotesTableAnnotationComposer
    extends Composer<_$CacheDb, $CachedNotesTable> {
  $$CachedNotesTableAnnotationComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  GeneratedColumn<String> get vaultId =>
      $composableBuilder(column: $table.vaultId, builder: (column) => column);

  GeneratedColumn<String> get id =>
      $composableBuilder(column: $table.id, builder: (column) => column);

  GeneratedColumn<String> get path =>
      $composableBuilder(column: $table.path, builder: (column) => column);

  GeneratedColumn<String> get title =>
      $composableBuilder(column: $table.title, builder: (column) => column);

  GeneratedColumn<int> get version =>
      $composableBuilder(column: $table.version, builder: (column) => column);

  GeneratedColumn<String> get content =>
      $composableBuilder(column: $table.content, builder: (column) => column);

  GeneratedColumn<String> get modified =>
      $composableBuilder(column: $table.modified, builder: (column) => column);

  GeneratedColumn<DateTime> get cachedAt =>
      $composableBuilder(column: $table.cachedAt, builder: (column) => column);

  GeneratedColumn<bool> get pinned =>
      $composableBuilder(column: $table.pinned, builder: (column) => column);
}

class $$CachedNotesTableTableManager
    extends
        RootTableManager<
          _$CacheDb,
          $CachedNotesTable,
          CachedNote,
          $$CachedNotesTableFilterComposer,
          $$CachedNotesTableOrderingComposer,
          $$CachedNotesTableAnnotationComposer,
          $$CachedNotesTableCreateCompanionBuilder,
          $$CachedNotesTableUpdateCompanionBuilder,
          (
            CachedNote,
            BaseReferences<_$CacheDb, $CachedNotesTable, CachedNote>,
          ),
          CachedNote,
          PrefetchHooks Function()
        > {
  $$CachedNotesTableTableManager(_$CacheDb db, $CachedNotesTable table)
    : super(
        TableManagerState(
          db: db,
          table: table,
          createFilteringComposer: () =>
              $$CachedNotesTableFilterComposer($db: db, $table: table),
          createOrderingComposer: () =>
              $$CachedNotesTableOrderingComposer($db: db, $table: table),
          createComputedFieldComposer: () =>
              $$CachedNotesTableAnnotationComposer($db: db, $table: table),
          updateCompanionCallback:
              ({
                Value<String> vaultId = const Value.absent(),
                Value<String> id = const Value.absent(),
                Value<String> path = const Value.absent(),
                Value<String> title = const Value.absent(),
                Value<int> version = const Value.absent(),
                Value<String> content = const Value.absent(),
                Value<String> modified = const Value.absent(),
                Value<DateTime> cachedAt = const Value.absent(),
                Value<bool> pinned = const Value.absent(),
                Value<int> rowid = const Value.absent(),
              }) => CachedNotesCompanion(
                vaultId: vaultId,
                id: id,
                path: path,
                title: title,
                version: version,
                content: content,
                modified: modified,
                cachedAt: cachedAt,
                pinned: pinned,
                rowid: rowid,
              ),
          createCompanionCallback:
              ({
                Value<String> vaultId = const Value.absent(),
                required String id,
                required String path,
                Value<String> title = const Value.absent(),
                required int version,
                required String content,
                Value<String> modified = const Value.absent(),
                required DateTime cachedAt,
                Value<bool> pinned = const Value.absent(),
                Value<int> rowid = const Value.absent(),
              }) => CachedNotesCompanion.insert(
                vaultId: vaultId,
                id: id,
                path: path,
                title: title,
                version: version,
                content: content,
                modified: modified,
                cachedAt: cachedAt,
                pinned: pinned,
                rowid: rowid,
              ),
          withReferenceMapper: (p0) => p0
              .map((e) => (e.readTable(table), BaseReferences(db, table, e)))
              .toList(),
          prefetchHooksCallback: null,
        ),
      );
}

typedef $$CachedNotesTableProcessedTableManager =
    ProcessedTableManager<
      _$CacheDb,
      $CachedNotesTable,
      CachedNote,
      $$CachedNotesTableFilterComposer,
      $$CachedNotesTableOrderingComposer,
      $$CachedNotesTableAnnotationComposer,
      $$CachedNotesTableCreateCompanionBuilder,
      $$CachedNotesTableUpdateCompanionBuilder,
      (CachedNote, BaseReferences<_$CacheDb, $CachedNotesTable, CachedNote>),
      CachedNote,
      PrefetchHooks Function()
    >;
typedef $$OutboxTableCreateCompanionBuilder =
    OutboxCompanion Function({
      Value<String> vaultId,
      required String noteId,
      required int baseVersion,
      required String content,
      Value<String> op,
      Value<String?> newPath,
      required DateTime queuedAt,
      Value<int> rowid,
    });
typedef $$OutboxTableUpdateCompanionBuilder =
    OutboxCompanion Function({
      Value<String> vaultId,
      Value<String> noteId,
      Value<int> baseVersion,
      Value<String> content,
      Value<String> op,
      Value<String?> newPath,
      Value<DateTime> queuedAt,
      Value<int> rowid,
    });

class $$OutboxTableFilterComposer extends Composer<_$CacheDb, $OutboxTable> {
  $$OutboxTableFilterComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnFilters<String> get vaultId => $composableBuilder(
    column: $table.vaultId,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get noteId => $composableBuilder(
    column: $table.noteId,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<int> get baseVersion => $composableBuilder(
    column: $table.baseVersion,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get content => $composableBuilder(
    column: $table.content,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get op => $composableBuilder(
    column: $table.op,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get newPath => $composableBuilder(
    column: $table.newPath,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<DateTime> get queuedAt => $composableBuilder(
    column: $table.queuedAt,
    builder: (column) => ColumnFilters(column),
  );
}

class $$OutboxTableOrderingComposer extends Composer<_$CacheDb, $OutboxTable> {
  $$OutboxTableOrderingComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnOrderings<String> get vaultId => $composableBuilder(
    column: $table.vaultId,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get noteId => $composableBuilder(
    column: $table.noteId,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<int> get baseVersion => $composableBuilder(
    column: $table.baseVersion,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get content => $composableBuilder(
    column: $table.content,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get op => $composableBuilder(
    column: $table.op,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get newPath => $composableBuilder(
    column: $table.newPath,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<DateTime> get queuedAt => $composableBuilder(
    column: $table.queuedAt,
    builder: (column) => ColumnOrderings(column),
  );
}

class $$OutboxTableAnnotationComposer
    extends Composer<_$CacheDb, $OutboxTable> {
  $$OutboxTableAnnotationComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  GeneratedColumn<String> get vaultId =>
      $composableBuilder(column: $table.vaultId, builder: (column) => column);

  GeneratedColumn<String> get noteId =>
      $composableBuilder(column: $table.noteId, builder: (column) => column);

  GeneratedColumn<int> get baseVersion => $composableBuilder(
    column: $table.baseVersion,
    builder: (column) => column,
  );

  GeneratedColumn<String> get content =>
      $composableBuilder(column: $table.content, builder: (column) => column);

  GeneratedColumn<String> get op =>
      $composableBuilder(column: $table.op, builder: (column) => column);

  GeneratedColumn<String> get newPath =>
      $composableBuilder(column: $table.newPath, builder: (column) => column);

  GeneratedColumn<DateTime> get queuedAt =>
      $composableBuilder(column: $table.queuedAt, builder: (column) => column);
}

class $$OutboxTableTableManager
    extends
        RootTableManager<
          _$CacheDb,
          $OutboxTable,
          OutboxData,
          $$OutboxTableFilterComposer,
          $$OutboxTableOrderingComposer,
          $$OutboxTableAnnotationComposer,
          $$OutboxTableCreateCompanionBuilder,
          $$OutboxTableUpdateCompanionBuilder,
          (OutboxData, BaseReferences<_$CacheDb, $OutboxTable, OutboxData>),
          OutboxData,
          PrefetchHooks Function()
        > {
  $$OutboxTableTableManager(_$CacheDb db, $OutboxTable table)
    : super(
        TableManagerState(
          db: db,
          table: table,
          createFilteringComposer: () =>
              $$OutboxTableFilterComposer($db: db, $table: table),
          createOrderingComposer: () =>
              $$OutboxTableOrderingComposer($db: db, $table: table),
          createComputedFieldComposer: () =>
              $$OutboxTableAnnotationComposer($db: db, $table: table),
          updateCompanionCallback:
              ({
                Value<String> vaultId = const Value.absent(),
                Value<String> noteId = const Value.absent(),
                Value<int> baseVersion = const Value.absent(),
                Value<String> content = const Value.absent(),
                Value<String> op = const Value.absent(),
                Value<String?> newPath = const Value.absent(),
                Value<DateTime> queuedAt = const Value.absent(),
                Value<int> rowid = const Value.absent(),
              }) => OutboxCompanion(
                vaultId: vaultId,
                noteId: noteId,
                baseVersion: baseVersion,
                content: content,
                op: op,
                newPath: newPath,
                queuedAt: queuedAt,
                rowid: rowid,
              ),
          createCompanionCallback:
              ({
                Value<String> vaultId = const Value.absent(),
                required String noteId,
                required int baseVersion,
                required String content,
                Value<String> op = const Value.absent(),
                Value<String?> newPath = const Value.absent(),
                required DateTime queuedAt,
                Value<int> rowid = const Value.absent(),
              }) => OutboxCompanion.insert(
                vaultId: vaultId,
                noteId: noteId,
                baseVersion: baseVersion,
                content: content,
                op: op,
                newPath: newPath,
                queuedAt: queuedAt,
                rowid: rowid,
              ),
          withReferenceMapper: (p0) => p0
              .map((e) => (e.readTable(table), BaseReferences(db, table, e)))
              .toList(),
          prefetchHooksCallback: null,
        ),
      );
}

typedef $$OutboxTableProcessedTableManager =
    ProcessedTableManager<
      _$CacheDb,
      $OutboxTable,
      OutboxData,
      $$OutboxTableFilterComposer,
      $$OutboxTableOrderingComposer,
      $$OutboxTableAnnotationComposer,
      $$OutboxTableCreateCompanionBuilder,
      $$OutboxTableUpdateCompanionBuilder,
      (OutboxData, BaseReferences<_$CacheDb, $OutboxTable, OutboxData>),
      OutboxData,
      PrefetchHooks Function()
    >;
typedef $$MetaTableCreateCompanionBuilder =
    MetaCompanion Function({
      required String key,
      required String value,
      Value<int> rowid,
    });
typedef $$MetaTableUpdateCompanionBuilder =
    MetaCompanion Function({
      Value<String> key,
      Value<String> value,
      Value<int> rowid,
    });

class $$MetaTableFilterComposer extends Composer<_$CacheDb, $MetaTable> {
  $$MetaTableFilterComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnFilters<String> get key => $composableBuilder(
    column: $table.key,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get value => $composableBuilder(
    column: $table.value,
    builder: (column) => ColumnFilters(column),
  );
}

class $$MetaTableOrderingComposer extends Composer<_$CacheDb, $MetaTable> {
  $$MetaTableOrderingComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnOrderings<String> get key => $composableBuilder(
    column: $table.key,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get value => $composableBuilder(
    column: $table.value,
    builder: (column) => ColumnOrderings(column),
  );
}

class $$MetaTableAnnotationComposer extends Composer<_$CacheDb, $MetaTable> {
  $$MetaTableAnnotationComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  GeneratedColumn<String> get key =>
      $composableBuilder(column: $table.key, builder: (column) => column);

  GeneratedColumn<String> get value =>
      $composableBuilder(column: $table.value, builder: (column) => column);
}

class $$MetaTableTableManager
    extends
        RootTableManager<
          _$CacheDb,
          $MetaTable,
          MetaData,
          $$MetaTableFilterComposer,
          $$MetaTableOrderingComposer,
          $$MetaTableAnnotationComposer,
          $$MetaTableCreateCompanionBuilder,
          $$MetaTableUpdateCompanionBuilder,
          (MetaData, BaseReferences<_$CacheDb, $MetaTable, MetaData>),
          MetaData,
          PrefetchHooks Function()
        > {
  $$MetaTableTableManager(_$CacheDb db, $MetaTable table)
    : super(
        TableManagerState(
          db: db,
          table: table,
          createFilteringComposer: () =>
              $$MetaTableFilterComposer($db: db, $table: table),
          createOrderingComposer: () =>
              $$MetaTableOrderingComposer($db: db, $table: table),
          createComputedFieldComposer: () =>
              $$MetaTableAnnotationComposer($db: db, $table: table),
          updateCompanionCallback:
              ({
                Value<String> key = const Value.absent(),
                Value<String> value = const Value.absent(),
                Value<int> rowid = const Value.absent(),
              }) => MetaCompanion(key: key, value: value, rowid: rowid),
          createCompanionCallback:
              ({
                required String key,
                required String value,
                Value<int> rowid = const Value.absent(),
              }) => MetaCompanion.insert(key: key, value: value, rowid: rowid),
          withReferenceMapper: (p0) => p0
              .map((e) => (e.readTable(table), BaseReferences(db, table, e)))
              .toList(),
          prefetchHooksCallback: null,
        ),
      );
}

typedef $$MetaTableProcessedTableManager =
    ProcessedTableManager<
      _$CacheDb,
      $MetaTable,
      MetaData,
      $$MetaTableFilterComposer,
      $$MetaTableOrderingComposer,
      $$MetaTableAnnotationComposer,
      $$MetaTableCreateCompanionBuilder,
      $$MetaTableUpdateCompanionBuilder,
      (MetaData, BaseReferences<_$CacheDb, $MetaTable, MetaData>),
      MetaData,
      PrefetchHooks Function()
    >;
typedef $$RecentsTableCreateCompanionBuilder =
    RecentsCompanion Function({
      required String vaultId,
      required String noteId,
      Value<String> vaultName,
      Value<String> path,
      Value<String> title,
      required DateTime openedAt,
      Value<int> rowid,
    });
typedef $$RecentsTableUpdateCompanionBuilder =
    RecentsCompanion Function({
      Value<String> vaultId,
      Value<String> noteId,
      Value<String> vaultName,
      Value<String> path,
      Value<String> title,
      Value<DateTime> openedAt,
      Value<int> rowid,
    });

class $$RecentsTableFilterComposer extends Composer<_$CacheDb, $RecentsTable> {
  $$RecentsTableFilterComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnFilters<String> get vaultId => $composableBuilder(
    column: $table.vaultId,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get noteId => $composableBuilder(
    column: $table.noteId,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get vaultName => $composableBuilder(
    column: $table.vaultName,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get path => $composableBuilder(
    column: $table.path,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get title => $composableBuilder(
    column: $table.title,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<DateTime> get openedAt => $composableBuilder(
    column: $table.openedAt,
    builder: (column) => ColumnFilters(column),
  );
}

class $$RecentsTableOrderingComposer
    extends Composer<_$CacheDb, $RecentsTable> {
  $$RecentsTableOrderingComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnOrderings<String> get vaultId => $composableBuilder(
    column: $table.vaultId,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get noteId => $composableBuilder(
    column: $table.noteId,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get vaultName => $composableBuilder(
    column: $table.vaultName,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get path => $composableBuilder(
    column: $table.path,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get title => $composableBuilder(
    column: $table.title,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<DateTime> get openedAt => $composableBuilder(
    column: $table.openedAt,
    builder: (column) => ColumnOrderings(column),
  );
}

class $$RecentsTableAnnotationComposer
    extends Composer<_$CacheDb, $RecentsTable> {
  $$RecentsTableAnnotationComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  GeneratedColumn<String> get vaultId =>
      $composableBuilder(column: $table.vaultId, builder: (column) => column);

  GeneratedColumn<String> get noteId =>
      $composableBuilder(column: $table.noteId, builder: (column) => column);

  GeneratedColumn<String> get vaultName =>
      $composableBuilder(column: $table.vaultName, builder: (column) => column);

  GeneratedColumn<String> get path =>
      $composableBuilder(column: $table.path, builder: (column) => column);

  GeneratedColumn<String> get title =>
      $composableBuilder(column: $table.title, builder: (column) => column);

  GeneratedColumn<DateTime> get openedAt =>
      $composableBuilder(column: $table.openedAt, builder: (column) => column);
}

class $$RecentsTableTableManager
    extends
        RootTableManager<
          _$CacheDb,
          $RecentsTable,
          Recent,
          $$RecentsTableFilterComposer,
          $$RecentsTableOrderingComposer,
          $$RecentsTableAnnotationComposer,
          $$RecentsTableCreateCompanionBuilder,
          $$RecentsTableUpdateCompanionBuilder,
          (Recent, BaseReferences<_$CacheDb, $RecentsTable, Recent>),
          Recent,
          PrefetchHooks Function()
        > {
  $$RecentsTableTableManager(_$CacheDb db, $RecentsTable table)
    : super(
        TableManagerState(
          db: db,
          table: table,
          createFilteringComposer: () =>
              $$RecentsTableFilterComposer($db: db, $table: table),
          createOrderingComposer: () =>
              $$RecentsTableOrderingComposer($db: db, $table: table),
          createComputedFieldComposer: () =>
              $$RecentsTableAnnotationComposer($db: db, $table: table),
          updateCompanionCallback:
              ({
                Value<String> vaultId = const Value.absent(),
                Value<String> noteId = const Value.absent(),
                Value<String> vaultName = const Value.absent(),
                Value<String> path = const Value.absent(),
                Value<String> title = const Value.absent(),
                Value<DateTime> openedAt = const Value.absent(),
                Value<int> rowid = const Value.absent(),
              }) => RecentsCompanion(
                vaultId: vaultId,
                noteId: noteId,
                vaultName: vaultName,
                path: path,
                title: title,
                openedAt: openedAt,
                rowid: rowid,
              ),
          createCompanionCallback:
              ({
                required String vaultId,
                required String noteId,
                Value<String> vaultName = const Value.absent(),
                Value<String> path = const Value.absent(),
                Value<String> title = const Value.absent(),
                required DateTime openedAt,
                Value<int> rowid = const Value.absent(),
              }) => RecentsCompanion.insert(
                vaultId: vaultId,
                noteId: noteId,
                vaultName: vaultName,
                path: path,
                title: title,
                openedAt: openedAt,
                rowid: rowid,
              ),
          withReferenceMapper: (p0) => p0
              .map((e) => (e.readTable(table), BaseReferences(db, table, e)))
              .toList(),
          prefetchHooksCallback: null,
        ),
      );
}

typedef $$RecentsTableProcessedTableManager =
    ProcessedTableManager<
      _$CacheDb,
      $RecentsTable,
      Recent,
      $$RecentsTableFilterComposer,
      $$RecentsTableOrderingComposer,
      $$RecentsTableAnnotationComposer,
      $$RecentsTableCreateCompanionBuilder,
      $$RecentsTableUpdateCompanionBuilder,
      (Recent, BaseReferences<_$CacheDb, $RecentsTable, Recent>),
      Recent,
      PrefetchHooks Function()
    >;

class $CacheDbManager {
  final _$CacheDb _db;
  $CacheDbManager(this._db);
  $$CachedNotesTableTableManager get cachedNotes =>
      $$CachedNotesTableTableManager(_db, _db.cachedNotes);
  $$OutboxTableTableManager get outbox =>
      $$OutboxTableTableManager(_db, _db.outbox);
  $$MetaTableTableManager get meta => $$MetaTableTableManager(_db, _db.meta);
  $$RecentsTableTableManager get recents =>
      $$RecentsTableTableManager(_db, _db.recents);
}

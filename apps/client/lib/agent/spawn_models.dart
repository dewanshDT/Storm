/// Agent spawning as the client sees it (decision 134; 138p corrects the
/// handoff's §6). Field names are the server's; every rule is the server's,
/// and nothing here decides anything.
library;

/// What a session's view says about spawning: its lineage (134d), whether
/// it waits for a person (134p), a child's own workspace (137) and the spawn
/// switch (134c). All absent on a server or host that has none of it, which
/// is every host until one passes its gates.
class SpawnFacts {
  const SpawnFacts({
    this.parentSession,
    this.rootSession,
    this.depth = 0,
    this.needsAttention,
    this.childWorkspace,
    this.switchState,
  });

  static const none = SpawnFacts();

  /// Who started it; null for a top-level session, a resumed child included
  /// (C7).
  final String? parentSession;
  final String? rootSession;
  final int depth;

  /// `approval`, `permission_prompt` or `blocked`; never from idle (A3).
  final String? needsAttention;

  /// A child's own workspace, once its host has made it.
  final ChildWorkspace? childWorkspace;

  /// The spawn switch, only where its host could offer spawning now or it is
  /// already on. Null everywhere else, and the client then shows none of it.
  final SpawnSwitch? switchState;

  bool get isChild => parentSession != null;

  factory SpawnFacts.fromJson(Map<String, dynamic> j) {
    final ws = j['child_workspace'];
    final sw = j['spawn'];
    final attention = j['needs_attention'];
    return SpawnFacts(
      parentSession: j['parent_session'] as String?,
      rootSession: j['root_session'] as String?,
      depth: (j['depth'] as num?)?.toInt() ?? 0,
      needsAttention: attention is String ? attention : null,
      childWorkspace: ws is Map<String, dynamic>
          ? ChildWorkspace.fromJson(ws)
          : null,
      switchState: sw is Map<String, dynamic> ? SpawnSwitch.fromJson(sw) : null,
    );
  }
}

/// The spawn switch on a session's view (134-10).
class SpawnSwitch {
  const SpawnSwitch({required this.enabled, this.refused});

  final bool enabled;

  /// What turning it on would be refused with now (134-2's codes), or null.
  final String? refused;

  factory SpawnSwitch.fromJson(Map<String, dynamic> j) => SpawnSwitch(
    enabled: j['enabled'] == true,
    refused: j['refused'] as String?,
  );
}

/// A child's own workspace (137b, 137c): a clone of its parent's last commit
/// or a scratch folder beside a read-only view of the parent's.
/// A child's workspace kind: `clone` (W2) or `scratch` (W3), which an
/// older or partial answer leaves as `scratch`.
mixin _WorkspaceMode {
  String get mode;

  bool get clone => mode == 'clone';
}

String _modeOf(Map<String, dynamic> j) => j['mode'] as String? ?? 'scratch';

class ChildWorkspace with _WorkspaceMode {
  const ChildWorkspace({
    required this.mode,
    this.commit,
    this.branch,
    this.submodules = false,
    this.largeFiles = false,
  });

  @override
  final String mode;
  final String? commit;
  final String? branch;
  final bool submodules;
  final bool largeFiles;

  factory ChildWorkspace.fromJson(Map<String, dynamic> j) => ChildWorkspace(
    mode: _modeOf(j),
    commit: j['commit'] as String?,
    branch: j['branch'] as String?,
    submodules: j['submodules'] == true,
    largeFiles: j['large_files'] == true,
  );
}

/// A start, message or end an agent asked for (134o), as its card shows it.
class SpawnRequest {
  const SpawnRequest({
    required this.id,
    required this.kind,
    required this.requester,
    required this.state,
    required this.createdAt,
    required this.expiresAt,
    this.target,
    this.outcome,
    this.reason,
    this.cap,
    this.session,
    this.decidedAt,
    this.task,
    this.name,
    this.contextTitle,
    this.provider,
    this.text,
  });

  final String id;

  /// `start`, `message` or `end`.
  final String kind;
  final String requester;
  final String? target;

  /// `pending`, `approving` or `decided`.
  final String state;
  final String createdAt;
  final String expiresAt;

  /// `started`, `start_failed`, `delivered`, `ended`, `denied`, `expired`
  /// or `refused`.
  final String? outcome;
  final String? reason;
  final String? cap;

  /// A start's child.
  final String? session;
  final String? decidedAt;

  /// Exactly what the agent sent: a start's task, a message's text.
  final String? task;
  final String? name;
  final String? contextTitle;
  final String? provider;
  final String? text;

  /// Waiting for a person, and not yet past its expiry.
  bool waiting(DateTime now) => state != 'decided' && left(now) > Duration.zero;

  Duration left(DateTime now) {
    final at = DateTime.tryParse(expiresAt);
    if (at == null) return Duration.zero;
    final d = at.difference(now);
    return d.isNegative ? Duration.zero : d;
  }

  factory SpawnRequest.fromJson(Map<String, dynamic> j) {
    final note = j['context_note'];
    return SpawnRequest(
      id: j['id'] as String,
      kind: j['kind'] as String? ?? 'start',
      requester: j['requester'] as String? ?? '',
      target: j['target'] as String?,
      state: j['state'] as String? ?? 'pending',
      createdAt: j['created_at'] as String? ?? '',
      expiresAt: j['expires_at'] as String? ?? '',
      outcome: j['outcome'] as String?,
      reason: j['reason'] as String?,
      cap: j['cap'] as String?,
      session: j['session'] as String?,
      decidedAt: j['decided_at'] as String?,
      task: j['task'] as String?,
      name: j['name'] as String?,
      contextTitle: note is Map ? note['title'] as String? : null,
      provider: j['provider'] as String?,
      text: j['text'] as String?,
    );
  }
}

/// A session under another, as ending it would stop it (C1).
class DescendantRef {
  const DescendantRef({
    required this.id,
    required this.name,
    required this.status,
    this.endReason,
  });

  final String id;
  final String name;
  final String status;
  final String? endReason;

  factory DescendantRef.fromJson(Map<String, dynamic> j) => DescendantRef(
    id: j['id'] as String,
    name: j['name'] as String? ?? j['id'] as String,
    status: j['status'] as String? ?? 'unknown',
    endReason: j['end_reason'] as String?,
  );
}

/// What ending a session, or ending its children, would stop.
class Descendants {
  const Descendants({required this.running, required this.ended});

  final List<DescendantRef> running;
  final List<DescendantRef> ended;

  factory Descendants.fromJson(Map<String, dynamic> j) => Descendants(
    running: [
      for (final d in j['running'] as List? ?? const [])
        DescendantRef.fromJson(d as Map<String, dynamic>),
    ],
    ended: [
      for (final d in j['ended'] as List? ?? const [])
        DescendantRef.fromJson(d as Map<String, dynamic>),
    ],
  );
}

/// One file in a child's work: a clone's changed file with its line counts
/// (null for a binary), or a scratch file with its size.
class ChildFile {
  const ChildFile({required this.path, this.added, this.removed, this.size});

  final String path;
  final int? added;
  final int? removed;
  final int? size;

  factory ChildFile.fromJson(Map<String, dynamic> j) => ChildFile(
    path: j['path'] as String? ?? '',
    added: (j['added'] as num?)?.toInt(),
    removed: (j['removed'] as num?)?.toInt(),
    size: (j['size'] as num?)?.toInt(),
  );
}

/// An ended child's work as data (137e), for a person to read before Apply.
class ChildWork with _WorkspaceMode {
  const ChildWork({
    required this.mode,
    this.tip,
    this.base,
    this.baseFound = true,
    this.commits = const [],
    this.moreCommits = false,
    this.files = const [],
    this.moreFiles = false,
    this.patch,
    this.patchTruncated = false,
    this.fileCount,
    this.bytes,
    this.skipped = 0,
  });

  @override
  final String mode;
  final String? tip;
  final String? base;
  final bool baseFound;
  final List<({String id, String subject})> commits;
  final bool moreCommits;
  final List<ChildFile> files;
  final bool moreFiles;
  final String? patch;
  final bool patchTruncated;
  final int? fileCount;
  final int? bytes;
  final int skipped;

  factory ChildWork.fromJson(Map<String, dynamic> j) => ChildWork(
    mode: _modeOf(j),
    tip: j['tip'] as String?,
    base: j['base'] as String?,
    baseFound: j['base_found'] != false,
    commits: [
      for (final c in j['commits'] as List? ?? const [])
        if (c is Map)
          (id: '${c['id'] ?? ''}', subject: '${c['subject'] ?? ''}'),
    ],
    moreCommits: j['more_commits'] == true,
    files: [
      for (final f in j['files'] as List? ?? const [])
        if (f is Map<String, dynamic>) ChildFile.fromJson(f),
    ],
    moreFiles: j['more_files'] == true,
    patch: j['patch'] as String?,
    patchTruncated: j['patch_truncated'] == true,
    fileCount: (j['file_count'] as num?)?.toInt(),
    bytes: (j['bytes'] as num?)?.toInt(),
    skipped: (j['skipped'] as num?)?.toInt() ?? 0,
  );
}

/// "abc1234": a commit as people read it.
String shortCommit(String? commit) =>
    commit == null ? '' : commit.substring(0, commit.length.clamp(0, 7));

/// What a session waiting for a person shows beside its activity (134p).
String? attentionLabel(String? kind) => switch (kind) {
  'approval' => 'Needs approval',
  'permission_prompt' => 'Waiting at a prompt',
  'blocked' => 'Blocked',
  _ => null,
};

/// Why the spawn switch would not turn on, as specific as the server is
/// (138p point 9).
String spawnRefusalText(String code, String agent) => switch (code) {
  'identity_unverified' => "This host's session identity isn't verified",
  'isolation_unverified' => "This host doesn't isolate sessions",
  'workspace_held' => 'Another session is working in this workspace',
  'provider_cannot_spawn' => "$agent can't start agents",
  'launched_without_asks' =>
    'It started before this host could offer it. Start a new session to '
        'allow it',
  'session_not_live' => 'The session has ended',
  _ => code.replaceAll('_', ' '),
};

/// The cap a request hit, in the decision's words (C4).
String capText(String? cap) => switch (cap) {
  'depth' => 'children can’t start agents yet',
  'children' => '3 live children',
  'tree' => '6 live sessions in a tree',
  'pending' => '2 waiting requests',
  'max_sessions' => 'the host’s session limit',
  'inbox' => 'the child’s inbox is full',
  _ => cap ?? 'a limit',
};

/// Why the server refused a request when a person approved it.
String requestRefusalText(String? reason, String? cap) => switch (reason) {
  'limit' => 'limit: ${capText(cap)}',
  'not_permitted' => 'not permitted for this session',
  'revoked' => 'starting agents was turned off for this session',
  'requester_ended' => 'the asking session ended',
  'target_gone' => 'the session it named is gone or has ended',
  'identity_unverified' => 'the host’s session identity isn’t verified',
  'isolation_unverified' => 'the host doesn’t isolate sessions',
  null => 'refused',
  _ => reason.replaceAll('_', ' '),
};

/// "identity verified · isolates sessions": a host's two states as the
/// card shows them (138p point 8: a state, never a fingerprint).
String hostStatesLine(String? identity, String? isolation) => [
  switch (identity) {
    'verified' => 'identity verified',
    'unsupported' => 'identity unsupported',
    _ => 'identity not verified',
  },
  isolation == 'verified' ? 'isolates sessions' : 'doesn’t isolate sessions',
].join(' · ');

/// "8:42": what is left of a request's ten minutes.
String clockLeft(Duration d) {
  final s = d.inSeconds;
  return '${s ~/ 60}:${(s % 60).toString().padLeft(2, '0')}';
}

/// What a scratch child's work is applied as by default: the parent's
/// workspace and the child, `<ws>-<child>` (138p point 9). A person may
/// rename it.
String scratchApplyName(String parentWorkspace, String child) {
  final base = parentWorkspace.split('/').where((p) => p.isNotEmpty).lastOrNull;
  return '${base ?? 'workspace'}-$child';
}

/// The Agent Runtime's records as the client sees them (`PLAN.md` decisions
/// 77c and 77d). Field names are the server's; nothing here is invented.
library;

/// A provider a host offers.
class ProviderCap {
  ProviderCap({required this.id, required this.kind, required this.available});

  final String id;
  final String kind;
  final bool available;

  factory ProviderCap.fromJson(Map<String, dynamic> j) => ProviderCap(
    id: j['id'] as String,
    kind: j['kind'] as String? ?? 'cli',
    available: j['available'] as bool? ?? false,
  );

  /// How a person reads the id.
  String get label => providerLabel(id);
}

/// `claude-code` → "Claude Code". Unknown ids are shown as they are.
String providerLabel(String id) => switch (id) {
  'claude-code' => 'Claude Code',
  'opencode' => 'OpenCode',
  'shell' => 'Shell',
  'fake' => 'Fake (test)',
  _ => id,
};

/// A Runtime Host: a machine that runs agents for this server.
class AgentHost {
  AgentHost({
    required this.id,
    required this.name,
    required this.status,
    required this.providers,
    required this.workspaces,
    required this.maxSessions,
    required this.lastSeen,
  });

  final String id;
  final String name;

  /// `online`, `offline` or `revoked`.
  final String status;
  final List<ProviderCap> providers;
  final List<String> workspaces;
  final int maxSessions;
  final String? lastSeen;

  bool get online => status == 'online';

  factory AgentHost.fromJson(Map<String, dynamic> j) {
    final caps = j['capabilities'] as Map<String, dynamic>?;
    return AgentHost(
      id: j['id'] as String,
      name: j['name'] as String,
      status: j['status'] as String? ?? 'offline',
      providers: [
        for (final p in (caps?['providers'] as List? ?? const []))
          ProviderCap.fromJson(p as Map<String, dynamic>),
      ],
      workspaces: [
        for (final w in (caps?['workspaces'] as List? ?? const [])) w as String,
      ],
      maxSessions: (caps?['max_sessions'] as num?)?.toInt() ?? 0,
      lastSeen: j['last_seen'] as String?,
    );
  }
}

/// A workspace on a host, with how many live sessions already share it.
class AgentWorkspace {
  AgentWorkspace({required this.name, required this.liveSessions});

  final String name;
  final int liveSessions;

  factory AgentWorkspace.fromJson(Map<String, dynamic> j) => AgentWorkspace(
    name: j['name'] as String,
    liveSessions: (j['live_sessions'] as num?)?.toInt() ?? 0,
  );
}

/// The provider the server used instead of the one asked for (freeze §6).
class ProviderFallback {
  ProviderFallback({required this.requested, required this.used});

  final String requested;
  final String used;

  factory ProviderFallback.fromJson(Map<String, dynamic> j) => ProviderFallback(
    requested: j['requested'] as String,
    used: j['used'] as String,
  );
}

/// The note a session was started from. [title] is the server's snapshot.
class SessionContext {
  const SessionContext({
    required this.vaultId,
    required this.noteId,
    required this.title,
  });

  final String vaultId;
  final String noteId;
  final String title;

  factory SessionContext.fromJson(Map<String, dynamic> j) => SessionContext(
    vaultId: j['vault_id'] as String,
    noteId: j['note_id'] as String,
    title: j['title'] as String? ?? '',
  );
}

/// An agent session (freeze §7.1), with what it was launched with
/// (decision 82, slice 5).
class AgentSession {
  AgentSession({
    required this.id,
    required this.hostId,
    required this.workspace,
    required this.provider,
    required this.status,
    required this.endReason,
    required this.exitCode,
    required this.signal,
    required this.cols,
    required this.rows,
    required this.createdAt,
    required this.fallback,
    this.startedAt,
    this.endedAt,
    String? name,
    this.context,
    this.writeVaultId,
    this.wroteCount = 0,
    this.integrations,
    this.launchNotice,
    this.title,
    this.activity,
  }) : name = name ?? workspace;

  final String id;
  final String hostId;
  final String workspace;
  final String provider;

  /// One vocabulary: creating, starting, running, completed, failed,
  /// stopped, unknown.
  final String status;
  final String? endReason;
  final int? exitCode;
  final int? signal;
  final int cols;
  final int rows;
  final String createdAt;
  final String? startedAt;
  final String? endedAt;
  final ProviderFallback? fallback;

  /// Assigned by the server (Q12), so it is the same on every device.
  final String name;

  /// The agent's own name for the session, from its terminal title (D15
  /// AM42); kept after it ends.
  final String? title;

  /// `working` or `idle` while running, when the agent says.
  final String? activity;

  /// What every surface calls the session.
  String get displayName => title ?? name;

  bool get working => activity == 'working';
  final SessionContext? context;

  /// The one vault it may write to; null is read only.
  final String? writeVaultId;
  final int wroteCount;

  /// The integrations it was granted at launch, named as then; null from a
  /// server that does not say.
  final List<String>? integrations;

  /// Said at launch when the session got less than it asked for. Only on the
  /// launch answer.
  final String? launchNotice;

  bool get ended =>
      status == 'completed' || status == 'failed' || status == 'stopped';

  bool get isShell => provider == 'shell';

  /// The handoff's status words (§3.2); why it ended is [endDetail].
  String get statusLabel => switch (status) {
    'creating' || 'starting' => 'Starting',
    'running' => 'Running',
    'unknown' => 'Unknown',
    'completed' => 'Completed',
    'stopped' => 'Stopped',
    'failed' => 'Failed',
    _ => status,
  };

  /// Why an ended session ended, when there is more to say than its status.
  String? get endDetail => switch (status) {
    'failed' => switch (endReason) {
      'host_restart' => 'host restarted',
      'host_revoked' => 'host revoked',
      'lost' => 'lost',
      'signal' => 'killed (signal ${signal ?? '?'})',
      _ => 'failed to start',
    },
    'completed' when exitCode != null && exitCode != 0 => 'exit $exitCode',
    _ => null,
  };

  /// This record with the status a live stream has seen since. The stream
  /// carries the bare record: no name, context or count.
  AgentSession withLive(AgentSession? live) => live == null || live.id != id
      ? this
      : AgentSession(
          id: id,
          hostId: hostId,
          workspace: workspace,
          provider: provider,
          status: live.status,
          endReason: live.endReason,
          exitCode: live.exitCode,
          signal: live.signal,
          cols: live.cols,
          rows: live.rows,
          createdAt: createdAt,
          fallback: fallback,
          startedAt: live.startedAt ?? startedAt,
          endedAt: live.endedAt ?? endedAt,
          name: name,
          context: context,
          writeVaultId: writeVaultId,
          wroteCount: wroteCount,
          integrations: integrations,
          title: live.title ?? title,
          activity: live.activity,
        );

  factory AgentSession.fromJson(Map<String, dynamic> j) => AgentSession(
    id: j['id'] as String,
    hostId: j['host_id'] as String,
    workspace: j['workspace'] as String,
    provider: j['provider'] as String,
    status: j['status'] as String,
    endReason: j['end_reason'] as String?,
    exitCode: (j['exit_code'] as num?)?.toInt(),
    signal: (j['signal'] as num?)?.toInt(),
    cols: (j['cols'] as num?)?.toInt() ?? 80,
    rows: (j['rows'] as num?)?.toInt() ?? 24,
    createdAt: j['created_at'] as String? ?? '',
    startedAt: j['started_at'] as String?,
    endedAt: j['ended_at'] as String?,
    fallback: j['provider_fallback'] == null
        ? null
        : ProviderFallback.fromJson(
            j['provider_fallback'] as Map<String, dynamic>,
          ),
    name: j['name'] as String?,
    context: j['context'] is Map<String, dynamic>
        ? SessionContext.fromJson(j['context'] as Map<String, dynamic>)
        : null,
    writeVaultId: j['write_vault_id'] as String?,
    wroteCount: (j['wrote_count'] as num?)?.toInt() ?? 0,
    integrations: j['integrations'] is List
        ? [
            for (final i in j['integrations'] as List)
              if (i is Map) '${i['display_name'] ?? i['slug'] ?? ''}',
          ]
        : null,
    launchNotice: (j['mcp'] as Map?)?['notice'] as String?,
    title: j['title'] as String?,
    activity: j['activity'] as String?,
  );
}

/// One thing a session wrote (`GET …/sessions/{id}/writes`): a note, or a
/// kit script. [title] and [path] are null once a note is gone, and a row
/// with no [noteId] has nothing to open.
class SessionWrite {
  const SessionWrite({
    required this.vaultId,
    required this.noteId,
    required this.title,
    required this.path,
    required this.kind,
    required this.version,
    required this.at,
  });

  final String? vaultId;
  final String? noteId;
  final String? title;
  final String? path;

  /// `created` or `edited` for a note; `script_created` or `script_edited`
  /// for a kit script, which has a path and no note.
  final String kind;
  final int? version;
  final String at;

  bool get created => kind == 'created' || kind == 'script_created';

  bool get isScript => kind.startsWith('script_');

  /// The note to open, if there is one.
  ({String vaultId, String noteId})? get note =>
      vaultId == null || noteId == null
      ? null
      : (vaultId: vaultId!, noteId: noteId!);

  factory SessionWrite.fromJson(Map<String, dynamic> j) => SessionWrite(
    vaultId: j['vault_id'] as String?,
    noteId: j['note_id'] as String?,
    title: j['title'] as String?,
    path: j['path'] as String?,
    kind: j['kind'] as String? ?? 'edited',
    version: (j['version'] as num?)?.toInt(),
    at: j['at'] as String? ?? '',
  );
}

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

/// An agent session (freeze §7.1).
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
    this.launchNotice,
  });

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
  final ProviderFallback? fallback;

  /// Said at launch when the session could have had integrations and does
  /// not: its host is too old to bridge them (MCP Gateway, spec §6). Only on
  /// the launch answer.
  final String? launchNotice;

  bool get ended =>
      status == 'completed' || status == 'failed' || status == 'stopped';

  /// What the status chip says. Plain words, never a raw code.
  String get statusLabel => switch (status) {
    'creating' || 'starting' => 'Starting',
    'running' => 'Running',
    'completed' =>
      exitCode == null || exitCode == 0 ? 'Finished' : 'Exited ($exitCode)',
    'stopped' => 'Ended',
    'unknown' => 'Host unreachable',
    'failed' => switch (endReason) {
      'host_restart' => 'Host restarted',
      'host_revoked' => 'Host revoked',
      'lost' => 'Lost',
      'signal' => 'Killed (signal ${signal ?? '?'})',
      _ => 'Failed to start',
    },
    _ => status,
  };

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
    fallback: j['provider_fallback'] == null
        ? null
        : ProviderFallback.fromJson(
            j['provider_fallback'] as Map<String, dynamic>,
          ),
    launchNotice: (j['mcp'] as Map?)?['notice'] as String?,
  );
}

/// The Agent Runtime's records as the client sees them (decisions 77c and
/// 77d). Field names are the server's; nothing here is invented.
library;

import '../ui/accents.dart';
import 'claude_login_api.dart' show claudeCredentialNotice;
import 'spawn_models.dart';

/// A provider a host offers.
class ProviderCap {
  ProviderCap({
    required this.id,
    required this.kind,
    required this.available,
    this.mcp,
    this.version,
    this.source,
    this.installable = false,
    this.installBlocked,
    this.checksumVerified = true,
    this.latest,
  });

  final String id;
  final String kind;
  final bool available;

  /// What the installed CLI reports; null when not installed or not probed
  /// yet (decision 133).
  final String? version;

  /// `storm` (Storm installed it and can update or remove it) or `system`
  /// (the owner's own install); null when not installed.
  final String? source;

  /// The host could install it now.
  final bool installable;

  /// Why it could not: `disabled`, `unsupported_platform`, `no_node` or
  /// `node_too_old`.
  final String? installBlocked;

  /// False for Cursor, whose downloads have no published checksum.
  final bool checksumVerified;

  /// The newest version available, when the server knows it.
  final String? latest;

  bool get managed => source == 'storm';

  /// The host can give its sessions Storm's MCP (decision 130a); null from a
  /// host too old to say, or for an agent it has no adapter for.
  final bool? mcp;

  factory ProviderCap.fromJson(Map<String, dynamic> j) => ProviderCap(
    id: j['id'] as String,
    kind: j['kind'] as String? ?? 'cli',
    available: j['available'] as bool? ?? false,
    mcp: j['mcp'] as bool?,
    version: j['version'] as String?,
    source: j['source'] as String?,
    installable: j['installable'] as bool? ?? false,
    installBlocked: j['install_blocked'] as String?,
    checksumVerified: j['checksum_verified'] as bool? ?? true,
    latest: j['latest'] as String?,
  );

  /// Its sessions can read your vaults and use your integrations: what the
  /// server grants, so `shell` never, and an agent the host cannot keep to
  /// its own MCP servers never (130a).
  bool get reachesStorm => mcp ?? id != 'shell';

  /// How a person reads the id.
  String get label => providerLabel(id);
}

/// `claude-code` → "Claude Code". Unknown ids are shown as they are.
String providerLabel(String id) => switch (id) {
  'claude-code' => 'Claude Code',
  'opencode' => 'OpenCode',
  'codex' => 'Codex',
  'cursor' => 'Cursor',
  'command-code' => 'Command Code',
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
    this.workspaceRoots = const [],
    this.resumes = const [],
    this.providerCredentials = const [],
    this.capabilitiesKnown = false,
    this.agentInstall = false,
    this.identityState,
    this.isolationState,
    this.isolationReason,
  });

  final String id;
  final String name;

  /// `online`, `offline` or `revoked`.
  final String status;
  final List<ProviderCap> providers;
  final List<String> workspaces;

  /// The host's folders for workspaces, empty for an old host.
  final List<String> workspaceRoots;

  /// The agents whose conversations it can reopen (decision 89); empty for
  /// an old host.
  final List<String> resumes;
  final int maxSessions;
  final String? lastSeen;

  /// The synced credentials it can pull (decision 124); empty for an old
  /// host.
  final List<String> providerCredentials;

  /// Whether it has said what it can do: an offline host has not.
  final bool capabilitiesKnown;

  /// It can install, update and remove agents (decision 133); false for an
  /// older runtime.
  final bool agentInstall;

  /// Its bridge identity (decision 135d): `verified`, `unsupported` or
  /// `unavailable`; null from a server that does not say.
  final String? identityState;

  /// Anything but `verified` is labelled (135d). The label is disclosure,
  /// not a control.
  bool get identityUnverified =>
      identityState != null && identityState != 'verified';

  /// Its session isolation (decision 136j): state and reason; null from a
  /// server that does not say.
  final String? isolationState;
  final String? isolationReason;

  /// What the host row says about it; nothing while isolation is off.
  String? get isolationLabel =>
      isolationLabelFor(isolationState, isolationReason);

  bool get online => status == 'online';

  /// C0 as its view says (134r, 141e): identity and isolation verified.
  bool get offersSpawning =>
      identityState == 'verified' && isolationState == 'verified';

  bool get receivesClaudeLogin =>
      providerCredentials.contains('claude_subscription_token');

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
      workspaceRoots: [
        for (final r in (caps?['workspace_roots'] as List? ?? const []))
          r as String,
      ],
      resumes: [
        for (final p in (caps?['resume'] as List? ?? const [])) p as String,
      ],
      providerCredentials: [
        for (final k in (caps?['provider_credentials'] as List? ?? const []))
          k as String,
      ],
      capabilitiesKnown: caps != null,
      agentInstall: caps?['agent_install'] == true,
      identityState: _identityState(j),
      isolationState: _isolation(j).$1,
      isolationReason: _isolation(j).$2,
    );
  }
}

String? _identityState(Map<String, dynamic> j) {
  final identity = j['identity'];
  return identity is Map ? '${identity['state'] ?? 'unavailable'}' : null;
}

(String?, String?) _isolation(Map<String, dynamic> j) {
  final isolation = j['isolation'];
  if (isolation is! Map) return (null, null);
  final reason = isolation['reason'];
  return (
    '${isolation['state'] ?? 'unavailable'}',
    reason is String ? reason : null,
  );
}

/// What Storm says about a host's session isolation (decision 136j). Nothing
/// where it is not on: off (the default), an older host, or a platform that
/// has none. Anything but `verified` with isolation on refuses new sessions.
String? isolationLabelFor(String? state, String? reason) {
  if (state == null || state == 'unsupported') return null;
  if (state == 'verified') return kIsolationVerified;
  if (reason == null || reason == 'off' || reason == 'old_host') return null;
  final why = switch (reason) {
    'pending' => 'self-test running',
    'self_test' => 'self-test failed',
    'no_passt' => 'pasta missing',
    'helper' => 'helper not answering',
    'no_helper' => 'no isolation helper',
    _ => reason.replaceAll('_', ' '),
  };
  return 'Session isolation unavailable ($why) — new sessions are refused';
}

/// A host whose sessions each run as their own user (decision 136j).
const kIsolationVerified = 'Sessions isolated — each runs as its own user';

/// What Storm says on a host or session whose identity is not verified
/// (decision 135d).
const kIdentityNotVerified =
    'Session identity not verified — sessions on this host can act as each '
    'other';

/// The agents a host can install from Settings › Agents, in the page's
/// order (decision 133).
const kInstallableAgents = [
  'claude-code',
  'opencode',
  'codex',
  'cursor',
  'command-code',
];

/// An install, update or removal of an agent on a host (decision 133).
class AgentJob {
  const AgentJob({
    required this.id,
    required this.hostId,
    required this.agent,
    required this.op,
    required this.state,
    this.step,
    this.error,
    this.version,
    this.created,
    this.updated,
  });

  final String id;
  final String hostId;
  final String agent;

  /// `install`, `update` or `remove`.
  final String op;

  /// `queued`, `running`, `done` or `failed`.
  final String state;

  /// `Downloading`, `Verifying` or `Installing`, while running.
  final String? step;

  /// The stable failure code, once failed.
  final String? error;
  final String? version;
  final String? created;
  final String? updated;

  bool get done => state == 'done';
  bool get failed => state == 'failed';

  factory AgentJob.fromJson(Map<String, dynamic> j) => AgentJob(
    id: j['id'] as String,
    hostId: j['host_id'] as String? ?? '',
    agent: j['agent'] as String? ?? '',
    op: j['op'] as String? ?? '',
    state: j['state'] as String? ?? 'queued',
    step: j['step'] as String?,
    error: j['error'] as String?,
    version: j['version'] as String?,
    created: j['created'] as String?,
    updated: j['updated'] as String?,
  );
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
/// An MCP server a session did not get, and why (decision 131a): by name
/// only.
class SkippedServer {
  const SkippedServer({
    required this.name,
    required this.source,
    required this.reason,
  });

  final String name;
  final String source;
  final String reason;

  /// A plugin's remote server: it reaches a session only as an integration.
  bool get remote => reason.startsWith('remote');

  factory SkippedServer.fromJson(Map<String, dynamic> j) => SkippedServer(
    name: '${j['name'] ?? ''}',
    source: '${j['source'] ?? ''}',
    reason: '${j['reason'] ?? ''}',
  );
}

/// A crew (decision 138a): a named, coloured group of sessions. A label on
/// the server; it changes how the Agents page is organised and nothing else.
class Crew {
  const Crew({
    required this.id,
    required this.name,
    required this.color,
    required this.order,
  });

  final String id;
  final String name;

  /// A vault palette word; anything else reads as no colour.
  final String color;
  final int order;

  Accent get accent => Accent.parse(color);

  factory Crew.fromJson(Map<String, dynamic> j) => Crew(
    id: j['id'] as String,
    name: j['name'] as String? ?? '',
    color: j['color'] as String? ?? 'none',
    order: (j['order'] as num?)?.toInt() ?? 0,
  );
}

/// The crew a session is in and its place there.
class SessionCrew {
  const SessionCrew({required this.id, required this.order});

  final String id;
  final int order;

  factory SessionCrew.fromJson(Map<String, dynamic> j) => SessionCrew(
    id: j['id'] as String,
    order: (j['order'] as num?)?.toInt() ?? 0,
  );
}

/// A Kit note used as a role (138d, 138f), by id. The label's [title] is read
/// live, so it follows a rename; null once the note is deleted.
class RoleRef {
  const RoleRef({required this.vaultId, required this.noteId, this.title});

  final String vaultId;
  final String noteId;
  final String? title;

  bool get deleted => title == null;

  bool sameNote(RoleRef? other) =>
      other != null && other.vaultId == vaultId && other.noteId == noteId;

  factory RoleRef.fromJson(Map<String, dynamic> j) => RoleRef(
    vaultId: j['vault_id'] as String,
    noteId: j['note_id'] as String,
    title: j['title'] as String?,
  );
}

/// A note the role picker offers (`GET /v1/agent/roles`, 138e).
class RoleNote {
  const RoleNote({
    required this.vaultId,
    required this.noteId,
    required this.title,
    required this.path,
    required this.folder,
    this.summary,
  });

  final String vaultId;
  final String noteId;
  final String title;
  final String path;

  /// `agents` or `projects/<name>/agents`: what the picker groups by.
  final String folder;
  final String? summary;

  factory RoleNote.fromJson(Map<String, dynamic> j) => RoleNote(
    vaultId: j['vault_id'] as String,
    noteId: j['note_id'] as String,
    title: j['title'] as String? ?? '',
    path: j['path'] as String? ?? '',
    folder: j['folder'] as String? ?? 'agents',
    summary: j['summary'] as String?,
  );
}

/// The crew a launch joins (138h): an existing one by [id], or a new one
/// named [newName].
class LaunchCrew {
  const LaunchCrew.join(String this.id) : newName = null;
  const LaunchCrew.create(String this.newName) : id = null;

  final String? id;
  final String? newName;
}

/// A crew change, with the crews it emptied and so deleted (138c).
class CrewChange {
  const CrewChange({this.crew, this.removedCrews = const []});

  final Crew? crew;
  final List<Crew> removedCrews;

  factory CrewChange.fromJson(Map<String, dynamic>? j) => CrewChange(
    crew: j?['crew'] is Map<String, dynamic>
        ? Crew.fromJson(j!['crew'] as Map<String, dynamic>)
        : null,
    removedCrews: [
      for (final c in j?['removed_crews'] as List? ?? const [])
        if (c is Map<String, dynamic>) Crew.fromJson(c),
    ],
  );
}

/// A [AgentSession._copy] argument left as it is, where null is a value.
const _keep = Object();

Map<String, dynamic>? _map(Object? v) => v is Map<String, dynamic> ? v : null;

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
    this.credential,
    this.title,
    this.activity,
    this.harnessSession,
    this.resumedFrom,
    this.mcpSkipped = const [],
    this.mcpPluginLocal = const [],
    this.identityState,
    this.isolationState,
    this.isolationReason,
    this.spawn = SpawnFacts.none,
    this.crew,
    this.role,
    this.launchRole,
    this.model,
    this.permissionMode,
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

  /// The agent's own id for the conversation, once its host has said
  /// (decision 89).
  final String? harnessSession;

  /// The session whose conversation this one reopened.
  final String? resumedFrom;

  /// The MCP servers its host said it would not have (131a).
  final List<SkippedServer> mcpSkipped;

  /// Its plugins' local MCP servers, which run on the host outside Storm's
  /// integrations (131a).
  final List<String> mcpPluginLocal;

  /// Its host's bridge identity (decision 135d); null from a server that
  /// does not say.
  final String? identityState;

  bool get identityUnverified =>
      identityState != null && identityState != 'verified';

  /// Its host's session isolation (decision 136j).
  final String? isolationState;
  final String? isolationReason;

  String? get isolationLabel =>
      isolationLabelFor(isolationState, isolationReason);

  /// Its lineage, attention, own workspace and spawn switch (decision 134).
  final SpawnFacts spawn;

  /// Its crew, null when loose or from a server before decision 138.
  final SessionCrew? crew;

  /// Its role label (138d), set by a person; a later change moves only this.
  final RoleRef? role;

  /// The role note it was launched with, titled as then (138f). Fixed.
  final RoleRef? launchRole;

  /// The model it was launched with (decision 145b); null for the agent's
  /// own default.
  final String? model;

  /// `accept_edits` or `auto` as the person chose at launch, or inherited
  /// (decision 146b); null for the agent's own.
  final String? permissionMode;

  /// There is a conversation to reopen.
  bool get hasConversation =>
      harnessSession != null &&
      const {'claude-code', 'opencode', 'codex'}.contains(provider);
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

  /// Which Claude login it launched with (decision 124): `synced`, `native`,
  /// `old_host` or `expired`. Only on the launch answer.
  final String? credential;

  bool get ended =>
      status == 'completed' || status == 'failed' || status == 'stopped';

  bool get isShell => provider == 'shell';

  /// The handoff's status words (§3.2); why it ended is [endDetail].
  String get statusLabel => switch (status) {
    'creating' || 'starting' => 'Starting',
    'running' => 'Running',
    // 145e: a child whose round is done, its process stopped until its
    // parent's next message resumes it.
    'parked' => 'Parked',
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
      // Decision 136f: an isolated host had no credential it may use.
      'credential_unavailable' => 'no credential this host may use',
      // Decision 136j: the host's isolation was not verified.
      'isolation_unavailable' => 'isolation unavailable on this host',
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
      : _copy(
          status: live.status,
          endReason: live.endReason,
          exitCode: live.exitCode,
          signal: live.signal,
          cols: live.cols,
          rows: live.rows,
          startedAt: live.startedAt ?? startedAt,
          endedAt: live.endedAt ?? endedAt,
          title: live.title ?? title,
          activity: live.activity,
          harnessSession: live.harnessSession ?? harnessSession,
          launchNotice: null,
          credential: null,
        );

  /// This record in [crew] (null: none), as a change not yet confirmed
  /// draws it.
  AgentSession withCrew(SessionCrew? crew) => _copy(crew: crew);

  /// This record with the label [role] (null: none).
  AgentSession withRole(RoleRef? role) => _copy(role: role);

  AgentSession _copy({
    String? status,
    Object? endReason = _keep,
    Object? exitCode = _keep,
    Object? signal = _keep,
    int? cols,
    int? rows,
    String? startedAt,
    String? endedAt,
    String? title,
    Object? activity = _keep,
    String? harnessSession,
    Object? launchNotice = _keep,
    Object? credential = _keep,
    Object? crew = _keep,
    Object? role = _keep,
  }) => AgentSession(
    id: id,
    hostId: hostId,
    workspace: workspace,
    provider: provider,
    status: status ?? this.status,
    endReason: endReason == _keep ? this.endReason : endReason as String?,
    exitCode: exitCode == _keep ? this.exitCode : exitCode as int?,
    signal: signal == _keep ? this.signal : signal as int?,
    cols: cols ?? this.cols,
    rows: rows ?? this.rows,
    createdAt: createdAt,
    fallback: fallback,
    startedAt: startedAt ?? this.startedAt,
    endedAt: endedAt ?? this.endedAt,
    name: name,
    context: context,
    writeVaultId: writeVaultId,
    wroteCount: wroteCount,
    integrations: integrations,
    launchNotice: launchNotice == _keep
        ? this.launchNotice
        : launchNotice as String?,
    credential: credential == _keep ? this.credential : credential as String?,
    title: title ?? this.title,
    activity: activity == _keep ? this.activity : activity as String?,
    harnessSession: harnessSession ?? this.harnessSession,
    resumedFrom: resumedFrom,
    mcpSkipped: mcpSkipped,
    mcpPluginLocal: mcpPluginLocal,
    identityState: identityState,
    isolationState: isolationState,
    isolationReason: isolationReason,
    spawn: spawn,
    crew: crew == _keep ? this.crew : crew as SessionCrew?,
    role: role == _keep ? this.role : role as RoleRef?,
    launchRole: launchRole,
    model: model,
    permissionMode: permissionMode,
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
    launchNotice: _launchNotice(j),
    credential: j['credential'] as String?,
    title: j['title'] as String?,
    activity: j['activity'] as String?,
    harnessSession: j['harness_session'] as String?,
    resumedFrom: j['resumed_from'] as String?,
    mcpSkipped: [
      for (final s in j['mcp_skipped'] as List? ?? const [])
        if (s is Map<String, dynamic>) SkippedServer.fromJson(s),
    ],
    mcpPluginLocal: [
      for (final n in j['mcp_plugin_local'] as List? ?? const []) '$n',
    ],
    identityState: _identityState(j),
    isolationState: _isolation(j).$1,
    isolationReason: _isolation(j).$2,
    spawn: SpawnFacts.fromJson(j),
    crew: _map(j['crew']) == null
        ? null
        : SessionCrew.fromJson(_map(j['crew'])!),
    role: _map(j['role']) == null ? null : RoleRef.fromJson(_map(j['role'])!),
    launchRole: _map(j['launch_role']) == null
        ? null
        : RoleRef.fromJson(_map(j['launch_role'])!),
    model: j['model'] as String?,
    permissionMode: permissionModeOf(j['permission_mode']),
  );
}

/// The permission modes each agent offers at launch (decision 146b), the
/// agent's own first. There is never a bypass.
const kPermissionModes = {
  'claude-code': ['default', 'accept_edits', 'auto'],
  'opencode': ['default', 'accept_edits'],
};

List<String> permissionModesOf(String? provider) =>
    kPermissionModes[provider] ?? const ['default'];

/// Only a mode Storm knows; anything else reads as the agent's own.
String? permissionModeOf(Object? v) =>
    v == 'accept_edits' || v == 'auto' ? v as String : null;

/// "accept edits", "auto": a row's word for a mode; null for the default.
String? permissionModeWord(String? mode) => switch (mode) {
  'accept_edits' => 'accept edits',
  'auto' => 'auto',
  _ => null,
};

/// The launcher's and About's name for a mode.
String permissionModeLabel(String mode) => switch (mode) {
  'accept_edits' => 'Accept edits',
  'auto' => 'Auto',
  _ => 'Default (the agent asks)',
};

/// Everything the launch answer says out loud: the MCP notice, then the
/// Claude login's, unless the server already said it.
String? _launchNotice(Map<String, dynamic> j) {
  final mcp = (j['mcp'] as Map?)?['notice'] as String?;
  final login =
      j['credential_notice'] as String? ??
      claudeCredentialNotice(j['credential'] as String?);
  final notices = [
    ?mcp,
    if (login != null && !(mcp?.contains('synced Claude login') ?? false))
      login,
  ];
  return notices.isEmpty ? null : notices.join('\n');
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

/// Decision 161's inbox, prefs and mutes as the client sees them (slice
/// 161-1's server shapes). Field names are the server's; nothing here is
/// invented.
library;

/// One event kind a pref row may toggle (`AUTOMATIC_KINDS` on the server).
/// `summary` and `test` are not here: a summary is controlled by
/// [NotificationPrefs.summaryOn], and a test always sends.
const kNotificationEventKinds = [
  'needs_you',
  'delete_request',
  'finished',
  'error',
  'usage_limit',
  'host_offline',
  'child_finished',
  'stopped_working',
];

/// The server's own default (`default_on` in `notifications.rs`): every
/// kind is on except these two. A row an older server wrote, or one this
/// client has never seen, reads as this rather than as always-on.
const _kDefaultOffKinds = {'child_finished', 'stopped_working'};

bool defaultEventOn(String kind) => !_kDefaultOffKinds.contains(kind);

/// One account notice in the inbox (`GET /v1/notifications`, the
/// `notification` stream event). Named `AgentNotification` to avoid
/// clashing with Flutter's own `Notification`.
class AgentNotification {
  AgentNotification({
    required this.id,
    required this.seq,
    required this.created,
    required this.source,
    required this.kind,
    this.sessionId,
    this.hostId,
    this.provider,
    this.sessionTitle,
    this.noteVaultId,
    this.noteId,
    required this.title,
    this.body,
    required this.urgency,
    this.key,
    this.silent = false,
    this.silentReason,
    this.readAt,
    this.cancelled = false,
    this.deliver,
  });

  final String id;
  final int seq;
  final String created;

  /// `storm`, `agent` or `terminal`.
  final String source;
  final String kind;
  final String? sessionId;
  final String? hostId;

  /// The session's agent at the time (161-3/4: "{session} · {Agent}").
  final String? provider;
  final String? sessionTitle;
  final String? noteVaultId;
  final String? noteId;
  final String title;
  final String? body;

  /// `low`, `normal` or `high`.
  final String urgency;
  final String? key;
  final bool silent;

  /// Why it is silent: `quiet`, `paused`, `off` or `muted`.
  final String? silentReason;
  final String? readAt;
  final bool cancelled;

  /// 161-4, live frames only: `"toast:<device_id>"` or `"os"`, or `null`
  /// when a device already showing this session covers it. Never present
  /// on `GET /v1/notifications`'s backfill — only a *live* stream frame
  /// carries delivery, so this is never persisted or read back.
  final String? deliver;

  bool get unread => readAt == null;

  AgentNotification withReadAt(String at) => readAt != null
      ? this
      : AgentNotification(
          id: id,
          seq: seq,
          created: created,
          source: source,
          kind: kind,
          sessionId: sessionId,
          hostId: hostId,
          provider: provider,
          sessionTitle: sessionTitle,
          noteVaultId: noteVaultId,
          noteId: noteId,
          title: title,
          body: body,
          urgency: urgency,
          key: key,
          silent: silent,
          silentReason: silentReason,
          readAt: at,
          cancelled: cancelled,
          deliver: deliver,
        );

  factory AgentNotification.fromJson(Map<String, dynamic> j) =>
      AgentNotification(
        id: j['id'] as String,
        seq: (j['seq'] as num).toInt(),
        created: j['created'] as String? ?? '',
        source: j['source'] as String? ?? 'storm',
        kind: j['kind'] as String? ?? '',
        sessionId: j['session_id'] as String?,
        hostId: j['host_id'] as String?,
        provider: j['provider'] as String?,
        sessionTitle: j['session_title'] as String?,
        noteVaultId: j['note_vault_id'] as String?,
        noteId: j['note_id'] as String?,
        title: j['title'] as String? ?? '',
        body: j['body'] as String?,
        urgency: j['urgency'] as String? ?? 'normal',
        key: j['key'] as String?,
        silent: j['silent'] as bool? ?? false,
        silentReason: j['silent_reason'] as String?,
        readAt: j['read_at'] as String?,
        cancelled: j['cancelled'] as bool? ?? false,
        deliver: j['deliver'] as String?,
      );
}

/// Whether one event kind notifies, and after how long (`finished`,
/// `host_offline`).
class EventPref {
  const EventPref({required this.on, this.thresholdS});

  final bool on;
  final int? thresholdS;

  factory EventPref.fromJson(Map<String, dynamic> j) => EventPref(
    on: j['on'] as bool? ?? true,
    thresholdS: (j['threshold_s'] as num?)?.toInt(),
  );

  Map<String, dynamic> toJson() => {
    'on': on,
    if (thresholdS != null) 'threshold_s': thresholdS,
  };
}

/// `GET`/`PUT /v1/notifications/prefs`'s full row (Q15's optimistic
/// concurrency is [version]).
class NotificationPrefs {
  const NotificationPrefs({
    required this.version,
    required this.events,
    required this.quietOn,
    required this.quietStart,
    required this.quietEnd,
    required this.quietTz,
    required this.quietAllowDeleteRequests,
    required this.summaryOn,
    this.pausedUntil,
    required this.agentDefaultOn,
    required this.childrenDefaultOn,
    required this.perSessionLimit,
    required this.retentionDays,
    required this.retentionRows,
  });

  final int version;
  final Map<String, EventPref> events;
  final bool quietOn;

  /// `"HH:MM"`.
  final String quietStart;
  final String quietEnd;

  /// An IANA zone.
  final String quietTz;
  final bool quietAllowDeleteRequests;
  final bool summaryOn;

  /// An RFC3339 instant; null means not paused.
  final String? pausedUntil;
  final bool agentDefaultOn;
  final bool childrenDefaultOn;
  final int perSessionLimit;
  final int retentionDays;
  final int retentionRows;

  bool eventOn(String kind) => events[kind]?.on ?? defaultEventOn(kind);

  factory NotificationPrefs.fromJson(Map<String, dynamic> j) =>
      NotificationPrefs(
        version: (j['version'] as num?)?.toInt() ?? 1,
        events: {
          for (final e in (j['events'] as Map? ?? {}).entries)
            '${e.key}': EventPref.fromJson(e.value as Map<String, dynamic>),
        },
        quietOn: j['quiet_on'] as bool? ?? false,
        quietStart: j['quiet_start'] as String? ?? '22:00',
        quietEnd: j['quiet_end'] as String? ?? '08:00',
        quietTz: j['quiet_tz'] as String? ?? 'UTC',
        quietAllowDeleteRequests:
            j['quiet_allow_delete_requests'] as bool? ?? false,
        summaryOn: j['summary_on'] as bool? ?? true,
        pausedUntil: j['paused_until'] as String?,
        agentDefaultOn: j['agent_default_on'] as bool? ?? true,
        childrenDefaultOn: j['children_default_on'] as bool? ?? false,
        perSessionLimit: (j['per_session_limit'] as num?)?.toInt() ?? 20,
        retentionDays: (j['retention_days'] as num?)?.toInt() ?? 30,
        retentionRows: (j['retention_rows'] as num?)?.toInt() ?? 1000,
      );
}

/// `GET /v1/notifications/mutes`' row.
class NotificationMute {
  const NotificationMute({
    required this.id,
    required this.scope,
    required this.target,
    this.until,
    required this.createdAt,
  });

  final String id;

  /// `session` or `provider`.
  final String scope;
  final String target;

  /// An RFC3339 instant; null is forever.
  final String? until;
  final String createdAt;

  factory NotificationMute.fromJson(Map<String, dynamic> j) => NotificationMute(
    id: j['id'] as String,
    scope: j['scope'] as String,
    target: j['target'] as String,
    until: j['until'] as String?,
    createdAt: j['created_at'] as String? ?? '',
  );
}

/// The known device-prefs classes (161-4), in the order the page shows
/// them. `classes: []` on a row means every one of these.
const kDeviceNotificationClasses = [
  'needs_you',
  'delete_requests',
  'finished',
  'errors',
  'agent_messages',
  'hosts',
];

const kDevicePreviewLevels = ['full', 'session', 'storm'];

/// `GET`/`PUT /v1/notifications/device` — this device's own row (161-4).
class DeviceNotificationPrefs {
  const DeviceNotificationPrefs({
    required this.deviceId,
    required this.enabled,
    required this.classes,
    required this.preview,
    required this.updatedAt,
  });

  final String deviceId;
  final bool enabled;

  /// Empty means every class in [kDeviceNotificationClasses].
  final List<String> classes;

  /// `full`, `session` or `storm`.
  final String preview;
  final String updatedAt;

  bool classOn(String kind) => classes.isEmpty || classes.contains(kind);

  factory DeviceNotificationPrefs.fromJson(Map<String, dynamic> j) =>
      DeviceNotificationPrefs(
        deviceId: j['device_id'] as String,
        enabled: j['enabled'] as bool? ?? true,
        classes: [for (final c in j['classes'] as List? ?? const []) '$c'],
        preview: j['preview'] as String? ?? 'full',
        updatedAt: j['updated_at'] as String? ?? '',
      );
}

/// One row of `GET /v1/notifications/devices` (161-4): this device's prefs
/// beside its name, read-only, for a future "Other devices" list. The
/// `enabled`/`classes`/`preview` fields are null for a device that has
/// never opened the app since this slice shipped.
class NotificationDeviceRow {
  const NotificationDeviceRow({
    required this.deviceId,
    required this.deviceName,
    this.platform,
    required this.revoked,
    this.enabled,
    this.classes,
    this.preview,
  });

  final String deviceId;
  final String deviceName;
  final String? platform;
  final bool revoked;
  final bool? enabled;
  final List<String>? classes;
  final String? preview;

  factory NotificationDeviceRow.fromJson(Map<String, dynamic> j) =>
      NotificationDeviceRow(
        deviceId: j['device_id'] as String,
        deviceName: j['device_name'] as String? ?? '',
        platform: j['platform'] as String?,
        revoked: j['revoked'] as bool? ?? false,
        enabled: j['enabled'] as bool?,
        classes: (j['classes'] as List?)?.map((c) => '$c').toList(),
        preview: j['preview'] as String?,
      );
}

/// Wire types for decision 162's feedback endpoints.
///
/// Same philosophy as `models.dart`: these mirror the server's JSON exactly.
/// `client` and `bundle` inside [FeedbackPreview] are kept as raw maps rather
/// than modelled further — the contract is that they are forwarded **as
/// shown**, byte for byte, so re-encoding them through a typed model would
/// risk showing the person something other than what is actually sent.
library;

/// `GET /v1/feedback` — whether the sheet can send at all.
class FeedbackStatus {
  const FeedbackStatus({required this.enabled, required this.intakeConfigured});

  final bool enabled;
  final bool intakeConfigured;

  factory FeedbackStatus.fromJson(Map<String, dynamic> j) => FeedbackStatus(
    enabled: j['enabled'] as bool? ?? true,
    intakeConfigured: j['intake_configured'] as bool? ?? true,
  );
}

/// The failing action and its message, prefilled into the sheet by a
/// "Report this" snackbar action (decision 162a).
class FeedbackErrorContext {
  const FeedbackErrorContext({required this.action, required this.message});

  final String action;
  final String message;

  Map<String, dynamic> toJson() => {'action': action, 'message': message};
}

/// What the client sends as `client` in a preview request. The server
/// redacts every string in it before storing or forwarding, so nothing here
/// needs to be scrubbed on this side beyond [ActionTrail]'s own rule of
/// never carrying a note title or path.
class FeedbackClientInfo {
  const FeedbackClientInfo({
    required this.version,
    required this.platform,
    required this.clientKind,
    required this.screen,
    required this.actions,
    required this.connectionLog,
    this.error,
  });

  final String version;
  final String platform;
  final String clientKind;
  final String screen;
  final List<String> actions;
  final FeedbackErrorContext? error;
  final List<String> connectionLog;

  Map<String, dynamic> toJson() => {
    'version': version,
    'platform': platform,
    'client_kind': clientKind,
    'screen': screen,
    'actions': actions,
    'error': error?.toJson(),
    'connection_log': connectionLog,
  };
}

/// A preview from `POST /v1/feedback/preview`: the exact bytes a send with
/// this `preview_id` will forward (AC-162-3).
class FeedbackPreview {
  const FeedbackPreview({
    required this.previewId,
    required this.expiresAt,
    required this.client,
    required this.bundle,
    required this.bytes,
  });

  final String previewId;
  final String expiresAt;

  /// Exactly the server's redacted `client` object — shown verbatim in
  /// "See what's sent".
  final Map<String, dynamic> client;

  /// `{}` when diagnostics were left off; otherwise `{server, hosts}`.
  final Map<String, dynamic> bundle;
  final int bytes;

  factory FeedbackPreview.fromJson(Map<String, dynamic> j) => FeedbackPreview(
    previewId: j['preview_id'] as String,
    expiresAt: j['expires_at'] as String? ?? '',
    client: (j['client'] as Map?)?.cast<String, dynamic>() ?? const {},
    bundle: (j['bundle'] as Map?)?.cast<String, dynamic>() ?? const {},
    bytes: (j['bytes'] as num?)?.toInt() ?? 0,
  );
}

/// Result of `POST /v1/feedback`.
class FeedbackSendResult {
  const FeedbackSendResult({required this.reportId, required this.queued});

  final String reportId;
  final bool queued;

  factory FeedbackSendResult.fromJson(Map<String, dynamic> j) =>
      FeedbackSendResult(
        reportId: j['report_id'] as String,
        queued: j['queued'] as bool? ?? true,
      );
}

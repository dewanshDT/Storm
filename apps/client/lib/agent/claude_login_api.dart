/// Claude subscription token sync, client side (decision 124, token sync
/// spec §0.3, §4, §6). The token goes up once in a request body and never
/// comes back: no type here holds it.
library;

import 'dart:async';
import 'dart:convert';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:http/http.dart' as http;

import '../api/models.dart';
import '../state/app_state.dart' show sessionClientProvider;
import 'agent_models.dart';
import 'agent_state.dart' show agentRefreshInterval, sessionCredentialsProvider;

const kClaudeTokenKind = 'claude_subscription_token';

/// Where the token comes from and what to do with it, for the dialogs and
/// the README link.
const kClaudeLoginReadme =
    'https://github.com/dewanshDT/Storm#claude-login-sync-optional';

final _tokenShape = RegExp(r'^sk-ant-oat01-[A-Za-z0-9_-]{20,400}$');

/// The pasted text as the server will judge it (§0.1): bracketed-paste
/// markers, `\r` and surrounding whitespace removed. Null when it is not a
/// Claude token.
String? cleanClaudeToken(String pasted) {
  final cleaned = pasted
      .replaceAll('\x1b[200~', '')
      .replaceAll('\x1b[201~', '')
      .replaceAll('\r', '')
      .trim();
  return _tokenShape.hasMatch(cleaned) ? cleaned : null;
}

const kClaudeTokenRule =
    'A Claude token starts with sk-ant-oat01- and has no spaces.';

/// One of the page's states (§6).
enum ClaudeLoginState {
  notSynced,
  active,
  needsResync,
  expiring30,
  expiring7,
  expired,
}

/// `GET /v1/provider-credentials/claude-subscription`. **Never the token.**
class ClaudeLogin {
  const ClaudeLogin({
    required this.present,
    required this.status,
    required this.fingerprint,
    required this.mintedAt,
    required this.expiresAt,
    required this.daysLeft,
    required this.warn,
    required this.hosts,
  });

  const ClaudeLogin.none()
    : present = false,
      status = null,
      fingerprint = null,
      mintedAt = null,
      expiresAt = null,
      daysLeft = null,
      warn = null,
      hosts = const [];

  final bool present;

  /// `active` or `needs_resync`.
  final String? status;
  final String? fingerprint;
  final DateTime? mintedAt;
  final DateTime? expiresAt;
  final int? daysLeft;

  /// Null, `30d`, `7d` or `expired`.
  final String? warn;

  /// The hosts it goes to. Empty is none (opt-in, §4).
  final List<String> hosts;

  ClaudeLoginState get state {
    if (!present) return ClaudeLoginState.notSynced;
    if (status == 'needs_resync') return ClaudeLoginState.needsResync;
    return switch (warn) {
      'expired' => ClaudeLoginState.expired,
      '7d' => ClaudeLoginState.expiring7,
      '30d' => ClaudeLoginState.expiring30,
      _ => ClaudeLoginState.active,
    };
  }

  /// Whether the server would deliver it at all, wherever the host.
  bool get deliverable => present && status == 'active' && warn != 'expired';

  bool get needsAttention =>
      present && (status == 'needs_resync' || warn != null);

  factory ClaudeLogin.fromJson(Map<String, dynamic> j) {
    DateTime? at(String key) =>
        j[key] is String ? DateTime.tryParse(j[key] as String) : null;
    return ClaudeLogin(
      present: j['present'] as bool? ?? false,
      status: j['status'] as String?,
      fingerprint: j['fingerprint'] as String?,
      mintedAt: at('minted_at'),
      expiresAt: at('expires_at'),
      daysLeft: (j['days_left'] as num?)?.toInt(),
      warn: j['warn'] as String?,
      hosts: [for (final h in (j['hosts'] as List? ?? const [])) h as String],
    );
  }
}

/// Which login a `claude-code` session on a host will use, by the server's
/// four rules (§0.4), so the launcher's line stays true.
enum ClaudeLoginRoute { synced, own, oldHost, expired, needsResync }

ClaudeLoginRoute claudeLoginRoute(ClaudeLogin login, AgentHost host) {
  if (!login.present || !login.hosts.contains(host.id)) {
    return ClaudeLoginRoute.own;
  }
  if (login.status == 'needs_resync') return ClaudeLoginRoute.needsResync;
  if (login.warn == 'expired') return ClaudeLoginRoute.expired;
  if (!host.receivesClaudeLogin) return ClaudeLoginRoute.oldHost;
  return ClaudeLoginRoute.synced;
}

/// The launch answer's `credential` (§0.4) as something to tell the user,
/// or null when there is nothing to say.
String? claudeCredentialNotice(String? credential) => switch (credential) {
  'old_host' =>
    "This host can't receive your synced Claude login — update "
        "storm-runtime. This session uses the host's own login.",
  'expired' => 'Your synced Claude login has expired — re-sync it in Settings.',
  _ => null,
};

String _describe(int status, String? code) => switch ((status, code)) {
  (400, 'bad_token_shape') => kClaudeTokenRule,
  (400, 'unknown_host') => 'One of those hosts is no longer enrolled.',
  (403, _) => 'Claude login is managed by the server owner only.',
  (404, _) ||
  (405, _) => "This server can't sync a Claude login yet. Update Storm.",
  _ => 'HTTP $status',
};

/// REST client for `/v1/provider-credentials/claude-subscription` (session
/// tier, §0.3). Never logs a body.
class ClaudeLoginApi {
  ClaudeLoginApi({
    required this.baseUrl,
    required this.token,
    http.Client? client,
  }) : _client = client ?? http.Client();

  final String baseUrl;
  final String token;
  final http.Client _client;

  static const _path = '/v1/provider-credentials/claude-subscription';

  Map<String, String> get _headers => {
    'Authorization': 'Bearer $token',
    'Content-Type': 'application/json',
  };

  Uri _uri(String path) => Uri.parse('$baseUrl$path');

  ClaudeLogin? _view(http.Response r) {
    if (r.statusCode < 200 || r.statusCode >= 300) {
      String? code;
      try {
        final body = jsonDecode(r.body);
        if (body is Map && body['error'] != null) code = '${body['error']}';
      } catch (_) {}
      throw StormApiException(r.statusCode, _describe(r.statusCode, code));
    }
    if (r.bodyBytes.isEmpty) return null;
    return ClaudeLogin.fromJson(
      jsonDecode(utf8.decode(r.bodyBytes)) as Map<String, dynamic>,
    );
  }

  Future<ClaudeLogin> get() async =>
      _view(await _client.get(_uri(_path), headers: _headers)) ??
      const ClaudeLogin.none();

  /// Creates or replaces it. [mintedAt] null lets the server take the upload
  /// time.
  Future<ClaudeLogin> put(String claudeToken, {DateTime? mintedAt}) async =>
      _view(
        await _client.put(
          _uri(_path),
          headers: _headers,
          body: jsonEncode({
            'token': claudeToken,
            if (mintedAt != null)
              'minted_at': mintedAt.toUtc().toIso8601String(),
          }),
        ),
      )!;

  Future<void> delete() async {
    _view(await _client.delete(_uri(_path), headers: _headers));
  }

  /// `[]` is none.
  Future<ClaudeLogin> setHosts(List<String> hosts) async => _view(
    await _client.patch(
      _uri('$_path/hosts'),
      headers: _headers,
      body: jsonEncode({'hosts': hosts}),
    ),
  )!;

  void dispose() => _client.close();
}

/// How this client builds a [ClaudeLoginApi], or null without a session.
/// Tests override it.
final claudeLoginApiFactoryProvider = Provider<ClaudeLoginApi Function()?>((
  ref,
) {
  final session = ref.watch(sessionCredentialsProvider);
  if (session == null) return null;
  final (baseUrl, token) = session;
  final client = ref.watch(sessionClientProvider);
  return () => ClaudeLoginApi(baseUrl: baseUrl, token: token, client: client);
});

/// The synced login's status, refreshed on the agent list's interval. Null
/// when the server cannot be asked, or refuses.
final claudeLoginSummaryProvider = FutureProvider.autoDispose<ClaudeLogin?>((
  ref,
) async {
  final timer = Timer(agentRefreshInterval, ref.invalidateSelf);
  ref.onDispose(timer.cancel);
  final api = ref.watch(claudeLoginApiFactoryProvider)?.call();
  if (api == null) return null;
  try {
    return await api.get();
  } catch (_) {
    return null;
  } finally {
    api.dispose();
  }
});

/// The Claude login nav dot: expiring, expired or unreadable.
final claudeLoginAttentionProvider = Provider.autoDispose<bool>(
  (ref) => ref.watch(claudeLoginSummaryProvider).value?.needsAttention ?? false,
);

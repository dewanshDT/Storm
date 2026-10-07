import 'dart:convert';

import 'package:http/http.dart' as http;

import '../api/models.dart';

/// One integration as the owner sees it (`/v1/integrations/connections`,
/// decision 81c). **Never a credential**: only whether one is held.
class Integration {
  Integration({
    required this.id,
    required this.slug,
    required this.displayName,
    required this.url,
    required this.authKind,
    required this.status,
    required this.builtin,
    required this.hasCredential,
    required this.toolAllowlist,
    required this.knownTools,
    required this.lastError,
    required this.vaultWritesAvailable,
  });

  final String id;
  final String slug;
  final String displayName;
  final String? url;

  /// `oauth`, `static` or `none`.
  final String authKind;

  /// `pending_auth`, `connected`, `needs_reauth`, `error` or `disabled`.
  final String status;
  final bool builtin;
  final bool hasCredential;
  final List<String> toolAllowlist;
  final List<String>? knownTools;
  final String? lastError;

  /// Built-in only: whether the server lets agents write at all.
  final bool? vaultWritesAvailable;

  bool get disabled => status == 'disabled';
  bool get needsReconnect =>
      status == 'needs_reauth' || status == 'pending_auth';

  /// What the status chip says, in plain words.
  String get statusLabel => switch (status) {
    'connected' => 'Connected',
    'pending_auth' => 'Not authorized yet',
    'needs_reauth' => 'Needs reconnecting',
    'disabled' => 'Disabled',
    'error' => 'Error',
    _ => status,
  };

  factory Integration.fromJson(Map<String, dynamic> j) => Integration(
    id: j['id'] as String,
    slug: j['slug'] as String,
    displayName: j['display_name'] as String,
    url: j['url'] as String?,
    authKind: j['auth_kind'] as String,
    status: j['status'] as String,
    builtin: j['builtin'] as bool? ?? false,
    hasCredential: j['has_credential'] as bool? ?? false,
    toolAllowlist: [
      for (final t in (j['tool_allowlist'] as List? ?? const [])) t as String,
    ],
    knownTools: (j['known_tools'] as List?)?.cast<String>(),
    lastError: j['last_error_code'] as String?,
    vaultWritesAvailable: j['vault_writes_available'] as bool?,
  );
}

/// One upstream tool, for the allowlist editor (G-D16).
class IntegrationTool {
  IntegrationTool({
    required this.name,
    required this.description,
    required this.allowed,
    required this.isNew,
  });

  final String name;
  final String? description;
  final bool allowed;

  /// Seen for the first time: off until the owner turns it on.
  final bool isNew;

  factory IntegrationTool.fromJson(Map<String, dynamic> j) => IntegrationTool(
    name: j['name'] as String,
    description: j['description'] as String?,
    allowed: j['allowed'] as bool? ?? false,
    isNew: j['new'] as bool? ?? false,
  );
}

/// What a test (or an authorization, which ends in one) found.
class IntegrationCheck {
  IntegrationCheck({
    required this.ok,
    required this.errorCode,
    required this.toolCount,
    required this.newTools,
    required this.integration,
  });

  final bool ok;
  final String? errorCode;
  final int toolCount;
  final List<String> newTools;
  final Integration integration;

  factory IntegrationCheck.fromJson(Map<String, dynamic> j) => IntegrationCheck(
    ok: j['ok'] as bool? ?? false,
    errorCode: j['error_code'] as String?,
    toolCount: (j['tool_count'] as num?)?.toInt() ?? 0,
    newTools: [
      for (final t in (j['new_tools'] as List? ?? const [])) t as String,
    ],
    integration: Integration.fromJson(j['integration'] as Map<String, dynamic>),
  );
}

/// The plain-words reading of a stable failure code (spec §12).
String describeIntegrationError(String? code) => switch (code) {
  null => '',
  'upstream_unauthorized' ||
  'integration_needs_reauth' => 'The service refused the credential.',
  'upstream_unavailable' => 'The service could not be reached.',
  'upstream_rate_limited' => 'The service is rate limiting Storm.',
  'upstream_protocol_error' => 'That address does not answer as an MCP server.',
  'oauth_discovery_failed' => 'Could not find how this service signs in.',
  'oauth_not_offered' => 'This service does not offer sign-in; use a token.',
  'oauth_client_required' =>
    'This service needs a client id registered by hand, or a token.',
  'oauth_registration_failed' => 'The service refused to register Storm.',
  'oauth_exchange_failed' => 'Signing in did not complete. Try again.',
  'oauth_flow_expired_or_used' => 'That sign-in expired. Start it again.',
  _ => code,
};

/// REST client for `/v1/integrations/*` (owner only; decisions 81c, 81d, 81g).
class IntegrationsApi {
  IntegrationsApi({
    required this.baseUrl,
    required this.token,
    http.Client? client,
  }) : _client = client ?? http.Client();

  final String baseUrl;
  final String token;
  final http.Client _client;

  static const _base = '/v1/integrations/connections';

  Map<String, String> get _headers => {
    'Authorization': 'Bearer $token',
    'Content-Type': 'application/json',
  };

  Uri _uri(String path) => Uri.parse('$baseUrl$path');

  String _one(String id) => '$_base/${Uri.encodeComponent(id)}';

  dynamic _decode(http.Response r) {
    if (r.statusCode < 200 || r.statusCode >= 300) {
      var message = switch (r.statusCode) {
        403 => 'Integrations are managed by the server owner only.',
        _ => 'HTTP ${r.statusCode}',
      };
      try {
        final body = jsonDecode(r.body);
        if (body is Map && body['error'] != null && r.statusCode != 403) {
          message = describeIntegrationError('${body['error']}');
        }
      } catch (_) {}
      throw StormApiException(r.statusCode, message);
    }
    if (r.bodyBytes.isEmpty) return null;
    return jsonDecode(utf8.decode(r.bodyBytes));
  }

  Future<List<Integration>> list() async {
    final json = _decode(await _client.get(_uri(_base), headers: _headers));
    return [
      for (final i in json as List)
        Integration.fromJson(i as Map<String, dynamic>),
    ];
  }

  /// A `static` integration carries its header and value; `oauth` and `none`
  /// carry nothing. The value is sent once and never comes back.
  Future<Integration> create({
    required String displayName,
    required String url,
    required String authKind,
    String? header,
    String? value,
  }) async => Integration.fromJson(
    _decode(
          await _client.post(
            _uri(_base),
            headers: _headers,
            body: jsonEncode({
              'display_name': displayName,
              'url': url,
              'auth_kind': authKind,
              if (value != null)
                'credential': {'header': ?header, 'value': value},
            }),
          ),
        )
        as Map<String, dynamic>,
  );

  Future<Integration> update(
    String id, {
    bool? enabled,
    List<String>? toolAllowlist,
    String? header,
    String? value,
  }) async => Integration.fromJson(
    _decode(
          await _client.patch(
            _uri(_one(id)),
            headers: _headers,
            body: jsonEncode({
              'enabled': ?enabled,
              'tool_allowlist': ?toolAllowlist,
              if (value != null)
                'credential': {'header': ?header, 'value': value},
            }),
          ),
        )
        as Map<String, dynamic>,
  );

  Future<void> delete(String id) async {
    _decode(await _client.delete(_uri(_one(id)), headers: _headers));
  }

  Future<IntegrationCheck> test(String id) async => IntegrationCheck.fromJson(
    _decode(await _client.post(_uri('${_one(id)}/test'), headers: _headers))
        as Map<String, dynamic>,
  );

  Future<List<IntegrationTool>> tools(String id) async {
    final json = _decode(
      await _client.get(_uri('${_one(id)}/tools'), headers: _headers),
    );
    return [
      for (final t in json as List)
        IntegrationTool.fromJson(t as Map<String, dynamic>),
    ];
  }

  /// Starts an OAuth authorization: the URL to open in the system browser.
  Future<String> authorize(String id, {required String redirectUri}) async =>
      (_decode(
                await _client.post(
                  _uri('${_one(id)}/authorize'),
                  headers: _headers,
                  body: jsonEncode({'redirect_uri': redirectUri}),
                ),
              )
              as Map)['authorization_url']
          as String;

  /// Relays what the browser brought back (G-D13).
  Future<IntegrationCheck> callback({
    required String state,
    required String code,
    String? iss,
  }) async => IntegrationCheck.fromJson(
    _decode(
          await _client.post(
            _uri('/v1/integrations/oauth/callback'),
            headers: _headers,
            body: jsonEncode({'state': state, 'code': code, 'iss': ?iss}),
          ),
        )
        as Map<String, dynamic>,
  );

  void dispose() => _client.close();
}

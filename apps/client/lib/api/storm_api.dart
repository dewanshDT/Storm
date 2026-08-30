import 'dart:convert';

import 'package:http/http.dart' as http;

import 'auth_models.dart';
import 'models.dart';

/// REST client for `storm-server`.
///
/// A thin transport for one *server*. The vault is a parameter of each call
/// rather than a property of the client: one server hosts many vaults, and the
/// dashboard has to read across all of them.
///
/// Caching and the outbox live above this class, in [SyncEngine], so this stays
/// transport only.
class StormApi {
  StormApi({required this.baseUrl, required this.token, http.Client? client})
    : _client = client ?? http.Client();

  /// e.g. `http://192.168.1.20:8484` — no trailing slash.
  final String baseUrl;
  final String token;
  final http.Client _client;

  /// The credential exactly as the server expects it, scheme included.
  ///
  /// The query-string paths below send *this*, not the bare token. The
  /// server's query fallback takes the same string the header would carry and
  /// matches on the scheme, so a bare token authenticates nothing — it used to
  /// work only because the shared token was compared whole, and that is gone.
  String get credential => 'Bearer $token';

  Map<String, String> get _headers => {
    'Authorization': credential,
    'Content-Type': 'application/json',
  };

  Uri _uri(String path, [Map<String, String>? query]) =>
      Uri.parse('$baseUrl$path').replace(queryParameters: query);

  /// Vault-scoped path. Vault ids are UUIDs, but encoded anyway so a
  /// hand-edited registry cannot produce a malformed URL.
  String _v(String vaultId, String path) =>
      '/v1/vaults/${Uri.encodeComponent(vaultId)}$path';

  /// Percent-encodes a vault path for a URL, keeping its separators — folder
  /// and attachment names routinely contain spaces.
  String _path(String p) => p.split('/').map(Uri.encodeComponent).join('/');

  /// Decodes a response, converting any non-2xx into [StormApiException].
  dynamic _decode(http.Response r) {
    if (r.statusCode < 200 || r.statusCode >= 300) {
      String message = 'HTTP ${r.statusCode}';
      try {
        final body = jsonDecode(r.body);
        if (body is Map && body['error'] != null) message = '${body['error']}';
      } catch (_) {
        // Non-JSON error body (a proxy, say) — keep the status line.
      }
      throw StormApiException(r.statusCode, message);
    }
    // utf8.decode rather than r.body: the latter guesses latin-1 when the
    // server omits a charset, which mangles any non-ASCII note.
    return jsonDecode(utf8.decode(r.bodyBytes));
  }

  /// Verifies the server is reachable and the token is accepted.
  ///
  /// Uses the vault list rather than a tree: it needs no vault to exist yet,
  /// so a brand-new server verifies just as well as a populated one.
  Future<void> checkConnection() async {
    _decode(await _client.get(_uri('/v1/vaults'), headers: _headers));
  }

  Future<VaultTree> tree(String vaultId) async {
    final json = _decode(
      await _client.get(_uri(_v(vaultId, '/tree')), headers: _headers),
    );
    return VaultTree.fromJson(json as Map<String, dynamic>);
  }

  /// Every vault this server hosts.
  Future<List<VaultInfo>> vaults() async {
    final json = _decode(
      await _client.get(_uri('/v1/vaults'), headers: _headers),
    );
    return (json['vaults'] as List)
        .map((v) => VaultInfo.fromJson(v as Map<String, dynamic>))
        .toList();
  }

  Future<VaultInfo> createVault(String name) async {
    final json = _decode(
      await _client.post(
        _uri('/v1/vaults'),
        headers: _headers,
        body: jsonEncode({'name': name}),
      ),
    );
    return VaultInfo.fromJson(json as Map<String, dynamic>);
  }

  Future<void> renameVault(String vaultId, String name) async {
    _decode(
      await _client.patch(
        _uri('/v1/vaults/${Uri.encodeComponent(vaultId)}'),
        headers: _headers,
        body: jsonEncode({'name': name}),
      ),
    );
  }

  /// Unregisters a vault. The server never deletes its files.
  Future<void> removeVault(String vaultId) async {
    _decode(
      await _client.delete(
        _uri('/v1/vaults/${Uri.encodeComponent(vaultId)}'),
        headers: _headers,
      ),
    );
  }

  /// Recently opened notes across every vault, newest first.
  Future<List<RecentNote>> recents({int limit = 20}) async {
    final json = _decode(
      await _client.get(
        _uri('/v1/recents', {'limit': '$limit'}),
        headers: _headers,
      ),
    );
    return (json['recents'] as List)
        .map((r) => RecentNote.fromJson(r as Map<String, dynamic>))
        .toList();
  }

  Future<ServerConfig> config() async {
    final json = _decode(
      await _client.get(_uri('/v1/config'), headers: _headers),
    );
    return ServerConfig.fromJson(json as Map<String, dynamic>);
  }

  /// Points the server at a different storage root.
  ///
  /// Never moves files. Throws with a 409 when the new root holds none of the
  /// registered vaults, unless [orphanOk] — see the server's `put_config`.
  Future<void> setVaultRoot(String path, {bool orphanOk = false}) async {
    _decode(
      await _client.put(
        _uri('/v1/config'),
        headers: _headers,
        body: jsonEncode({'vault_root': path, 'orphan_ok': orphanOk}),
      ),
    );
  }

  /// Switches the server's MCP endpoint on or off.
  ///
  /// Persisted server-side, so it survives a restart — the setting is the
  /// server's, not this device's.
  Future<void> setMcpEnabled(bool enabled, {bool writable = false}) async {
    _decode(
      await _client.put(
        _uri('/v1/config/mcp'),
        headers: _headers,
        body: jsonEncode({'enabled': enabled, 'writable': writable}),
      ),
    );
  }

  // ---- MCP keys (A14) ---------------------------------------------------

  /// Mints an MCP key. **The secret in the response is the only copy.**
  ///
  /// Session tier: minting costs a real sign-in, which is also what makes "a
  /// key cannot mint a key" true without a special case — a key never reaches
  /// this route.
  Future<CreatedMcpKey> createMcpKey({
    required String name,
    String? expires,
  }) async {
    final json = _decode(
      await _client.post(
        _uri('/v1/keys'),
        headers: _headers,
        body: jsonEncode({'name': name, 'expires': ?expires}),
      ),
    );
    return CreatedMcpKey.fromJson(json as Map<String, dynamic>);
  }

  /// The caller's keys — or another user's, which only an owner may ask for.
  Future<List<McpKey>> mcpKeys({String? user}) async {
    final json = _decode(
      await _client.get(
        _uri('/v1/keys', user == null ? null : {'user': user}),
        headers: _headers,
      ),
    );
    return (json as List)
        .map((e) => McpKey.fromJson(e as Map<String, dynamic>))
        .toList();
  }

  /// Revokes a key. Effective on the next request the holder makes.
  Future<void> revokeMcpKey(String id) async {
    final r = await _client.delete(
      _uri('/v1/keys/${Uri.encodeComponent(id)}'),
      headers: _headers,
    );
    // 204 carries no body, so `_decode` would choke on the empty string.
    if (r.statusCode < 200 || r.statusCode >= 300) {
      throw StormApiException(r.statusCode, 'HTTP ${r.statusCode}');
    }
  }

  Future<void> createFolder(String vaultId, String path) async {
    _decode(
      await _client.post(
        _uri(_v(vaultId, '/folders')),
        headers: _headers,
        body: jsonEncode({'path': path}),
      ),
    );
  }

  Future<void> deleteFolder(String vaultId, String path) async {
    _decode(
      await _client.delete(
        _uri(_v(vaultId, '/folders/${_path(path)}')),
        headers: _headers,
      ),
    );
  }

  Future<void> renameFolder(String vaultId, String from, String to) async {
    _decode(
      await _client.post(
        _uri(_v(vaultId, '/folders/rename')),
        headers: _headers,
        body: jsonEncode({'from': from, 'to': to}),
      ),
    );
  }

  /// Records that a note was opened, for the cross-vault recents list.
  Future<void> markOpened(String vaultId, String id) async {
    _decode(
      await _client.post(
        _uri(_v(vaultId, '/notes/$id/opened')),
        headers: _headers,
      ),
    );
  }

  /// Changes after [since] — the delta-sync primitive.
  ///
  /// Clients store the returned `seq` and pass it back next time, so catching
  /// up never requires diffing a full vault manifest.
  Future<SyncBatch> sync(
    String vaultId, {
    int since = 0,
    int limit = 500,
  }) async {
    final json = _decode(
      await _client.get(
        _uri(_v(vaultId, '/sync'), {'since': '$since', 'limit': '$limit'}),
        headers: _headers,
      ),
    );
    return SyncBatch.fromJson(json as Map<String, dynamic>);
  }

  Future<Note> note(String vaultId, String id) async {
    final json = _decode(
      await _client.get(_uri(_v(vaultId, '/notes/$id')), headers: _headers),
    );
    return Note.fromJson(json as Map<String, dynamic>);
  }

  Future<WriteResult> createNote({
    required String vaultId,
    required String path,
    String content = '',
  }) async {
    final json = _decode(
      await _client.post(
        _uri(_v(vaultId, '/notes')),
        headers: _headers,
        body: jsonEncode({'path': path, 'content': content}),
      ),
    );
    return WriteResult.fromJson(json as Map<String, dynamic>);
  }

  /// Saves a note.
  ///
  /// [baseVersion] is the version this client last read. The server uses it to
  /// decide between a fast-forward and a 3-way merge — see the README.
  Future<WriteResult> saveNote({
    required String vaultId,
    required String id,
    required int baseVersion,
    required String content,
    String? deviceId,
  }) async {
    final json = _decode(
      await _client.put(
        _uri(_v(vaultId, '/notes/$id')),
        headers: _headers,
        body: jsonEncode({
          'base_version': baseVersion,
          'content': content,
          'device_id': ?deviceId,
        }),
      ),
    );
    return WriteResult.fromJson(json as Map<String, dynamic>);
  }

  Future<WriteResult> moveNote({
    required String vaultId,
    required String id,
    required String newPath,
  }) async {
    final json = _decode(
      await _client.post(
        _uri(_v(vaultId, '/notes/$id/move')),
        headers: _headers,
        body: jsonEncode({'new_path': newPath}),
      ),
    );
    return WriteResult.fromJson(json as Map<String, dynamic>);
  }

  Future<void> deleteNote(String vaultId, String id) async {
    _decode(
      await _client.delete(_uri(_v(vaultId, '/notes/$id')), headers: _headers),
    );
  }

  Future<List<SearchHit>> search(
    String vaultId,
    String query, {
    int limit = 50,
  }) async {
    if (query.trim().isEmpty) return const [];
    final json = _decode(
      await _client.get(
        _uri(_v(vaultId, '/search'), {'q': query, 'limit': '$limit'}),
        headers: _headers,
      ),
    );
    return (json['hits'] as List)
        .map((h) => SearchHit.fromJson(h as Map<String, dynamic>))
        .toList();
  }

  /// Uploads an attachment. Binary, opaque, last-write-wins on the server.
  Future<void> uploadAttachment(
    String vaultId,
    String path,
    List<int> bytes,
  ) async {
    final r = await _client.put(
      _uri(_v(vaultId, '/attachments/${_path(path)}')),
      headers: {'Authorization': 'Bearer $token'},
      body: bytes,
    );
    _decode(r);
  }

  Future<List<int>> attachment(String vaultId, String path) async {
    final r = await _client.get(
      _uri(_v(vaultId, '/attachments/${_path(path)}')),
      headers: {'Authorization': 'Bearer $token'},
    );
    if (r.statusCode < 200 || r.statusCode >= 300) {
      throw StormApiException(r.statusCode, 'HTTP ${r.statusCode}');
    }
    return r.bodyBytes;
  }

  /// A URL an `Image.network` can fetch directly.
  ///
  /// The credential rides in the query string because Flutter's image widgets
  /// can't set headers on the request they make.
  Uri attachmentUrl(String vaultId, String path) =>
      _uri(_v(vaultId, '/attachments/${_path(path)}'), {'token': credential});

  /// A single-use, 60-second credential for the change-feed handshake.
  ///
  /// A WebSocket handshake carries no headers, so *something* has to ride in
  /// the URL — and a URL lands in proxy logs, browser history and referrers.
  /// A session token there is good for thirty days; this is good for one use
  /// and one minute, which is the whole reason the endpoint exists.
  Future<String> wsTicket() async {
    final json = _decode(
      await _client.post(_uri('/v1/auth/ws-ticket'), headers: _headers),
    );
    return json['ticket'] as String;
  }

  Future<List<TagCount>> tags(String vaultId) async {
    final json = _decode(
      await _client.get(_uri(_v(vaultId, '/tags')), headers: _headers),
    );
    return (json['tags'] as List)
        .map((t) => TagCount.fromJson(t as Map<String, dynamic>))
        .toList();
  }

  Future<List<NoteMeta>> notesWithTag(String vaultId, String tag) async {
    final json = _decode(
      await _client.get(
        _uri(_v(vaultId, '/tags/${Uri.encodeComponent(tag)}')),
        headers: _headers,
      ),
    );
    return (json['notes'] as List)
        .map((n) => NoteMeta.fromJson(n as Map<String, dynamic>))
        .toList();
  }

  Future<List<NoteMeta>> backlinks(String vaultId, String id) async {
    final json = _decode(
      await _client.get(
        _uri(_v(vaultId, '/notes/$id/backlinks')),
        headers: _headers,
      ),
    );
    return (json['backlinks'] as List)
        .map((n) => NoteMeta.fromJson(n as Map<String, dynamic>))
        .toList();
  }

  /// `POST /v1/auth/logout` — revoke the current session.
  Future<void> logout() async {
    await _client.post(_uri('/v1/auth/logout'), headers: _headers);
  }

  /// `PUT /v1/config/registration` — open or close registration (A13).
  ///
  /// Owner only, enforced server-side. Off by default: with web bootstrap in
  /// play, on means anyone who can reach this server can make an account.
  Future<void> setRegistrationOpen(bool enabled) async {
    _decode(
      await _client.put(
        _uri('/v1/config/registration'),
        headers: _headers,
        body: jsonEncode({'enabled': enabled}),
      ),
    );
  }

  /// `POST /v1/pairings` — mint a pairing invite for a **new** device.
  ///
  /// Session tier: an already-signed-in client vouches for the device being
  /// added. Purpose is always `add_device` — the `first_user` window is a
  /// one-shot that belongs to the server console (A8), and asking for it here
  /// would be asking the server for something it must refuse.
  Future<PairingInvite> issuePairing() async {
    final r = await _client.post(
      _uri('/v1/pairings'),
      headers: _headers,
      body: jsonEncode({'purpose': 'add_device'}),
    );
    if (r.statusCode < 200 || r.statusCode >= 300) {
      throw StormApiException(r.statusCode, 'HTTP ${r.statusCode}: ${r.body}');
    }
    return PairingInvite.fromJson(
      jsonDecode(utf8.decode(r.bodyBytes)) as Map<String, dynamic>,
    );
  }

  void dispose() => _client.close();
}

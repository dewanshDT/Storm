import 'dart:async';

import 'package:http/http.dart' as http;

import 'server_verifier.dart';
import 'storm_connection.dart';

/// An `http.Client` that sends each request over the session's connection
/// (decision 112).
///
/// Requests are addressed to the saved server address, as every caller always
/// has. Each one goes out on whatever transport the connection has proven *at
/// that moment*, direct or relayed, so a reconnect that re-races moves the
/// next request with it rather than leaving it on a dead trunk. A request made
/// while the connection is not connected waits for it (§8.5), joining the
/// race in flight; it never starts a second one. A failure is reported to the
/// connection, which alone decides whether the transport is lost (§8.4).
///
/// Nothing is sent to an unproven server, and nothing to a host other than the
/// server: a request for any other URL is refused, never sent with the
/// session's credential.
class ConnectionClient extends http.BaseClient {
  ConnectionClient(this._connection);

  final StormConnection? Function() _connection;

  /// Whether the next request would go through a relay.
  bool get isRelayed => _connection()?.isRelayed ?? false;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    final c = _connection();
    if (c == null) {
      throw http.ClientException('Not signed in.', request.url);
    }
    final path = _onServer(c, request.url);
    if (path == null) {
      throw http.ClientException('Not this server.', request.url);
    }
    final proof = c.isConnected
        ? ServerProof.verified
        : await c.awaitConnected();
    switch (proof) {
      case ServerProof.verified:
        break;
      case ServerProof.impostor:
        throw http.ClientException(
          'Server failed the cryptographic challenge.',
          request.url,
        );
      case ServerProof.unreachable:
        throw http.ClientException('Server unreachable.', request.url);
    }
    final active = _onActive(c, path, request.url);
    final generation = c.generation;
    try {
      return await c.transportClient.send(
        active == request.url ? request : _copy(request, active),
      );
    } catch (e) {
      c.requestFailed(e, generation: generation);
      rethrow;
    }
  }

  /// [url]'s path below the saved address, or null when it is not on it.
  static String? _onServer(StormConnection c, Uri url) {
    final base = Uri.parse(c.localAddress);
    if (url.scheme != base.scheme ||
        url.host != base.host ||
        url.port != base.port ||
        !url.path.startsWith(base.path)) {
      return null;
    }
    return url.path.substring(base.path.length);
  }

  static Uri _onActive(StormConnection c, String path, Uri url) {
    final base = Uri.parse(c.active.baseUrl);
    return base.replace(
      path: '${base.path}$path',
      query: url.hasQuery ? url.query : null,
    );
  }

  static http.BaseRequest _copy(http.BaseRequest r, Uri url) {
    final out = http.StreamedRequest(r.method, url)
      ..headers.addAll(r.headers)
      ..contentLength = r.contentLength
      ..followRedirects = r.followRedirects
      ..maxRedirects = r.maxRedirects
      ..persistentConnection = r.persistentConnection;
    r.finalize().listen(
      out.sink.add,
      onError: out.sink.addError,
      onDone: out.sink.close,
    );
    return out;
  }

  /// The connection owns its transports; this borrows them.
  @override
  void close() {}
}

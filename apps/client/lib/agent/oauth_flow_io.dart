import 'dart:async';
import 'dart:io';

import 'integrations_api.dart';

/// Native clients can listen on loopback for the redirect.
const bool oauthSupported = true;

/// How long the browser has to come back.
const _deadline = Duration(minutes: 10);

/// Runs one sign-in: a loopback listener on `127.0.0.1` with a port the OS
/// picks, the authorization URL in the system browser, the one redirect it
/// sends back, and the relay to the server.
///
/// **Loopback on every native platform, macOS and Android included.** G1 found
/// Notion and Linear accept a loopback redirect, and Linear any loopback port
/// (RFC 8252 §7.3). `storm://oauth`, which the spec also allows for Android
/// and macOS, needs a native intent filter and URL type that only a release
/// build can prove, so it waits for one (decision 81h).
///
/// The listener answers **only** the redirect path and closes after the first
/// request that carries a `state`, so it is open for one sign-in and nothing
/// else. It never sees a token: the code it relays is exchanged by the server.
Future<IntegrationCheck> signIn(
  IntegrationsApi api,
  String integrationId, {
  required Future<bool> Function(Uri url) openBrowser,
}) async {
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
  // Listening before the browser opens: the redirect can arrive at any
  // moment after, even before `openBrowser` returns.
  final redirected = _awaitRedirect(server);
  try {
    final redirect = 'http://127.0.0.1:${server.port}/oauth/callback';
    final url = await api.authorize(integrationId, redirectUri: redirect);
    final opened = await openBrowser(Uri.parse(url));
    if (!opened) {
      throw StateError('Could not open the browser.');
    }
    final params = await redirected.timeout(_deadline);
    final error = params['error'];
    if (error != null) {
      throw StateError(
        error == 'access_denied'
            ? 'Sign-in was cancelled.'
            : 'The service refused the sign-in.',
      );
    }
    final state = params['state'];
    final code = params['code'];
    if (state == null || code == null) {
      throw StateError('The sign-in came back incomplete.');
    }
    return await api.callback(state: state, code: code, iss: params['iss']);
  } finally {
    await server.close(force: true);
    // A listener that never got its redirect ends with the server; that end
    // is expected here, not an error anyone awaits.
    redirected.ignore();
  }
}

Future<Map<String, String>> _awaitRedirect(HttpServer server) async {
  await for (final request in server) {
    final query = request.uri.queryParameters;
    if (request.uri.path != '/oauth/callback' ||
        (query['state'] == null && query['error'] == null)) {
      request.response.statusCode = HttpStatus.notFound;
      await request.response.close();
      continue;
    }
    request.response
      ..statusCode = HttpStatus.ok
      ..headers.contentType = ContentType.html
      ..write(
        '<!doctype html><meta charset="utf-8"><title>Storm</title>'
        '<body style="font-family:system-ui;padding:3em">'
        '<h2>Storm</h2><p>You can close this tab and return to Storm.</p>',
      );
    await request.response.close();
    return query;
  }
  throw StateError('The sign-in listener closed.');
}

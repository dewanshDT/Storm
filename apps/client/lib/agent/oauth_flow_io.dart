import 'dart:async';
import 'dart:io';

import 'package:flutter/widgets.dart';

import 'integrations_api.dart';
import 'oauth_links.dart';

/// Native clients can sign in with OAuth.
const bool oauthSupported = true;

/// How long the browser has to come back.
const _deadline = Duration(minutes: 10);

/// Whether this platform signs in through `storm://oauth` rather than a
/// loopback listener. **The spec's split, exactly** (§10.4, decision 81l):
/// Android and macOS register the scheme; desktop Linux and Windows listen on
/// loopback.
bool get usesAppScheme => Platform.isAndroid || Platform.isMacOS;

/// Runs one sign-in: the authorization URL in the system browser, the one
/// redirect it sends back, and the relay of `{state, code}` to the server
/// (G-D13). The client never sees a token: the server exchanges the code.
///
/// `appScheme`, `links` and `resumes` are for tests; the app uses the
/// platform's own answer, [OAuthLinks.instance] and the app's lifecycle.
Future<IntegrationCheck> signIn(
  IntegrationsApi api,
  String integrationId, {
  required Future<bool> Function(Uri url) openBrowser,
  bool? appScheme,
  OAuthLinks? links,
  Stream<void>? resumes,
}) {
  if (appScheme ?? usesAppScheme) {
    return _signInWithScheme(
      api,
      integrationId,
      openBrowser,
      links ?? OAuthLinks.instance,
      resumes ?? _appResumes(),
    );
  }
  return _signInWithLoopback(api, integrationId, openBrowser);
}

/// Android and macOS: the redirect is `storm://oauth/callback`, which the
/// platform hands to the app, bringing it to the front.
///
/// The redirect is matched by the `state` in the authorization URL. If the
/// platform delivers it to a fresh instance instead of this one, that
/// instance relays it (an orphan, [OAuthLinks]); this one then learns the
/// sign-in finished when it is resumed and finds the integration connected,
/// rather than waiting out the deadline.
Future<IntegrationCheck> _signInWithScheme(
  IntegrationsApi api,
  String integrationId,
  Future<bool> Function(Uri url) openBrowser,
  OAuthLinks links,
  Stream<void> resumes,
) async {
  await links.start();
  final url = await api.authorize(integrationId, redirectUri: appRedirect);
  final state = Uri.parse(url).queryParameters['state'];
  if (state == null || state.isEmpty) {
    throw StateError('The service did not start a sign-in.');
  }
  final wasConnected = await _isConnected(api, integrationId);
  final redirected = links.waitFor(state);
  final finishedElsewhere = Completer<IntegrationCheck>();
  final watching = resumes.listen((_) async {
    if (finishedElsewhere.isCompleted || wasConnected) return;
    try {
      if (await _isConnected(api, integrationId)) {
        final check = await api.test(integrationId);
        if (!finishedElsewhere.isCompleted) finishedElsewhere.complete(check);
      }
    } catch (_) {
      // Still waiting: the redirect may yet arrive here.
    }
  });
  try {
    if (!await openBrowser(Uri.parse(url))) {
      throw StateError('Could not open the browser.');
    }
    return await Future.any([
      redirected.then((params) => _relay(api, params)),
      finishedElsewhere.future,
    ]).timeout(_deadline);
  } finally {
    links.cancel(state);
    await watching.cancel();
  }
}

Future<bool> _isConnected(IntegrationsApi api, String id) async {
  final all = await api.list();
  return all.any((i) => i.id == id && i.status == 'connected');
}

/// The app coming back to the foreground, as a stream.
Stream<void> _appResumes() {
  late final AppLifecycleListener listener;
  late final StreamController<void> resumed;
  resumed = StreamController<void>(
    onListen: () =>
        listener = AppLifecycleListener(onResume: () => resumed.add(null)),
    onCancel: () => listener.dispose(),
  );
  return resumed.stream;
}

/// Linux and Windows: a loopback listener on `127.0.0.1` with a port the OS
/// picks, for the one redirect.
///
/// It listens before the browser opens, answers **only** the redirect path,
/// and closes after the first request that carries a `state`, so it is open
/// for one sign-in and nothing else.
Future<IntegrationCheck> _signInWithLoopback(
  IntegrationsApi api,
  String integrationId,
  Future<bool> Function(Uri url) openBrowser,
) async {
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
    return await _relay(api, await redirected.timeout(_deadline));
  } finally {
    await server.close(force: true);
    // A listener that never got its redirect ends with the server; that end
    // is expected here, not an error anyone awaits.
    redirected.ignore();
  }
}

/// What the redirect brought back, relayed to the server — or the reason the
/// sign-in did not finish.
Future<IntegrationCheck> relayRedirect(
  IntegrationsApi api,
  Map<String, String> params,
) => _relay(api, params);

Future<IntegrationCheck> _relay(
  IntegrationsApi api,
  Map<String, String> params,
) async {
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
  return api.callback(state: state, code: code, iss: params['iss']);
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

import 'integrations_api.dart';

/// The web client cannot receive a redirect of its own (G-D13): it connects
/// integrations with a token.
const bool oauthSupported = false;

Future<IntegrationCheck> signIn(
  IntegrationsApi api,
  String integrationId, {
  required Future<bool> Function(Uri url) openBrowser,
}) => Future.error(
  UnsupportedError('Sign-in is not available in the browser; use a token.'),
);

/// The web registers no `storm://` scheme, so it never has a redirect to relay.
Future<IntegrationCheck> relayRedirect(
  IntegrationsApi api,
  Map<String, String> params,
) => Future.error(UnsupportedError('No sign-in redirects in the browser.'));

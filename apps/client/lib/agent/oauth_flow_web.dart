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

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/auth_api.dart';

/// Every device-tier call must carry the `StormDevice` credential.
///
/// `require_auth` puts `users`, `users/first`, `auth/login` and `auth/refresh`
/// behind the device tier, and refuses anything without that header with `401`.
/// `refresh` omitted it and was refused by every real server — silently, since
/// `refreshSession()` catches the failure and returns `false`, so a session
/// could never be renewed and nothing said so.
///
/// A mock answers whatever it is told to, so a test that only checks the parsed
/// result cannot see a missing header. These assert the *request*.
void main() {
  const deviceId = 'dev_1';
  const deviceSecret = 'secret';
  const expectedHeader = 'StormDevice $deviceId:$deviceSecret';

  /// Captures the one request the call makes.
  late http.Request seen;

  http.Client capturing(Object body) => MockClient((request) async {
    seen = request;
    return http.Response(jsonEncode(body), 200);
  });

  test('refresh sends the device credential', () async {
    final api = AuthApi(
      baseUrl: 'http://server',
      client: capturing({
        'session_id': 's',
        'user_id': 'u',
        'device_id': deviceId,
        'access_token': 'a',
        'refresh_token': 'r',
        'expires': '2026-01-01T00:00:00Z',
        'refresh_expires': '2026-06-01T00:00:00Z',
      }),
    );

    await api.refresh(
      'old-refresh',
      deviceId: deviceId,
      deviceSecret: deviceSecret,
    );

    expect(
      seen.headers['Authorization'],
      expectedHeader,
      reason: 'refresh is device tier; without this header it is a 401',
    );
    expect(jsonDecode(seen.body)['refresh_token'], 'old-refresh');
  });

  test('login sends the device credential', () async {
    final api = AuthApi(
      baseUrl: 'http://server',
      client: capturing({
        'session_id': 's',
        'user_id': 'u',
        'device_id': deviceId,
        'access_token': 'a',
        'refresh_token': 'r',
        'expires': '2026-01-01T00:00:00Z',
        'refresh_expires': '2026-06-01T00:00:00Z',
      }),
    );

    await api.login(
      deviceId: deviceId,
      deviceSecret: deviceSecret,
      username: 'dewansh',
      password: 'a-long-enough-password',
    );

    expect(seen.headers['Authorization'], expectedHeader);
  });

  test('listUsers sends the device credential', () async {
    final api = AuthApi(baseUrl: 'http://server', client: capturing([]));

    await api.listUsers(deviceId: deviceId, deviceSecret: deviceSecret);

    expect(seen.headers['Authorization'], expectedHeader);
  });
}

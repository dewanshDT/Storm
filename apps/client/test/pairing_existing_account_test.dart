import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/auth_api.dart';

/// Pairing and first-run are not the same thing.
///
/// The pairing screen assumed they were: every successful pair went straight
/// to "create the owner account". Invisible while a fresh server was the only
/// thing anyone paired with, and wrong the moment "Add a device" made joining
/// an *existing* server normal — someone adding their phone to a server they
/// already had an account on was asked to invent a second one.
///
/// The branch is `GET /v1/users` behind the new device credential, which is
/// what *Storm Auth Protocol* has always described. These pin the two answers
/// that decide which screen the client shows.
void main() {
  const deviceId = 'dev_1';
  const deviceSecret = 'secret';

  AuthApi apiReturning(Object users, {void Function(http.Request)? onCall}) =>
      AuthApi(
        baseUrl: 'http://server',
        client: MockClient((request) async {
          onCall?.call(request);
          return http.Response(jsonEncode(users), 200);
        }),
      );

  test('an empty user list is the first-run signal', () async {
    final api = apiReturning(const []);
    final users = await api.listUsers(
      deviceId: deviceId,
      deviceSecret: deviceSecret,
    );
    expect(
      users,
      isEmpty,
      reason: 'empty means this device creates the owner account',
    );
  });

  test('an existing account means sign in, not sign up', () async {
    final api = apiReturning(const [
      {
        'id': 'usr_1',
        'username': 'dewansh',
        'display_name': null,
        'role': 'Owner',
        'status': 'Active',
        'created': '2026-08-19T00:00:00Z',
        'last_login': null,
      },
    ]);

    final users = await api.listUsers(
      deviceId: deviceId,
      deviceSecret: deviceSecret,
    );

    expect(users, hasLength(1));
    expect(users.first.username, 'dewansh');
  });

  test('the question is asked with the device credential', () async {
    // It is a device-tier call: asked without the credential the server
    // answers 401, and the client would have no way to tell the two cases
    // apart — which is the same as never asking.
    http.Request? seen;
    final api = apiReturning(const [], onCall: (r) => seen = r);

    await api.listUsers(deviceId: deviceId, deviceSecret: deviceSecret);

    expect(seen!.headers['Authorization'], 'StormDevice $deviceId:$deviceSecret');
    expect(seen!.url.path, '/v1/users');
  });
}

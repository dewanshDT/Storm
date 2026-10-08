import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/auth_api.dart';

/// Pairing decides between setup and sign-in from `GET /v1/account`.
void main() {
  const deviceId = 'dev_1';
  const deviceSecret = 'secret';

  AuthApi apiReturning(bool exists) => AuthApi(
    baseUrl: 'http://server',
    client: MockClient(
      (_) async => http.Response(jsonEncode({'exists': exists}), 200),
    ),
  );

  test('no account means this device sets up the Storm', () async {
    expect(
      await apiReturning(
        false,
      ).accountExists(deviceId: deviceId, deviceSecret: deviceSecret),
      isFalse,
    );
  });

  test('an existing account means sign in, not set up', () async {
    expect(
      await apiReturning(
        true,
      ).accountExists(deviceId: deviceId, deviceSecret: deviceSecret),
      isTrue,
    );
  });
}

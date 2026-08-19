import 'package:flutter_test/flutter_test.dart';

import 'package:storm/api/auth_api.dart';

/// A device credential the server does not know is not a credential.
///
/// This is the state every client is left in when `auth.db` is wiped or
/// restored, and the state one client is left in when its device is revoked —
/// the second of which is a normal thing to do. Before this was named, the
/// client could not tell it apart from a wrong password, and could not
/// recover: it held an id that failed every device-tier call, and
/// `bootstrapWebDevice` short-circuits on `isPaired`, so a browser never minted
/// another one.
void main() {
  test('the server refusing a device is not a wrong password', () {
    // What `require_auth` actually answers for a device it does not know.
    expect(
      isDeviceRejected(AuthApiException(401, 'invalid or missing token')),
      isTrue,
    );
    expect(isDeviceRejected(AuthApiException(401, 'device_revoked')), isTrue);
    expect(isDeviceRejected(AuthApiException(401, 'not_paired')), isTrue);
  });

  test('a wrong password is left alone', () {
    // The distinction that matters: retyping fixes one of these and not the
    // other, and clearing a good device credential over a typo would throw
    // away a working pairing.
    expect(
      isDeviceRejected(AuthApiException(401, 'invalid_credentials')),
      isFalse,
    );
    expect(isDeviceRejected(AuthApiException(401, 'user_disabled')), isFalse);
    expect(
      isDeviceRejected(AuthApiException(429, 'rate_limited', retryAfter: 60)),
      isFalse,
      reason: 'a lockout is temporary; the device is fine',
    );
  });

  test('a non-401 is never a rejected device', () {
    // A 500 or a 403 says something else entirely, and treating either as a
    // dead device would silently unpair over a server-side fault.
    expect(
      isDeviceRejected(AuthApiException(500, 'invalid or missing token')),
      isFalse,
    );
    expect(
      isDeviceRejected(AuthApiException(403, 'registration_disabled')),
      isFalse,
    );
  });
}

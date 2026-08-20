/// Where the auth credentials live, which is not `shared_preferences`.
///
/// `shared_preferences` is a plain file on native and `localStorage` on web —
/// readable by anything running as the user, and included in a naive device
/// backup. That was tolerable when the only credential was a LAN-only shared
/// token. It stopped being tolerable when pairing and login added a device
/// secret and a refresh token beside it: the refresh token alone is 180 days
/// of access to every vault on the server (A6), and the device secret is what
/// lets a caller ask for a new one.
library;

import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// The keys that hold a credential, and therefore never belong in prefs.
///
/// `storm.token` — the shared token — was here until the cutover removed that
/// credential. An install upgrading from before then still has the value in
/// its keychain; it is simply never read again.
const secretKeys = <String>[
  'storm.deviceSecret',
  'storm.accessToken',
  'storm.refreshToken',
];

/// Reads and writes the credentials, and migrates them out of prefs once.
///
/// **Degrades rather than locks you out.** If the platform has no keychain
/// Storm can reach — a Linux box with no `libsecret`, say — this falls back to
/// `shared_preferences` and sets [degraded]. That is a deliberate trade: the
/// exposure being fixed here is a local-read risk, and refusing to start would
/// turn it into "the notes are gone" on the machine that holds them. The app
/// can say so; it must not strand anyone.
class SecretStore {
  SecretStore({FlutterSecureStorage? secure})
    : _secure = secure ?? const FlutterSecureStorage(mOptions: _macOptions);

  /// **`usesDataProtectionKeychain: false`, and it is not a downgrade to
  /// tidy up.** The data-protection keychain requires a
  /// `keychain-access-groups` entitlement, and that entitlement requires
  /// signing with a *development certificate* — Storm's macOS app is
  /// ad-hoc signed, so adding it does not weaken the build, it **fails** it:
  ///
  /// ```
  /// error: "Runner" has entitlements that require signing with a development
  /// certificate.
  /// ```
  ///
  /// The file-based keychain this selects instead is still the encrypted,
  /// per-user system keychain — which is the whole point of moving off
  /// `shared_preferences`. Revisit if the macOS app ever gets a real signing
  /// identity (it needs one for notarization anyway).
  static const _macOptions = MacOsOptions(usesDataProtectionKeychain: false);

  final FlutterSecureStorage _secure;

  /// True once a secure read or write has failed and prefs are carrying the
  /// credentials instead. Surfaced so the app can be honest about it.
  bool get degraded => _degraded;
  bool _degraded = false;

  /// Every credential, taking the keychain as authoritative and migrating
  /// anything still sitting in [prefs].
  ///
  /// Migration is **write-then-erase**, never move: the prefs copy is deleted
  /// only after the secure write has come back. The reverse order loses the
  /// credential outright if the write fails, which on this data means
  /// re-pairing a device to reach notes that were working a moment ago.
  Future<Map<String, String>> load(SharedPreferences prefs) async {
    Map<String, String> secure;
    try {
      secure = await _secure.readAll();
    } catch (e) {
      _degraded = true;
      debugPrint('secure storage unreadable, falling back to prefs: $e');
      return {for (final k in secretKeys) k: prefs.getString(k) ?? ''};
    }

    final out = <String, String>{};
    for (final key in secretKeys) {
      final fromSecure = secure[key];
      if (fromSecure != null && fromSecure.isNotEmpty) {
        out[key] = fromSecure;
        // A leftover copy from before the migration is still a copy. Erase it
        // even when the keychain already has the value.
        await _erase(prefs, key);
        continue;
      }

      final fromPrefs = prefs.getString(key) ?? '';
      out[key] = fromPrefs;
      if (fromPrefs.isEmpty) continue;

      try {
        await _secure.write(key: key, value: fromPrefs);
        await _erase(prefs, key);
      } catch (e) {
        _degraded = true;
        debugPrint('could not migrate $key into secure storage: $e');
      }
    }
    return out;
  }

  /// Persists the credentials, and keeps prefs clear of them.
  ///
  /// An empty value is a deletion — signing out writes `''`, and a stored
  /// empty string would otherwise read back as a credential that exists and
  /// is blank.
  Future<void> save(SharedPreferences prefs, Map<String, String> values) async {
    for (final key in secretKeys) {
      final value = values[key] ?? '';
      try {
        if (value.isEmpty) {
          await _secure.delete(key: key);
        } else {
          await _secure.write(key: key, value: value);
        }
        await _erase(prefs, key);
      } catch (e) {
        _degraded = true;
        debugPrint('secure storage unwritable, keeping $key in prefs: $e');
        // Losing the credential is worse than storing it where we already
        // were, so the fallback writes rather than drops.
        await prefs.setString(key, value);
      }
    }
  }

  /// Clears every credential from both stores. For `unpair()`.
  Future<void> clear(SharedPreferences prefs) async {
    await save(prefs, {for (final k in secretKeys) k: ''});
  }

  Future<void> _erase(SharedPreferences prefs, String key) async {
    if (prefs.containsKey(key)) await prefs.remove(key);
  }
}

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

import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// The credentials carried inside the one keychain item ([SecretStore]
/// writes them all as a single JSON blob now, not one item per key).
const secretKeys = <String>[
  'storm.deviceSecret',
  'storm.accessToken',
  'storm.refreshToken',
];

/// The pre-cutover shared token, and the pre-single-item keychain layout.
///
/// `storm.token` was a credential before the cutover that removed it;
/// `storm.deviceSecret`/`storm.accessToken`/`storm.refreshToken` as their own
/// keychain items were the layout before this file moved them into one blob.
/// An install upgrading from either state has them in its keychain; the
/// one-time migration in [SecretStore._migrateLegacy] erases all four, and
/// after that none of them is read again.
const _legacyTokenKey = 'storm.token';

/// Reads and writes the credentials, and migrates them out of prefs — and
/// out of their one-item-per-credential keychain layout — once.
///
/// **One keychain item, not three.** Storm's macOS app is ad-hoc signed (see
/// `.github/workflows/release.yml`), so its designated requirement is the
/// build's cdhash: every release is a new code identity, and the file-based
/// keychain's "Always Allow" is an access-control entry keyed to that
/// identity, not to Storm in general. A fresh install or update is therefore
/// at least one fresh prompt no matter what — there is no code identity
/// stable enough to avoid it without a real signing certificate — but it was
/// one prompt *per keychain item, per read and write*: `storm.deviceSecret`,
/// `storm.accessToken` and `storm.refreshToken` were three separate items,
/// each with its own access-control entry, so a new identity meant three or
/// four macOS password dialogs back to back on first launch. Writing every
/// credential as one JSON object under a single item ([_blobKey]) means a
/// new identity costs at most one prompt.
///
/// **Degrades rather than locks you out.** If the platform has no keychain
/// Storm can reach — a Linux box with no `libsecret`, say — this falls back
/// to `shared_preferences` and sets [degraded]. That is a deliberate trade:
/// the exposure being fixed here is a local-read risk, and refusing to start
/// would turn it into "the notes are gone" on the machine that holds them.
/// The app can say so; it must not strand anyone.
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

  /// The one keychain item every credential now lives in, as a JSON object
  /// keyed by [secretKeys]. One item on every platform, not macOS only —
  /// there is no platform reason to keep the three-item layout anywhere, and
  /// the blob is small enough for the tightest store among them (Windows
  /// Credential Manager's ~2.5 KB per item; three 256-bit tokens as base64
  /// plus their prefixes and JSON punctuation comes to a few hundred bytes).
  static const _blobKey = 'storm.credentials';

  final FlutterSecureStorage _secure;

  /// True once a secure read or write has failed and prefs are carrying the
  /// credentials instead. Surfaced so the app can be honest about it.
  bool get degraded => _degraded;
  bool _degraded = false;

  /// The map this store knows is already sitting in the keychain blob — from
  /// whichever [load] or [save] last touched it. `null` until one of those
  /// runs, so the first [save] on a fresh instance always writes: there is
  /// nothing yet to compare against.
  Map<String, String>? _lastPersisted;

  /// Every credential, taking the keychain blob as authoritative and
  /// migrating anything still sitting in [prefs] or in the old per-item
  /// keychain layout.
  Future<Map<String, String>> load(SharedPreferences prefs) async {
    String? blobRaw;
    try {
      blobRaw = await _secure.read(key: _blobKey);
    } catch (e) {
      _degraded = true;
      debugPrint('secure storage unreadable, falling back to prefs: $e');
      return _prefsFallback(prefs);
    }

    final decoded = blobRaw == null ? null : _decodeBlob(blobRaw);
    if (decoded != null) {
      _lastPersisted = decoded;
      return decoded;
    }

    return _migrateLegacy(prefs);
  }

  Map<String, String> _prefsFallback(SharedPreferences prefs) => {
    for (final k in secretKeys) k: prefs.getString(k) ?? '',
  };

  /// Reads the old one-item-per-credential keychain entries once, folds in
  /// anything still in [prefs], writes the single blob, and erases every
  /// legacy copy — the old keychain items, `storm.token`, and the prefs
  /// copies — in that order.
  ///
  /// Migration is **write-then-erase, never move**: the blob is confirmed
  /// written before a single legacy copy is removed. The reverse order loses
  /// a credential outright if the write fails, which on this data means
  /// re-pairing a device to reach notes that were working a moment ago.
  /// After the blob is written once, [load] never reaches this method again
  /// — it finds the blob first and stops there.
  Future<Map<String, String>> _migrateLegacy(SharedPreferences prefs) async {
    final legacy = <String, String?>{};
    try {
      for (final key in secretKeys) {
        legacy[key] = await _secure.read(key: key);
      }
    } catch (e) {
      _degraded = true;
      debugPrint('secure storage unreadable, falling back to prefs: $e');
      return _prefsFallback(prefs);
    }

    final out = <String, String>{};
    for (final key in secretKeys) {
      final fromSecure = legacy[key];
      out[key] = (fromSecure != null && fromSecure.isNotEmpty)
          ? fromSecure
          : (prefs.getString(key) ?? '');
    }

    if (out.values.every((v) => v.isEmpty)) {
      // Nothing anywhere to migrate — a fresh install. Leave the keychain
      // untouched; the first real `save()` writes the blob once instead.
      return out;
    }

    try {
      await _secure.write(key: _blobKey, value: _encodeBlob(out));
      _lastPersisted = out;
      for (final key in secretKeys) {
        await _secure.delete(key: key);
        await _erase(prefs, key);
      }
      await _secure.delete(key: _legacyTokenKey);
    } catch (e) {
      _degraded = true;
      debugPrint('could not migrate credentials into one keychain item: $e');
      // The write never confirmed, so nothing legacy is erased.
    }

    return out;
  }

  /// Persists the credentials as one keychain blob, and keeps prefs clear of
  /// them.
  ///
  /// Writes only when [values] differ from what this store last loaded or
  /// saved — `save(Settings next)` in `app_state.dart` calls this on *every*
  /// settings change, credential or not, so without the comparison a theme
  /// toggle would rewrite the keychain blob it never touched.
  ///
  /// An all-empty map is a deletion — signing out writes `''` for every
  /// key — and deletes the item outright rather than writing a blob of empty
  /// strings, so a cleared credential reads back as absent, not as a
  /// credential that exists and is blank.
  Future<void> save(SharedPreferences prefs, Map<String, String> values) async {
    final next = {for (final key in secretKeys) key: values[key] ?? ''};

    for (final key in secretKeys) {
      await _erase(prefs, key);
    }

    if (_lastPersisted != null && mapEquals(_lastPersisted, next)) return;

    final allEmpty = next.values.every((v) => v.isEmpty);
    try {
      if (allEmpty) {
        await _secure.delete(key: _blobKey);
      } else {
        await _secure.write(key: _blobKey, value: _encodeBlob(next));
      }
      _lastPersisted = next;
    } catch (e) {
      _degraded = true;
      debugPrint('secure storage unwritable, keeping credentials in prefs: $e');
      // Losing the credential is worse than storing it where we already
      // were, so the fallback writes rather than drops.
      for (final key in secretKeys) {
        await prefs.setString(key, next[key]!);
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

  static String _encodeBlob(Map<String, String> values) => jsonEncode(values);

  /// `null` on anything that isn't the JSON object this store writes —
  /// caller treats that exactly like no blob at all.
  static Map<String, String>? _decodeBlob(String raw) {
    try {
      final decoded = jsonDecode(raw);
      if (decoded is! Map) return null;
      return {
        for (final key in secretKeys) key: (decoded[key] as String?) ?? '',
      };
    } catch (_) {
      return null;
    }
  }
}

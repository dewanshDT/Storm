import 'package:flutter/services.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/state/secret_store.dart';

/// Getting the credentials out of `shared_preferences`, and keeping them out.
///
/// Three things carry this: the migration has to move an existing install's
/// secrets across *and erase the originals* — a copy left behind means the
/// change was cosmetic — a platform with no reachable keychain has to
/// degrade rather than lock someone out of their own notes — and, since the
/// fix for repeated macOS keychain prompts, every credential has to collapse
/// into one keychain item rather than one per credential.
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  const deviceSecret = 'storm.deviceSecret';
  const accessToken = 'storm.accessToken';
  const refreshToken = 'storm.refreshToken';

  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  Future<SharedPreferences> prefsWith(Map<String, Object> values) async {
    SharedPreferences.setMockInitialValues(values);
    return SharedPreferences.getInstance();
  }

  group('migration off shared_preferences', () {
    test(
      'moves an existing install\'s secrets and erases the originals',
      () async {
        final prefs = await prefsWith({
          'storm.baseUrl': 'http://vault.local',
          deviceSecret: 'dev-secret',
          accessToken: 'access-1',
          refreshToken: 'refresh-1',
        });
        final store = SecretStore();

        final loaded = await store.load(prefs);

        expect(loaded[deviceSecret], 'dev-secret');
        expect(loaded[accessToken], 'access-1');
        expect(loaded[refreshToken], 'refresh-1');
        expect(store.degraded, isFalse);

        // The point of the exercise: nothing left behind.
        for (final key in secretKeys) {
          expect(
            prefs.containsKey(key),
            isFalse,
            reason: '$key is still in shared_preferences',
          );
        }
        // And a second load reads them from the keychain.
        expect((await SecretStore().load(prefs))[refreshToken], 'refresh-1');

        // Non-secrets stay exactly where they were.
        expect(prefs.getString('storm.baseUrl'), 'http://vault.local');
      },
    );

    test(
      'erases a leftover prefs copy even when the keychain already has it',
      () async {
        // The state after a half-finished migration, or a downgrade and upgrade.
        // "The keychain already has it" is not a reason to leave a second copy
        // of a refresh token lying in a plain file.
        FlutterSecureStorage.setMockInitialValues({
          refreshToken: 'from-keychain',
        });
        final prefs = await prefsWith({refreshToken: 'stale-copy'});

        final loaded = await SecretStore().load(prefs);

        expect(loaded[refreshToken], 'from-keychain');
        expect(prefs.containsKey(refreshToken), isFalse);
      },
    );

    test(
      'a fresh install has nothing to migrate and nothing to report',
      () async {
        final prefs = await prefsWith({});
        final store = SecretStore();

        final loaded = await store.load(prefs);

        expect(loaded.values.every((v) => v.isEmpty), isTrue);
        expect(store.degraded, isFalse);
      },
    );
  });

  group('saving', () {
    test('writes to the keychain and never to prefs', () async {
      final prefs = await prefsWith({});
      final store = SecretStore();

      await store.save(prefs, {accessToken: 'access-2'});

      expect(prefs.containsKey(accessToken), isFalse);
      expect((await SecretStore().load(prefs))[accessToken], 'access-2');
    });

    test(
      'an empty value deletes rather than storing a blank credential',
      () async {
        final prefs = await prefsWith({});
        await SecretStore().save(prefs, {accessToken: 'access-2'});

        // What signing out does.
        await SecretStore().save(prefs, {accessToken: ''});

        final reloaded = await SecretStore().load(prefs);
        expect(reloaded[accessToken], isEmpty);
        final raw = await const FlutterSecureStorage().readAll();
        expect(
          raw.containsKey(accessToken),
          isFalse,
          reason:
              'a stored empty string reads back as a credential that exists',
        );
      },
    );

    test('clear() empties every credential', () async {
      final prefs = await prefsWith({});
      await SecretStore().save(prefs, {
        deviceSecret: 'd',
        accessToken: 'a',
        refreshToken: 'r',
      });

      await SecretStore().clear(prefs);

      final reloaded = await SecretStore().load(prefs);
      expect(reloaded.values.every((v) => v.isEmpty), isTrue);
    });
  });

  group('on the macOS file-based keychain', () {
    test('loads credentials without readAll, which it refuses', () async {
      // The bug: `readAll` asks for every item *with its data* in one query,
      // which the file-based keychain answers with errSecParam (-50). Writes
      // are per key and succeeded, so pairing worked and every relaunch came
      // up with no device secret — then the login was rejected and unpaired
      // the device. The mock keychain answers `readAll` happily, which is why
      // nothing caught it; this fake answers it the way macOS does.
      final prefs = await prefsWith({});
      final keychain = _FileKeychain();
      await SecretStore(secure: keychain).save(prefs, {
        deviceSecret: 'dev-secret',
        accessToken: 'access-1',
        refreshToken: 'refresh-1',
      });

      final store = SecretStore(secure: keychain);
      final loaded = await store.load(prefs);

      expect(loaded[deviceSecret], 'dev-secret');
      expect(loaded[accessToken], 'access-1');
      expect(loaded[refreshToken], 'refresh-1');
      expect(store.degraded, isFalse);
    });
  });

  group('collapsing into one keychain item', () {
    const blobKey = 'storm.credentials';

    test('migrates the old per-item keychain layout once, deletes it, and '
        'writes a single blob', () async {
      final prefs = await prefsWith({});
      final keychain = _CountingKeychain({
        deviceSecret: 'dev-secret',
        accessToken: 'access-1',
        refreshToken: 'refresh-1',
      });

      final loaded = await SecretStore(secure: keychain).load(prefs);

      expect(loaded[deviceSecret], 'dev-secret');
      expect(loaded[accessToken], 'access-1');
      expect(loaded[refreshToken], 'refresh-1');
      expect(keychain.writes[blobKey], 1);
      for (final key in secretKeys) {
        expect(keychain.reads[key], 1, reason: '$key read more than once');
        expect(keychain.deletes[key], 1, reason: '$key never erased');
      }

      // The blob exists now, so a second load never touches a legacy item.
      final again = await SecretStore(secure: keychain).load(prefs);
      expect(again[refreshToken], 'refresh-1');
      for (final key in secretKeys) {
        expect(
          keychain.reads[key],
          1,
          reason: '$key re-read after migration finished',
        );
      }
    });

    test('also erases the pre-cutover storm.token item', () async {
      final prefs = await prefsWith({});
      final keychain = _CountingKeychain({
        accessToken: 'access-1',
        'storm.token': 'shared-token',
      });

      await SecretStore(secure: keychain).load(prefs);

      expect(keychain.deletes['storm.token'], 1);
    });

    test(
      'a fresh install writes nothing until there is something to save',
      () async {
        final prefs = await prefsWith({});
        final keychain = _CountingKeychain();

        final loaded = await SecretStore(secure: keychain).load(prefs);

        expect(loaded.values.every((v) => v.isEmpty), isTrue);
        expect(keychain.writes, isEmpty);
      },
    );

    test('saving the same values again never touches the keychain', () async {
      final prefs = await prefsWith({});
      final keychain = _CountingKeychain();
      final store = SecretStore(secure: keychain);
      final values = {accessToken: 'access-1', refreshToken: 'refresh-1'};

      await store.save(prefs, values);
      expect(keychain.writes[blobKey], 1);

      await store.save(prefs, values);
      expect(
        keychain.writes[blobKey],
        1,
        reason: 'saving identical values wrote the keychain a second time',
      );
    });

    test(
      'signing out deletes the blob rather than writing empty strings',
      () async {
        final prefs = await prefsWith({});
        final keychain = _CountingKeychain();
        final store = SecretStore(secure: keychain);
        await store.save(prefs, {
          deviceSecret: 'd',
          accessToken: 'a',
          refreshToken: 'r',
        });

        await store.clear(prefs);

        expect(keychain.deletes[blobKey], 1);
        final reloaded = await SecretStore(secure: keychain).load(prefs);
        expect(reloaded.values.every((v) => v.isEmpty), isTrue);
      },
    );
  });

  group('when the keychain cannot be reached', () {
    test('reading falls back to prefs rather than losing access', () async {
      // A Linux box with no libsecret is the real case. Refusing to start
      // would turn a local-read risk into "the notes are gone" on the machine
      // that holds them.
      final prefs = await prefsWith({refreshToken: 'refresh-1'});
      final store = SecretStore(secure: _BrokenStorage());

      final loaded = await store.load(prefs);

      expect(loaded[refreshToken], 'refresh-1');
      expect(store.degraded, isTrue);
    });

    test('writing keeps the credential rather than dropping it', () async {
      final prefs = await prefsWith({});
      final store = SecretStore(secure: _BrokenStorage());

      await store.save(prefs, {accessToken: 'access-3'});

      expect(store.degraded, isTrue);
      expect(
        prefs.getString(accessToken),
        'access-3',
        reason: 'losing the credential is worse than storing it where it was',
      );
    });

    test('a failed migration leaves the prefs copy in place', () async {
      // Write-then-erase, not move. If the secure write fails, the only copy
      // must still be there — erasing first would mean re-pairing a device to
      // reach notes that were working a moment ago.
      final prefs = await prefsWith({deviceSecret: 'dev-secret'});
      final store = SecretStore(secure: _BrokenStorage());

      final loaded = await store.load(prefs);

      expect(loaded[deviceSecret], 'dev-secret');
      expect(prefs.getString(deviceSecret), 'dev-secret');
    });
  });
}

/// A keychain that is present but unusable — what an unsupported platform or a
/// locked secret service looks like from here.
class _BrokenStorage extends FlutterSecureStorage {
  const _BrokenStorage();

  @override
  Future<Map<String, String>> readAll({
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async => throw Exception('no secret service');

  @override
  Future<String?> read({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async => throw Exception('no secret service');

  @override
  Future<void> write({
    required String key,
    required String? value,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async => throw Exception('no secret service');

  @override
  Future<void> delete({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async => throw Exception('no secret service');
}

/// The macOS file-based keychain as `usesDataProtectionKeychain: false` sees
/// it: one item at a time works, and `readAll` — `kSecMatchLimitAll` with
/// `kSecReturnData` — is refused with errSecParam.
class _FileKeychain extends FlutterSecureStorage {
  _FileKeychain();

  final _items = <String, String>{};

  @override
  Future<Map<String, String>> readAll({
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async => throw PlatformException(code: '-50', message: 'errSecParam');

  @override
  Future<String?> read({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async => _items[key];

  @override
  Future<void> write({
    required String key,
    required String? value,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async {
    if (value == null) {
      _items.remove(key);
    } else {
      _items[key] = value;
    }
  }

  @override
  Future<void> delete({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async {
    _items.remove(key);
  }
}

/// Counts calls per key, to prove migration reads and erases a legacy item
/// exactly once, and that an unchanged `save()` never writes the keychain.
class _CountingKeychain extends FlutterSecureStorage {
  _CountingKeychain([Map<String, String>? seed]) : _items = {...?seed};

  final Map<String, String> _items;
  final Map<String, int> reads = {};
  final Map<String, int> writes = {};
  final Map<String, int> deletes = {};

  @override
  Future<String?> read({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async {
    reads[key] = (reads[key] ?? 0) + 1;
    return _items[key];
  }

  @override
  Future<void> write({
    required String key,
    required String? value,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async {
    writes[key] = (writes[key] ?? 0) + 1;
    if (value == null) {
      _items.remove(key);
    } else {
      _items[key] = value;
    }
  }

  @override
  Future<void> delete({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async {
    deletes[key] = (deletes[key] ?? 0) + 1;
    _items.remove(key);
  }
}

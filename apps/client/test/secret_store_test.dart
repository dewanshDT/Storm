import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'package:storm/state/secret_store.dart';

/// Getting the credentials out of `shared_preferences`, and keeping them out.
///
/// Two things carry this: the migration has to move an existing install's
/// secrets across *and erase the originals* — a copy left behind means the
/// change was cosmetic — and a platform with no reachable keychain has to
/// degrade rather than lock someone out of their own notes.
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  const token = 'storm.token';
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
          token: 'legacy-token',
          deviceSecret: 'dev-secret',
          accessToken: 'access-1',
          refreshToken: 'refresh-1',
        });
        final store = SecretStore();

        final loaded = await store.load(prefs);

        expect(loaded[token], 'legacy-token');
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
        token: 't',
        deviceSecret: 'd',
        accessToken: 'a',
        refreshToken: 'r',
      });

      await SecretStore().clear(prefs);

      final reloaded = await SecretStore().load(prefs);
      expect(reloaded.values.every((v) => v.isEmpty), isTrue);
    });
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

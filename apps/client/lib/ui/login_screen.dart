/// Sign in on a device that is already paired — or, on a fresh Storm reached
/// by web bootstrap, set up the account. One account, password only
/// (decision 82).
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/auth_api.dart';
import '../state/app_state.dart';
import '../state/web_bootstrap.dart';
import 'tokens.dart';
import 'widgets.dart';

class LoginScreen extends ConsumerStatefulWidget {
  const LoginScreen({super.key});

  @override
  ConsumerState<LoginScreen> createState() => _LoginScreenState();
}

class _LoginScreenState extends ConsumerState<LoginScreen> {
  final _passwordController = TextEditingController();

  bool _loading = true;
  bool _needsSetup = false;
  bool _busy = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    _loadAccountState();
  }

  @override
  void dispose() {
    _passwordController.dispose();
    super.dispose();
  }

  Future<void> _loadAccountState() async {
    final settings = ref.read(settingsProvider).value;
    if (settings == null || !settings.isPaired) {
      setState(() => _loading = false);
      return;
    }
    // Nothing to ask the server with. Asking anyway is a guaranteed 401 that
    // proves nothing about the device — see [Settings.deviceSecretMissing].
    if (settings.deviceSecretMissing) {
      setState(() => _loading = false);
      return;
    }
    final api = AuthApi(baseUrl: settings.baseUrl);
    try {
      final exists = await api.accountExists(
        deviceId: settings.deviceId,
        deviceSecret: settings.deviceSecret,
      );
      if (!mounted) return;
      setState(() {
        _needsSetup = !exists;
        _loading = false;
      });
    } on AuthApiException catch (e) {
      if (!mounted) return;
      setState(() {
        _loading = false;
        if (e.message == 'device_revoked' || e.message == 'not_paired') {
          _error = authFailureMessage(e);
        }
      });
    } catch (_) {
      if (mounted) setState(() => _loading = false);
    } finally {
      api.dispose();
    }
  }

  Future<void> _signIn() async {
    final settings = ref.read(settingsProvider).value;
    if (settings == null || _passwordController.text.isEmpty) return;

    setState(() {
      _busy = true;
      _error = null;
    });

    final api = AuthApi(baseUrl: settings.baseUrl);
    try {
      final tokens = await api.login(
        deviceId: settings.deviceId,
        deviceSecret: settings.deviceSecret,
        password: _passwordController.text,
      );
      await ref
          .read(settingsProvider.notifier)
          .save(
            settings.copyWith(
              accessToken: tokens.accessToken,
              refreshToken: tokens.refreshToken,
              accessTokenExpiresAt: tokens.expires,
              userId: tokens.userId,
            ),
          );
    } on AuthApiException catch (e) {
      if (!mounted) return;
      // The server no longer knows this device, so no password will help:
      // forget it and pair again (on the web, from a fresh document).
      //
      // Only when we actually presented a secret. A rejection of an empty one
      // is the server answering a question we never asked, and forgetting the
      // device on it deletes a credential that may be fine in the keychain.
      if (isDeviceRejected(e) && settings.deviceSecret.isNotEmpty) {
        await ref.read(settingsProvider.notifier).forgetDevice();
        if (!mounted) return;
        reloadForFreshBootstrap();
        setState(() {
          _error =
              'This device is no longer registered with the server. '
              'Setting it up again — sign in once more when it reloads.';
          _busy = false;
        });
        return;
      }
      setState(() {
        _error = authFailureMessage(e);
        _busy = false;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _error = "Couldn't reach the server.\n\n$e";
        _busy = false;
      });
    } finally {
      api.dispose();
    }
  }

  Future<void> _setUp() async {
    final settings = ref.read(settingsProvider).value;
    if (settings == null) return;
    if (_passwordController.text.length < 12) {
      setState(() => _error = 'Password must be at least 12 characters.');
      return;
    }

    setState(() {
      _busy = true;
      _error = null;
    });

    final api = AuthApi(baseUrl: settings.baseUrl);
    try {
      await api.setUpAccount(
        password: _passwordController.text,
        deviceId: settings.deviceId,
        deviceSecret: settings.deviceSecret,
      );
      if (!mounted) return;
      await _signIn();
    } on AuthApiException catch (e) {
      if (!mounted) return;
      setState(() {
        _error = e.isConflict
            ? 'This Storm is already set up. Sign in instead.'
            : 'Server error: ${e.message}';
        _needsSetup = !e.isConflict;
        _busy = false;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _error = "Couldn't reach the server.\n\n$e";
        _busy = false;
      });
    } finally {
      api.dispose();
    }
  }

  /// What a paired device whose secret did not load sees instead of a
  /// password field. No password can help — the request would carry half a
  /// credential — so it offers the two things that can: read the keychain
  /// again, or pair again on purpose. Neither happens behind anyone's back.
  List<Widget> _secretMissing(StormTokens t) => [
    Container(
      key: const Key('login-secret-missing'),
      padding: EdgeInsets.all(t.sp * 1.5),
      decoration: BoxDecoration(
        color: t.surface,
        borderRadius: BorderRadius.circular(t.rControl),
        border: Border.all(color: t.danger, width: t.bw),
      ),
      child: Text(
        "Storm couldn't read this device's credentials from the system "
        'keychain. Nothing has been removed — try again, or pair this '
        'device again if it keeps happening.',
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontSize: t.codeSize,
          color: t.danger,
        ),
      ),
    ),
    SizedBox(height: t.sp * 2.5),
    FilledButton(
      key: const Key('login-secret-retry'),
      // Rebuilding the provider re-runs `build()`, which reads the keychain
      // again; the router holds on /starting until it lands.
      onPressed: () => ref.invalidate(settingsProvider),
      child: const Text('Try again'),
    ),
    SizedBox(height: t.sp * 1.5),
    OutlinedButton(
      key: const Key('login-secret-repair'),
      onPressed: () async {
        await ref.read(settingsProvider.notifier).forgetDevice();
        reloadForFreshBootstrap();
      },
      child: const Text('Pair this device again'),
    ),
  ];

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final settings = ref.watch(settingsProvider).value;
    final password = _passwordController.text;
    final canSubmit =
        !_busy && (_needsSetup ? password.length >= 12 : password.isNotEmpty);
    final submit = _needsSetup ? _setUp : _signIn;
    final secretMissing = settings?.deviceSecretMissing ?? false;

    return Scaffold(
      body: Center(
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: t.sp * 52),
          child: Padding(
            padding: EdgeInsets.all(t.cardPad),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const Center(child: BrandMark(size: 44, withWordmark: true)),
                SizedBox(height: t.sp * 2),
                Text(
                  _needsSetup ? 'Set up your Storm' : 'Sign in',
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text3,
                  ),
                ),
                SizedBox(height: t.sp),
                Text(
                  settings?.baseUrl ?? '',
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text3,
                  ),
                ),
                SizedBox(height: t.sp * 3.5),
                if (secretMissing)
                  ..._secretMissing(t)
                else if (_loading)
                  const Center(child: CircularProgressIndicator())
                else
                  StormInput(
                    key: Key(
                      _needsSetup ? 'first-user-password' : 'login-password',
                    ),
                    controller: _passwordController,
                    labelText: _needsSetup
                        ? 'Choose a password (12+ characters)'
                        : 'Password',
                    obscureText: true,
                    autocorrect: false,
                    autofocus: true,
                    onChanged: (_) => setState(() {}),
                    onSubmitted: canSubmit ? (_) => submit() : null,
                  ),
                if (_error != null && !secretMissing) ...[
                  SizedBox(height: t.sp * 2),
                  Container(
                    padding: EdgeInsets.all(t.sp * 1.5),
                    decoration: BoxDecoration(
                      color: t.surface,
                      borderRadius: BorderRadius.circular(t.rControl),
                      border: Border.all(color: t.danger, width: t.bw),
                    ),
                    child: Text(
                      _error!,
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.codeSize,
                        color: t.danger,
                      ),
                    ),
                  ),
                ],
                if (!secretMissing) ...[
                  SizedBox(height: t.sp * 2.5),
                  FilledButton(
                    key: Key(
                      _needsSetup ? 'first-user-submit' : 'login-submit',
                    ),
                    onPressed: canSubmit ? submit : null,
                    child: _busy
                        ? const SizedBox(
                            height: 18,
                            width: 18,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        : Text(_needsSetup ? 'Set up & sign in' : 'Sign in'),
                  ),
                ],
                SizedBox(height: t.sp * 1.5),
                TextButton(
                  key: const Key('login-unpair'),
                  onPressed: _busy
                      ? null
                      : () => ref.read(settingsProvider.notifier).unpair(),
                  child: const Text('Use a different server'),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

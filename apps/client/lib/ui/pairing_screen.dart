/// First-run pairing screen — the bridge between a fresh install and a
/// paired, authenticated installation.
///
/// Flow:
/// 1. User pastes a `storm://pair` URI (or scans a QR on mobile later).
/// 2. Client verifies the server's identity via the challenge step.
/// 3. Client consumes the pairing nonce → device credentials.
/// 4. Client creates the owner account (first user).
/// 5. Client logs in → session tokens.
/// 6. Done — saved to Settings, router redirects to the dashboard.
library;

import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:package_info_plus/package_info_plus.dart';

import '../api/auth_api.dart';
import '../api/auth_models.dart';
import '../api/ed25519_verify.dart';
import '../state/app_state.dart';
import 'tokens.dart';
import 'widgets.dart';

class PairingScreen extends ConsumerStatefulWidget {
  const PairingScreen({super.key});

  @override
  ConsumerState<PairingScreen> createState() => _PairingScreenState();
}

class _PairingScreenState extends ConsumerState<PairingScreen> {
  // Step 1: paste URI.
  final _uriController = TextEditingController();
  PairingUri? _parsedUri;

  // Step 2: verify + pair.
  bool _verifying = false;
  String? _error;

  // Step 3: create account.
  final _usernameController = TextEditingController();
  final _passwordController = TextEditingController();
  bool _creatingAccount = false;

  // Step 4: login.
  bool _loggingIn = false;

  // Server info from verification.
  ServerInfo? _serverInfo;

  // Device credentials from pairing.
  PairingResult? _pairResult;

  @override
  void dispose() {
    _uriController.dispose();
    _usernameController.dispose();
    _passwordController.dispose();
    super.dispose();
  }

  // ---- Step 1: parse and verify ----

  void _onUriChanged(String value) {
    final parsed = PairingUri.parse(value.trim());
    setState(() {
      _parsedUri = parsed;
      _error = null;
    });
  }

  Future<void> _verifyAndPair() async {
    final uri = _parsedUri;
    if (uri == null) {
      setState(() => _error = 'Invalid storm://pair URI.');
      return;
    }
    if (uri.isExpired) {
      setState(
        () => _error = 'This pairing QR has expired. Ask for a new one.',
      );
      return;
    }

    setState(() {
      _verifying = true;
      _error = null;
    });

    try {
      final authApi = AuthApi(baseUrl: uri.address);

      // 1. Fetch server identity and verify it matches the QR.
      final serverInfo = await authApi.serverInfo();
      if (serverInfo.serverId != uri.serverId) {
        setState(() {
          _error = 'Server id mismatch. This QR may be for a different server.';
          _verifying = false;
        });
        return;
      }
      if (serverInfo.publicKey != uri.publicKey) {
        setState(() {
          _error =
              'Server public key mismatch. This QR may be for a different server.';
          _verifying = false;
        });
        return;
      }

      // 2. Challenge — send a random nonce, verify the Ed25519 signature.
      final challengeNonce = _randomNonce();
      final answer = await authApi.challenge(challengeNonce);
      final valid = await verifyChallenge(
        publicKeyB64: uri.publicKey,
        signatureB64: answer.signature,
        serverId: uri.serverId,
        nonce: challengeNonce,
      );
      if (!valid) {
        setState(() {
          _error =
              'Server failed the cryptographic challenge. Connection may be intercepted.';
          _verifying = false;
        });
        return;
      }

      // 3. Consume the pairing nonce.
      final packageInfo = await PackageInfo.fromPlatform();
      final pairResult = await authApi.pair(
        nonce: uri.nonce,
        deviceName: '${packageInfo.appName} (${_platformName()})',
        platform: _platformName(),
        version: packageInfo.version,
      );

      setState(() {
        _serverInfo = serverInfo;
        _pairResult = pairResult;
        _verifying = false;
      });
    } on AuthApiException catch (e) {
      setState(() {
        _error = e.isGone
            ? 'This pairing QR has expired. Ask for a new one.'
            : e.isConflict
            ? 'This pairing QR has already been used. Ask for a new one.'
            : e.isRateLimited
            ? 'Too many pairing attempts. Ask for a new QR code.'
            : 'Server error: ${e.message}';
        _verifying = false;
      });
    } catch (e) {
      setState(() {
        _error = "Couldn't reach the server.\n\n$e";
        _verifying = false;
      });
    }
  }

  // ---- Step 3: create account ----

  Future<void> _createAccount() async {
    final username = _usernameController.text.trim();
    final password = _passwordController.text;
    if (username.isEmpty || password.isEmpty) {
      setState(() => _error = 'Username and password are required.');
      return;
    }
    if (username.length < 3) {
      setState(() => _error = 'Username must be at least 3 characters.');
      return;
    }
    if (password.length < 8) {
      setState(() => _error = 'Password must be at least 8 characters.');
      return;
    }

    setState(() {
      _creatingAccount = true;
      _error = null;
    });

    try {
      final uri = _parsedUri!;
      final authApi = AuthApi(baseUrl: uri.address);
      // Device tier: the credential from pairing, a step earlier in this flow.
      final pair = _pairResult!;
      await authApi.createFirstUser(
        username: username,
        password: password,
        deviceId: pair.deviceId,
        deviceSecret: pair.deviceSecret,
      );
      if (!mounted) return;
      // Account created — now log in.
      await _login(username, password);
    } on AuthApiException catch (e) {
      setState(() {
        _error = e.isConflict
            ? 'That username is already taken.'
            : 'Server error: ${e.message}';
        _creatingAccount = false;
      });
    } catch (e) {
      setState(() {
        _error = "Couldn't reach the server.\n\n$e";
        _creatingAccount = false;
      });
    }
  }

  // ---- Step 4: login ----

  Future<void> _login(String username, String password) async {
    setState(() {
      _loggingIn = true;
      _creatingAccount = false;
      _error = null;
    });

    try {
      final uri = _parsedUri!;
      final pair = _pairResult!;
      final authApi = AuthApi(baseUrl: uri.address);
      final tokens = await authApi.login(
        deviceId: pair.deviceId,
        deviceSecret: pair.deviceSecret,
        username: username,
        password: password,
      );

      if (!mounted) return;

      // Save everything to Settings.
      final current = ref.read(settingsProvider).value ?? const Settings();
      // The server sends the absolute instant; there is nothing to compute.
      final expiresAt = tokens.expires;
      await ref
          .read(settingsProvider.notifier)
          .save(
            current.copyWith(
              baseUrl: uri.address,
              // Device credentials.
              deviceId: pair.deviceId,
              deviceSecret: pair.deviceSecret,
              serverId: _serverInfo!.serverId,
              serverKeyId: _serverInfo!.keyId,
              serverPublicKey: _serverInfo!.publicKey,
              // Session tokens.
              accessToken: tokens.accessToken,
              refreshToken: tokens.refreshToken,
              userId: tokens.userId,
              accessTokenExpiresAt: expiresAt,
              // Clear legacy token — we're paired now.
              token: '',
            ),
          );
    } on AuthApiException catch (e) {
      setState(() {
        _error = 'Login failed: ${e.message}';
        _loggingIn = false;
      });
    } catch (e) {
      setState(() {
        _error = "Login failed.\n\n$e";
        _loggingIn = false;
      });
    }
  }

  // ---- UI ----

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;

    // After pairing + account creation + login, show a brief success then the
    // router will redirect. During login, show a spinner.
    if (_loggingIn) {
      return Scaffold(
        body: Center(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const CircularProgressIndicator(),
              SizedBox(height: t.sp * 2),
              Text(
                'Signing in...',
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.bodySize,
                  color: t.text2,
                ),
              ),
            ],
          ),
        ),
      );
    }

    // Step 2+: show account creation form if device is paired.
    if (_pairResult != null) return _buildAccountForm(t);

    // Step 1: paste URI.
    return _buildUriInput(t);
  }

  Widget _buildUriInput(StormTokens t) {
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
                  'Pair with your Storm server',
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text3,
                  ),
                ),
                SizedBox(height: t.sp * 1),
                Text(
                  'Scan or paste the pairing QR from your server.',
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text3,
                  ),
                ),
                SizedBox(height: t.sp * 3.5),
                StormInput(
                  controller: _uriController,
                  autofocus: true,
                  labelText: 'Pairing URI',
                  hintText: 'storm://pair?v=1&sid=...',
                  autocorrect: false,
                  keyboardType: TextInputType.url,
                  onChanged: _onUriChanged,
                ),
                if (_error != null) ...[
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
                SizedBox(height: t.sp * 2.5),
                FilledButton(
                  onPressed: (_parsedUri != null && !_verifying)
                      ? _verifyAndPair
                      : null,
                  child: _verifying
                      ? const SizedBox(
                          height: 18,
                          width: 18,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : const Text('Verify & Pair'),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Widget _buildAccountForm(StormTokens t) {
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
                  'Create your account',
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text3,
                  ),
                ),
                SizedBox(height: t.sp * 1),
                Text(
                  'Paired with ${_parsedUri?.address ?? 'server'}.',
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.green,
                  ),
                ),
                SizedBox(height: t.sp * 3.5),
                StormInput(
                  controller: _usernameController,
                  autofocus: true,
                  labelText: 'Username',
                  autocorrect: false,
                  textInputAction: TextInputAction.next,
                ),
                SizedBox(height: t.sp * 1.75),
                StormInput(
                  controller: _passwordController,
                  labelText: 'Password',
                  obscureText: true,
                  autocorrect: false,
                  onSubmitted: (_) => _createAccount(),
                ),
                if (_error != null) ...[
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
                SizedBox(height: t.sp * 2.5),
                FilledButton(
                  onPressed: _creatingAccount ? null : _createAccount,
                  child: _creatingAccount
                      ? const SizedBox(
                          height: 18,
                          width: 18,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : const Text('Create Account & Sign In'),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  // ---- helpers ----

  /// A random 16-character nonce for the challenge step.
  static String _randomNonce() {
    const chars =
        'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
    final rng = Random.secure();
    return List.generate(16, (_) => chars[rng.nextInt(chars.length)]).join();
  }

  static String _platformName() {
    switch (defaultTargetPlatform) {
      case TargetPlatform.android:
        return 'android';
      case TargetPlatform.iOS:
        return 'ios';
      case TargetPlatform.macOS:
        return 'macos';
      case TargetPlatform.linux:
        return 'linux';
      case TargetPlatform.windows:
        return 'windows';
      case TargetPlatform.fuchsia:
        return 'fuchsia';
    }
  }
}

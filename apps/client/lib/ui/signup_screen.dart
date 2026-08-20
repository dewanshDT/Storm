/// Create an account on a server that is taking them (A13).
///
/// Reachable only when the server said registration is open — the login screen
/// hides the link otherwise, and `POST /v1/users` refuses regardless. Device
/// tier, like login: this device is paired, and being paired is what lets it
/// ask at all.
///
/// **Registration mints members.** The owner is the bootstrap account, made by
/// the first-run flow, and nothing here can produce another one.
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../api/auth_api.dart';
import '../router.dart';
import '../state/app_state.dart';
import 'tokens.dart';
import 'widgets.dart';

class SignupScreen extends ConsumerStatefulWidget {
  const SignupScreen({super.key});

  @override
  ConsumerState<SignupScreen> createState() => _SignupScreenState();
}

class _SignupScreenState extends ConsumerState<SignupScreen> {
  final _usernameController = TextEditingController();
  final _passwordController = TextEditingController();
  bool _submitting = false;
  String? _error;

  @override
  void dispose() {
    _usernameController.dispose();
    _passwordController.dispose();
    super.dispose();
  }

  Future<void> _create() async {
    final settings = ref.read(settingsProvider).value;
    if (settings == null) return;
    final username = _usernameController.text.trim();
    final password = _passwordController.text;

    if (username.length < 3) {
      setState(() => _error = 'Username must be at least 3 characters.');
      return;
    }
    // The server's own minimum. Accepting less here would mean a form that
    // takes a password and then a server that refuses it.
    if (password.length < 12) {
      setState(() => _error = 'Password must be at least 12 characters.');
      return;
    }

    setState(() {
      _submitting = true;
      _error = null;
    });

    final api = AuthApi(baseUrl: settings.baseUrl);
    try {
      await api.register(
        deviceId: settings.deviceId,
        deviceSecret: settings.deviceSecret,
        username: username,
        password: password,
      );
      // Straight into a session, so creating an account and using it are one
      // action rather than two.
      final tokens = await api.login(
        deviceId: settings.deviceId,
        deviceSecret: settings.deviceSecret,
        username: username,
        password: password,
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
      // The router takes it from here: there is a session now.
    } on AuthApiException catch (e) {
      if (!mounted) return;
      setState(() {
        // **Includes the case where the switch was turned off while this form
        // was open.** `registration_disabled` says exactly that rather than
        // leaving someone retyping a password that was never the problem.
        _error = authFailureMessage(e);
        _submitting = false;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _error = "Couldn't reach the server.\n\n$e";
        _submitting = false;
      });
    } finally {
      api.dispose();
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final settings = ref.watch(settingsProvider).value;

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
                  'Create an account',
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text3,
                  ),
                ),
                SizedBox(height: t.sp * 1),
                Text(
                  settings?.baseUrl ?? '',
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text3,
                  ),
                ),
                SizedBox(height: t.sp * 3),
                StormInput(
                  key: const Key('signup-username'),
                  controller: _usernameController,
                  labelText: 'Username',
                  autocorrect: false,
                  onChanged: (_) => setState(() {}),
                ),
                SizedBox(height: t.sp * 2),
                StormInput(
                  key: const Key('signup-password'),
                  controller: _passwordController,
                  labelText: 'Password',
                  obscureText: true,
                  autocorrect: false,
                  onChanged: (_) => setState(() {}),
                ),
                if (_error != null) ...[
                  SizedBox(height: t.sp * 2),
                  Text(
                    _error!,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.codeSize,
                      color: t.danger,
                    ),
                  ),
                ],
                SizedBox(height: t.sp * 3),
                FilledButton(
                  key: const Key('signup-submit'),
                  onPressed: _submitting ? null : _create,
                  child: _submitting
                      ? const SizedBox(
                          height: 18,
                          width: 18,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : const Text('Create account'),
                ),
                SizedBox(height: t.sp * 1.5),
                // **Always offered**, unlike its counterpart on the login
                // screen: signing in is possible on any server that has
                // accounts, so this link can never lead somewhere closed.
                TextButton(
                  key: const Key('signup-to-login'),
                  onPressed: _submitting
                      ? null
                      : () => context.go(Routes.login),
                  child: const Text('Sign in instead'),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

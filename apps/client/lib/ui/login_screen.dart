/// Sign in on a device that is already paired.
///
/// The gap `PairingScreen` deliberately left: that screen is first-run only, so
/// a device holding device credentials but no session had nowhere to enter a
/// password. Without this, `logout()` had no caller, because signing out would
/// have stranded you at a screen asking for a pairing QR you no longer need.
///
/// The account list comes from `GET /v1/users` on the **device** credential
/// (A7/A8) — a paired device may see who exists so it can offer a picker; a
/// stranger on the LAN may not.
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/auth_api.dart';
import '../api/auth_models.dart';
import '../state/app_state.dart';
import 'tokens.dart';
import 'widgets.dart';

class LoginScreen extends ConsumerStatefulWidget {
  const LoginScreen({super.key});

  @override
  ConsumerState<LoginScreen> createState() => _LoginScreenState();
}

class _LoginScreenState extends ConsumerState<LoginScreen> {
  final _passwordController = TextEditingController();
  final _usernameController = TextEditingController();

  List<AuthUser>? _users;
  String? _selected;
  bool _loadingUsers = true;
  bool _signingIn = false;
  String? _error;

  /// Set when the account list could not be fetched. The password form still
  /// works — you type the username instead — because a picker that failed to
  /// load must not be the thing standing between someone and their notes.
  bool _pickerUnavailable = false;

  @override
  void initState() {
    super.initState();
    _loadUsers();
  }

  @override
  void dispose() {
    _passwordController.dispose();
    _usernameController.dispose();
    super.dispose();
  }

  Future<void> _loadUsers() async {
    final settings = ref.read(settingsProvider).value;
    if (settings == null || !settings.isPaired) {
      setState(() {
        _loadingUsers = false;
        _pickerUnavailable = true;
      });
      return;
    }
    final api = AuthApi(baseUrl: settings.baseUrl);
    try {
      final users = await api.listUsers(
        deviceId: settings.deviceId,
        deviceSecret: settings.deviceSecret,
      );
      if (!mounted) return;
      setState(() {
        _users = users;
        // Preselect when there is exactly one account, which is the homelab
        // case — the picker is then a label, not a decision.
        final usable = users.where((u) => !u.isDisabled).toList();
        _selected = usable.length == 1 ? usable.first.username : null;
        _loadingUsers = false;
      });
    } on AuthApiException catch (e) {
      if (!mounted) return;
      setState(() {
        _loadingUsers = false;
        _pickerUnavailable = true;
        // A revoked device is the one failure worth surfacing here: no password
        // will fix it, and the remedy is to pair again.
        if (e.message == 'device_revoked' || e.message == 'not_paired') {
          _error = authFailureMessage(e);
        }
      });
    } catch (_) {
      if (!mounted) return;
      setState(() {
        _loadingUsers = false;
        _pickerUnavailable = true;
      });
    } finally {
      api.dispose();
    }
  }

  String get _username => _selected ?? _usernameController.text.trim();

  Future<void> _signIn() async {
    final settings = ref.read(settingsProvider).value;
    if (settings == null) return;
    final username = _username;
    if (username.isEmpty || _passwordController.text.isEmpty) return;

    setState(() {
      _signingIn = true;
      _error = null;
    });

    final api = AuthApi(baseUrl: settings.baseUrl);
    try {
      final tokens = await api.login(
        deviceId: settings.deviceId,
        deviceSecret: settings.deviceSecret,
        username: username,
        password: _passwordController.text,
      );
      final expiresAt = DateTime.now()
          .add(Duration(seconds: tokens.accessExpiresIn))
          .toUtc()
          .toIso8601String();
      await ref
          .read(settingsProvider.notifier)
          .save(
            settings.copyWith(
              accessToken: tokens.accessToken,
              refreshToken: tokens.refreshToken,
              accessTokenExpiresAt: expiresAt,
              userId: tokens.userId,
              // A stale shared token would otherwise outlive the sign-in and
              // keep being preferred over nothing if this session is dropped.
              token: '',
            ),
          );
      // The router's redirect takes it from here: `hasSession` is now true, so
      // /login sends us to the dashboard.
    } on AuthApiException catch (e) {
      if (!mounted) return;
      setState(() {
        _error = authFailureMessage(e);
        _signingIn = false;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        // Not an auth refusal — this is the genuinely-unreachable case, and it
        // is the only one allowed to say so.
        _error = "Couldn't reach the server.\n\n$e";
        _signingIn = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final settings = ref.watch(settingsProvider).value;
    final canSubmit =
        !_signingIn &&
        _username.isNotEmpty &&
        _passwordController.text.isNotEmpty;

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
                  'Sign in',
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text3,
                  ),
                ),
                SizedBox(height: t.sp * 1),
                Text(
                  // The address, because a paired device may be one of several
                  // and "sign in" alone does not say to what.
                  settings?.baseUrl ?? '',
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text3,
                  ),
                ),
                SizedBox(height: t.sp * 3.5),

                if (_loadingUsers)
                  const Center(child: CircularProgressIndicator())
                else ...[
                  if (_users != null && _users!.isNotEmpty)
                    _AccountPicker(
                      users: _users!,
                      selected: _selected,
                      onChanged: _signingIn
                          ? null
                          : (u) => setState(() => _selected = u),
                    )
                  else if (_pickerUnavailable)
                    StormInput(
                      key: const Key('login-username'),
                      controller: _usernameController,
                      labelText: 'Username',
                      autocorrect: false,
                      onChanged: (_) => setState(() {}),
                    ),
                  SizedBox(height: t.sp * 2),
                  StormInput(
                    key: const Key('login-password'),
                    controller: _passwordController,
                    labelText: 'Password',
                    obscureText: true,
                    autofocus: _selected != null,
                    onChanged: (_) => setState(() {}),
                    onSubmitted: canSubmit ? (_) => _signIn() : null,
                  ),
                ],

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
                  key: const Key('login-submit'),
                  onPressed: canSubmit ? _signIn : null,
                  child: _signingIn
                      ? const SizedBox(
                          height: 18,
                          width: 18,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : const Text('Sign in'),
                ),
                SizedBox(height: t.sp * 1.5),
                TextButton(
                  key: const Key('login-unpair'),
                  onPressed: _signingIn
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

/// The account list, as a picker rather than a username field.
class _AccountPicker extends StatelessWidget {
  const _AccountPicker({
    required this.users,
    required this.selected,
    required this.onChanged,
  });

  final List<AuthUser> users;
  final String? selected;
  final ValueChanged<String?>? onChanged;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    // `RadioGroup` rather than per-tile `groupValue`/`onChanged`, which this
    // Flutter deprecates.
    return RadioGroup<String>(
      groupValue: selected,
      // `RadioGroup` wants a non-null callback, so "disabled" is expressed on
      // the tiles instead — a null `onChanged` here is the sign-in being in
      // flight, and every tile goes inert with it.
      onChanged: onChanged ?? (_) {},
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final user in users)
            Padding(
              padding: EdgeInsets.only(bottom: t.sp * 0.5),
              child: RadioListTile<String>(
                key: Key('login-account-${user.username}'),
                contentPadding: EdgeInsets.zero,
                dense: true,
                value: user.username,
                // A disabled account cannot log in, so offering it would only
                // produce a refusal after the password was typed.
                enabled: !user.isDisabled && onChanged != null,
                title: Text(
                  user.label,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: user.isDisabled ? t.text3 : null,
                  ),
                ),
                subtitle: user.isDisabled
                    ? Text(
                        'Disabled',
                        style: TextStyle(
                          fontFamily: StormTokens.sansFamily,
                          fontSize: t.codeSize * 0.9,
                          color: t.text3,
                        ),
                      )
                    : null,
              ),
            ),
        ],
      ),
    );
  }
}

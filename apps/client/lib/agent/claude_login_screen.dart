import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../ui/controls.dart';
import '../ui/session_status.dart' show StatusPill;
import '../ui/settings/settings_widgets.dart';
import '../ui/states.dart';
import '../ui/tokens.dart';
import 'agent_models.dart' show AgentHost;
import 'agent_state.dart' show agentApi;
import 'claude_login_api.dart';
import 'integrations_screen.dart' show openBrowserProvider;

const _months = [
  'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', //
  'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec',
];

String claudeLoginDate(DateTime at) {
  final d = at.toLocal();
  return '${d.day} ${_months[d.month - 1]} ${d.year}';
}

String _days(int? n) => n == null ? '' : '$n ${n == 1 ? 'day' : 'days'}';

/// The status line (§4): "Not synced", or "Synced · fingerprint … · minted …
/// · expires …".
String claudeLoginStatusLine(ClaudeLogin login) {
  if (!login.present) return 'Not synced';
  return [
    login.status == 'needs_resync' ? 'Unreadable' : 'Synced',
    if (login.fingerprint != null) 'fingerprint ${login.fingerprint}',
    if (login.mintedAt != null) 'minted ${claudeLoginDate(login.mintedAt!)}',
    if (login.expiresAt != null)
      '${login.warn == 'expired' ? 'expired' : 'expires'} '
          '${claudeLoginDate(login.expiresAt!)}',
  ].join(' · ');
}

/// The banner for the expiry and unreadable states (§3, §6), or null.
String? claudeLoginBanner(ClaudeLogin login) => switch (login.state) {
  ClaudeLoginState.expiring30 || ClaudeLoginState.expiring7 =>
    'Your synced Claude login expires in ${_days(login.daysLeft)}'
        '${login.expiresAt == null ? '' : ', on ${claudeLoginDate(login.expiresAt!)}'}. '
        'Mint a new token, use Replace token, then revoke the old one on '
        'claude.ai.',
  ClaudeLoginState.expired =>
    'Your synced Claude login has expired. Sessions on ticked hosts use '
        "each host's own login until you sync a new token.",
  ClaudeLoginState.needsResync =>
    "Your synced Claude token can't be read on this server (its key is "
        'missing). Sync it again.',
  _ => null,
};

/// Settings ▸ Claude login (decision 124, token sync spec §4).
///
/// The token is pasted once into an obscured field, sent in one request and
/// forgotten: nothing on this screen or in any provider keeps it. The
/// server's 403 decides who sees the page, as on Integrations.
class ClaudeLoginScreen extends ConsumerStatefulWidget {
  const ClaudeLoginScreen({super.key});

  @override
  ConsumerState<ClaudeLoginScreen> createState() => _ClaudeLoginScreenState();
}

class _ClaudeLoginScreenState extends ConsumerState<ClaudeLoginScreen> {
  ClaudeLogin? _login;
  List<AgentHost>? _hosts;
  Set<String> _ticked = {};
  String? _error;
  bool _busy = false;

  ClaudeLoginApi? _api() => ref.read(claudeLoginApiFactoryProvider)?.call();

  @override
  void initState() {
    super.initState();
    _load();
    _loadHosts();
  }

  void _adopt(ClaudeLogin login) {
    _login = login;
    _ticked = {...login.hosts};
  }

  Future<void> _load() async {
    final api = _api();
    if (api == null) {
      setState(() => _error = 'Sign in first.');
      return;
    }
    try {
      final login = await api.get();
      if (mounted) setState(() => _adopt(login));
    } catch (e) {
      if (mounted) setState(() => _error = describeFailure(e));
    } finally {
      api.dispose();
    }
  }

  Future<void> _loadHosts() async {
    final api = agentApi(ref);
    if (api == null) return;
    try {
      final hosts = [
        for (final h in await api.hosts())
          if (h.status != 'revoked') h,
      ];
      if (mounted) setState(() => _hosts = hosts);
    } catch (_) {
      if (mounted) setState(() => _hosts = const []);
    } finally {
      api.dispose();
    }
  }

  /// Runs [action], adopting the view it returns; failures are said, not
  /// thrown.
  Future<bool> _with(
    Future<ClaudeLogin?> Function(ClaudeLoginApi api) action,
  ) async {
    final api = _api();
    if (api == null) return false;
    setState(() => _busy = true);
    try {
      final login = await action(api);
      if (mounted) setState(() => _adopt(login ?? const ClaudeLogin.none()));
      ref.invalidate(claudeLoginSummaryProvider);
      return true;
    } catch (e) {
      if (mounted) settingsToast(context, describeFailure(e));
      return false;
    } finally {
      api.dispose();
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _sync() async {
    final replacing = _login?.present ?? false;
    final draft = await showDialog<_SyncDraft>(
      context: context,
      builder: (_) => _SyncDialog(replacing: replacing),
    );
    if (draft == null || !mounted) return;
    final ok = await _with(
      (api) => api.put(draft.token, mintedAt: draft.mintedAt),
    );
    if (!ok || !mounted) return;
    settingsToast(
      context,
      replacing
          ? 'Replaced. The old token still works at Anthropic until you '
                'revoke it on claude.ai.'
          : _ticked.isEmpty
          ? 'Synced. No host gets it until you tick one below.'
          : 'Synced. The first Claude Code session on a ticked host is the '
                'test.',
    );
  }

  Future<void> _remove() async {
    final ok = await confirmSetting(
      context,
      title: 'Remove the synced Claude login?',
      body:
          'This stops Storm giving the token to new sessions. To make it stop '
          'working, revoke it in your Claude account settings, then end your '
          'running sessions.',
      action: 'Remove',
      confirmKey: const Key('claude-remove-confirm'),
    );
    if (!ok) return;
    await _with((api) async {
      await api.delete();
      return const ClaudeLogin.none();
    });
  }

  Future<void> _saveHosts() async {
    final enrolled = {for (final h in _hosts ?? const <AgentHost>[]) h.id};
    final hosts = [
      for (final id in _ticked)
        if (enrolled.contains(id)) id,
    ]..sort();
    await _with((api) => api.setHosts(hosts));
  }

  bool get _hostsDirty {
    final saved = {...?_login?.hosts};
    return saved.length != _ticked.length || !saved.containsAll(_ticked);
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final login = _login;
    return SettingsPage(
      title: 'Claude login',
      beta: true,
      intro:
          'Sign in to Claude once, here, and Claude Code sessions on the hosts '
          'you tick use that login.',
      children: [
        const InfoBox(
          'This is for one person, on their own machines, for their own work. '
          'Do not use it to share a Claude account, to give other people '
          'access to your plan, or on machines you do not control.\n\n'
          'You are responsible for following Anthropic\'s terms. Anthropic\'s '
          'documentation says third-party developers "may not collect, store, '
          'or intermediate Claude.ai credentials or session tokens". Storm '
          'stores your token at your request, on your server. If you are not '
          'comfortable with that risk to your account, don\'t turn this on: '
          'log in on each host instead.\n\n'
          'Only claude.ai can revoke the token: remove it in your Claude '
          'account settings there, then end any running sessions on ticked '
          'hosts. Removing it here stops new sessions from getting it.',
          key: Key('claude-login-notice'),
        ),
        SizedBox(height: t.sp),
        Align(
          alignment: Alignment.centerLeft,
          child: TextAction(
            key: const Key('claude-login-readme'),
            label: 'How Claude login sync works ›',
            accent: true,
            onTap: () =>
                ref.read(openBrowserProvider)(Uri.parse(kClaudeLoginReadme)),
          ),
        ),
        if (_error != null)
          SettingsMuted(_error!, danger: true)
        else if (login == null)
          const SkeletonRows(rows: 2)
        else ...[
          GroupLabel('Token', top: t.sp * 3),
          if (claudeLoginBanner(login) case final banner?)
            Padding(
              padding: EdgeInsets.only(bottom: t.sp * 1.5),
              child: _Banner(
                key: Key('claude-login-banner-${login.state.name}'),
                text: banner,
                strong: login.state != ClaudeLoginState.expiring30,
              ),
            ),
          _StatusRow(login: login),
          SettingsButtonRow(
            child: Wrap(
              spacing: t.sp * 1.5,
              runSpacing: t.sp,
              crossAxisAlignment: WrapCrossAlignment.center,
              children: [
                StormButton.primary(
                  key: const Key('claude-sync'),
                  label: switch (login.state) {
                    ClaudeLoginState.notSynced => 'Sync a token',
                    ClaudeLoginState.needsResync => 'Sync again',
                    _ => 'Replace token',
                  },
                  onPressed: _busy ? null : _sync,
                ),
                if (login.present)
                  TextAction(
                    key: const Key('claude-remove'),
                    label: 'Remove',
                    danger: true,
                    onTap: _busy ? null : _remove,
                  ),
              ],
            ),
          ),
          SizedBox(height: t.sp * 1.5),
          const SettingsNote(
            'Storm never tests the token with Anthropic: the first Claude Code '
            'session on a ticked host is the test. If sessions on synced hosts '
            'say the login is revoked or expired, sync a new token.',
            small: true,
          ),
          GroupLabel('Hosts', top: t.sp * 3),
          const SettingsNote(
            'Claude Code sessions on a ticked host use this login; every other '
            'host uses its own. No host gets it until you tick it, including '
            'hosts you enroll later.',
          ),
          SizedBox(height: t.sp),
          ..._hostRows(),
          Padding(
            padding: EdgeInsets.only(top: t.sp * 1.5),
            child: Text(
              'Every agent on a ticked host can read this token. Tick only '
              'your own machines.',
              key: const Key('claude-hosts-warning'),
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                color: t.text2,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
          SettingsButtonRow(
            child: StormButton.outline(
              key: const Key('claude-hosts-save'),
              label: 'Save hosts',
              onPressed: _busy || !_hostsDirty || _hosts == null
                  ? null
                  : _saveHosts,
            ),
          ),
        ],
      ],
    );
  }

  List<Widget> _hostRows() {
    final hosts = _hosts;
    if (hosts == null) return const [SkeletonRows(rows: 2)];
    if (hosts.isEmpty) {
      return const [
        SettingsMuted('No hosts are enrolled. Enroll one in Settings › Hosts.'),
      ];
    }
    return [
      for (final h in hosts)
        CheckboxListTile(
          key: Key('claude-host-${h.id}'),
          contentPadding: EdgeInsets.zero,
          controlAffinity: ListTileControlAffinity.leading,
          value: _ticked.contains(h.id),
          onChanged: _busy
              ? null
              : (v) => setState(
                  () => v == true ? _ticked.add(h.id) : _ticked.remove(h.id),
                ),
          title: Text(h.name),
          subtitle: Text(
            [
              h.online ? 'Online' : 'Offline',
              if (h.capabilitiesKnown && !h.receivesClaudeLogin)
                'update storm-runtime to use this',
            ].join(' · '),
            key: Key('claude-host-detail-${h.id}'),
          ),
        ),
    ];
  }
}

class _StatusRow extends StatelessWidget {
  const _StatusRow({required this.login});

  final ClaudeLogin login;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final (pill, color) = switch (login.state) {
      ClaudeLoginState.notSynced => ('Not synced', t.text3),
      ClaudeLoginState.active => ('Synced', t.text2),
      ClaudeLoginState.needsResync => ('Needs re-sync', t.danger),
      ClaudeLoginState.expiring30 => (
        'Expires in ${_days(login.daysLeft)}',
        t.amber,
      ),
      ClaudeLoginState.expiring7 => (
        'Expires in ${_days(login.daysLeft)}',
        t.danger,
      ),
      ClaudeLoginState.expired => ('Expired', t.danger),
    };
    return Container(
      padding: EdgeInsets.symmetric(vertical: t.sp * 1.5),
      decoration: BoxDecoration(
        border: Border(
          bottom: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'Claude subscription token',
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.uiSize,
                    color: t.text,
                  ),
                ),
                SizedBox(height: t.sp * 0.25),
                Text(
                  claudeLoginStatusLine(login),
                  key: const Key('claude-login-status'),
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text3,
                    height: 1.45,
                  ),
                ),
              ],
            ),
          ),
          SizedBox(width: t.sp * 1.5),
          StatusPill(
            key: Key('claude-login-pill-${login.state.name}'),
            label: pill,
            color: color,
          ),
        ],
      ),
    );
  }
}

/// `amberSoft` at 30 days; a `danger` edge at 7 days, expired and unreadable.
class _Banner extends StatelessWidget {
  const _Banner({super.key, required this.text, required this.strong});

  final String text;
  final bool strong;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Container(
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 1.75,
        vertical: t.sp * 1.5,
      ),
      decoration: BoxDecoration(
        color: strong ? t.surface : t.amberSoft,
        borderRadius: BorderRadius.circular(t.rControl),
        border: Border.all(
          color: strong ? t.danger : t.amber,
          width: strong ? t.bw * 2 : t.bw,
        ),
      ),
      child: Text(
        text,
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontSize: t.codeSize,
          color: t.text,
          height: 1.55,
        ),
      ),
    );
  }
}

class _SyncDraft {
  const _SyncDraft(this.token, this.mintedAt);

  final String token;

  /// Null: today, which the server stamps itself.
  final DateTime? mintedAt;
}

/// Sync a token / Replace token (§2.3). The warnings come before the field.
class _SyncDialog extends StatefulWidget {
  const _SyncDialog({required this.replacing});

  final bool replacing;

  @override
  State<_SyncDialog> createState() => _SyncDialogState();
}

class _SyncDialogState extends State<_SyncDialog> {
  final _token = TextEditingController();
  late final DateTime _today = _dateOnly(DateTime.now());
  late DateTime _minted = _today;
  String? _problem;

  static DateTime _dateOnly(DateTime d) => DateTime(d.year, d.month, d.day);

  @override
  void dispose() {
    _token.clear();
    _token.dispose();
    super.dispose();
  }

  Future<void> _pickDate() async {
    final picked = await showDatePicker(
      context: context,
      initialDate: _minted,
      firstDate: _today.subtract(const Duration(days: 365)),
      lastDate: _today,
      helpText: 'Minted on',
    );
    if (picked != null && mounted) setState(() => _minted = _dateOnly(picked));
  }

  void _submit() {
    final cleaned = cleanClaudeToken(_token.text);
    _token.clear();
    if (cleaned == null) {
      setState(() => _problem = kClaudeTokenRule);
      return;
    }
    Navigator.of(
      context,
    ).pop(_SyncDraft(cleaned, _minted == _today ? null : _minted));
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final small = TextStyle(
      fontSize: t.labelSize * 1.09,
      color: t.text3,
      height: 1.5,
    );
    return AlertDialog(
      title: Text(widget.replacing ? 'Replace token' : 'Sync a token'),
      content: SizedBox(
        width: 460,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text.rich(
                TextSpan(
                  children: [
                    const TextSpan(text: 'Run '),
                    TextSpan(
                      text: 'claude setup-token',
                      style: TextStyle(fontFamily: StormTokens.monoFamily),
                    ),
                    const TextSpan(
                      text:
                          ' in a terminal outside Storm, approve it in your '
                          'browser, and paste the token here.',
                    ),
                  ],
                ),
              ),
              SizedBox(height: t.sp),
              Text(
                'If you run it inside a Storm session, the token stays in that '
                "session's scrollback: end and dismiss the session, and mint a "
                'new token.',
                key: const Key('claude-scrollback-warning'),
                style: small.copyWith(color: t.text2),
              ),
              SizedBox(height: t.sp * 1.5),
              TextField(
                key: const Key('claude-token-field'),
                controller: _token,
                autofocus: true,
                obscureText: true,
                autocorrect: false,
                enableSuggestions: false,
                enableIMEPersonalizedLearning: false,
                keyboardType: TextInputType.visiblePassword,
                inputFormatters: [LengthLimitingTextInputFormatter(1024)],
                decoration: const InputDecoration(
                  labelText: 'Token',
                  hintText: 'sk-ant-oat01-…',
                ),
                onSubmitted: (_) => _submit(),
              ),
              SizedBox(height: t.sp * 1.5),
              Row(
                children: [
                  Expanded(
                    child: Text(
                      'Minted on ${claudeLoginDate(_minted)}'
                      '${_minted == _today ? ' (today)' : ''}',
                      key: const Key('claude-minted'),
                    ),
                  ),
                  TextAction(
                    key: const Key('claude-minted-change'),
                    label: 'Change',
                    accent: true,
                    onTap: _pickDate,
                  ),
                ],
              ),
              SizedBox(height: t.sp * 0.5),
              Text(
                'A token lasts a year from the day it was minted. Storm '
                "can't read the date from it, so set it if you made it earlier.",
                style: small,
              ),
              SizedBox(height: t.sp),
              Text(
                'For your own machines only. Storm stores it encrypted on your '
                'server and gives it only to the hosts you tick.',
                style: small,
              ),
              if (widget.replacing) ...[
                SizedBox(height: t.sp),
                Text(
                  'The old token keeps working at Anthropic until you revoke '
                  'it on claude.ai.',
                  style: small,
                ),
              ],
              if (_problem != null) ...[
                SizedBox(height: t.sp),
                Text(
                  _problem!,
                  key: const Key('claude-token-problem'),
                  style: TextStyle(color: t.danger),
                ),
              ],
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('claude-token-save'),
          onPressed: _submit,
          child: Text(widget.replacing ? 'Replace' : 'Sync'),
        ),
      ],
    );
  }
}

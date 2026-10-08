import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:url_launcher/url_launcher.dart';

import '../ui/breakpoints.dart';
import '../ui/controls.dart';
import '../ui/session_status.dart' show StatusPill;
import '../ui/settings/settings_widgets.dart';
import '../ui/states.dart';
import '../ui/tokens.dart';
import 'agent_state.dart' show sessionCredentialsProvider;
import 'integrations_api.dart';
import 'oauth_flow.dart';
import 'oauth_links.dart';

/// How this client builds an [IntegrationsApi], or null without a session.
/// Tests override it.
final integrationsApiFactoryProvider = Provider<IntegrationsApi Function()?>((
  ref,
) {
  final session = ref.watch(sessionCredentialsProvider);
  if (session == null) return null;
  final (baseUrl, token) = session;
  return () => IntegrationsApi(baseUrl: baseUrl, token: token);
});

/// Opens a URL in the system browser. Tests override it.
final openBrowserProvider = Provider<Future<bool> Function(Uri)>(
  (ref) =>
      (url) => launchUrl(url, mode: LaunchMode.externalApplication),
);

/// Whether this platform can sign in with OAuth. Tests override it.
final oauthSupportedProvider = Provider<bool>((ref) => oauthSupported);

/// Settings ▸ Integrations (spec §14, G-D21; decision 81h).
///
/// The owner connects a service **once, here**, and every agent session —
/// except a shell — can then use it, without the credential ever reaching a
/// Runtime Host or an agent. The server's 403 is what decides who sees this
/// list: the screen never infers a role (the 77d rule).
class IntegrationsScreen extends ConsumerStatefulWidget {
  const IntegrationsScreen({super.key});

  @override
  ConsumerState<IntegrationsScreen> createState() => _IntegrationsScreenState();
}

class _IntegrationsScreenState extends ConsumerState<IntegrationsScreen> {
  List<Integration>? _items;
  String? _error;
  bool _loading = false;

  IntegrationsApi? _api() => ref.read(integrationsApiFactoryProvider)?.call();

  late final OAuthLinks _links = ref.read(oauthLinksProvider);

  @override
  void initState() {
    super.initState();
    _load();
    // A sign-in redirect this process was not waiting for (spec §10.4,
    // 81l): finish it here. The app root brings this screen up for one.
    _links.orphans.addListener(_relayOrphans);
    WidgetsBinding.instance.addPostFrameCallback((_) => _relayOrphans());
  }

  @override
  void dispose() {
    _links.orphans.removeListener(_relayOrphans);
    super.dispose();
  }

  void _relayOrphans() {
    if (!mounted || _links.orphans.value.isEmpty) return;
    final orphans = _links.takeOrphans();
    _with((api) async {
      for (final params in orphans) {
        _reportCheck(await relayRedirect(api, params));
      }
    });
  }

  Future<void> _load() async {
    final api = _api();
    if (api == null) {
      setState(() => _error = 'Sign in first.');
      return;
    }
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final items = await api.list();
      if (mounted) setState(() => _items = items);
    } catch (e) {
      if (mounted) setState(() => _error = describeFailure(e));
    } finally {
      api.dispose();
      if (mounted) setState(() => _loading = false);
    }
  }

  void _say(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text(message)));
  }

  void _reportCheck(IntegrationCheck check) {
    if (check.ok) {
      final fresh = check.newTools.isEmpty
          ? ''
          : ' ${check.newTools.length} new tool(s) are off until you review them.';
      _say('Connected: ${check.toolCount} tool(s).$fresh');
    } else {
      _say(describeIntegrationError(check.errorCode));
    }
  }

  /// Runs `action` with an API, reporting failures and reloading after.
  Future<void> _with(Future<void> Function(IntegrationsApi api) action) async {
    final api = _api();
    if (api == null) return;
    setState(() => _loading = true);
    try {
      await action(api);
    } catch (e) {
      _say(switch (e) {
        StateError(:final message) => message,
        // This platform cannot do it (signing in from the browser): say so,
        // rather than blaming the network.
        UnsupportedError(:final message) => message ?? 'Not available here.',
        _ => describeFailure(e),
      });
    } finally {
      api.dispose();
      if (mounted) setState(() => _loading = false);
    }
    await _load();
  }

  Future<void> _add() async {
    final draft = await showDialog<_Draft>(
      context: context,
      builder: (_) =>
          _AddDialog(oauthSupported: ref.read(oauthSupportedProvider)),
    );
    if (draft == null || !mounted) return;
    await _with((api) async {
      final created = await api.create(
        displayName: draft.name,
        url: draft.url,
        authKind: draft.kind,
        header: draft.header,
        value: draft.value,
      );
      if (draft.kind == 'oauth') {
        _reportCheck(await _signIn(api, created.id));
      } else {
        _reportCheck(await api.test(created.id));
      }
    });
  }

  Future<IntegrationCheck> _signIn(IntegrationsApi api, String id) {
    _say('Finish signing in in your browser.');
    return signIn(api, id, openBrowser: ref.read(openBrowserProvider));
  }

  Future<void> _reconnect(Integration i) async {
    if (i.authKind == 'oauth' && !ref.read(oauthSupportedProvider)) {
      _say('Sign in again from a Storm app: the browser cannot sign in.');
      return;
    }
    if (i.authKind == 'oauth') {
      await _with((api) async => _reportCheck(await _signIn(api, i.id)));
      return;
    }
    final credential = await showDialog<(String, String)>(
      context: context,
      builder: (_) => _TokenDialog(integration: i),
    );
    if (credential == null || !mounted) return;
    final (header, value) = credential;
    await _with((api) async {
      await api.update(i.id, header: header, value: value);
      _reportCheck(await api.test(i.id));
    });
  }

  Future<void> _tools(Integration i) async {
    final api = _api();
    if (api == null) return;
    List<IntegrationTool> tools;
    try {
      tools = await api.tools(i.id);
    } catch (e) {
      _say(describeFailure(e));
      return;
    } finally {
      api.dispose();
    }
    if (!mounted) return;
    final chosen = await showDialog<List<String>>(
      context: context,
      builder: (_) => _ToolsDialog(integration: i, tools: tools),
    );
    if (chosen == null) return;
    await _with((api) => api.update(i.id, toolAllowlist: chosen));
  }

  Future<void> _disconnect(Integration i) async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Disconnect ${i.displayName}?'),
        content: Text(
          i.authKind == 'oauth'
              ? 'Storm deletes its sign-in and asks the service to revoke it. '
                    'Agents lose access at once.'
              : 'Storm deletes the token, and agents lose access at once. '
                    'Storm cannot revoke a token itself: revoke it at the '
                    'service too.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            key: const Key('confirm-disconnect'),
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Disconnect'),
          ),
        ],
      ),
    );
    if (ok != true) return;
    await _with((api) => api.delete(i.id));
  }

  @override
  Widget build(BuildContext context) {
    final items = _items;
    return SettingsPage(
      title: 'Integrations',
      intro: 'Services your agents can use, such as Linear or GitHub.',
      children: [
        const InfoBox(
          'Connect a service once, here. Every agent session except a shell '
          'can then use it. Anything an agent can read, it can send '
          'elsewhere.',
        ),
        SizedBox(height: context.tokens.sp * 0.5),
        if (_error != null)
          SettingsMuted(_error!, danger: true)
        else if (items == null)
          const SkeletonRows(rows: 3)
        else ...[
          for (final i in items)
            _IntegrationTile(
              integration: i,
              busy: _loading,
              canSignIn: ref.read(oauthSupportedProvider),
              onTest: () =>
                  _with((api) async => _reportCheck(await api.test(i.id))),
              onTools: () => _tools(i),
              onReconnect: () => _reconnect(i),
              onToggle: () =>
                  _with((api) => api.update(i.id, enabled: i.disabled)),
              onDisconnect: () => _disconnect(i),
            ),
          SettingsButtonRow(
            child: StormButton.primary(
              key: const Key('add-integration'),
              label: '＋ Add integration',
              onPressed: _loading ? null : _add,
            ),
          ),
        ],
      ],
    );
  }
}

class _IntegrationTile extends StatelessWidget {
  const _IntegrationTile({
    required this.integration,
    required this.busy,
    required this.canSignIn,
    required this.onTest,
    required this.onTools,
    required this.onReconnect,
    required this.onToggle,
    required this.onDisconnect,
  });

  final Integration integration;
  final bool busy;

  /// Whether this platform can run an OAuth sign-in (G-D13: not the web).
  final bool canSignIn;
  final VoidCallback onTest;
  final VoidCallback onTools;
  final VoidCallback onReconnect;
  final VoidCallback onToggle;
  final VoidCallback onDisconnect;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final i = integration;
    final attention = i.needsReconnect || i.status == 'error';
    final detail = i.builtin
        ? 'Built in. Read only unless a session is allowed to write.'
        : i.disabled
        ? 'Turned off'
        : attention && i.lastError != null
        ? describeIntegrationError(i.lastError)
        : [
            switch (i.authKind) {
              'oauth' => 'Signed in',
              'static' => 'Token',
              _ => 'No sign-in',
            },
            if (i.status == 'connected')
              '${i.toolAllowlist.length} '
                  '${i.toolAllowlist.length == 1 ? 'tool' : 'tools'} on',
          ].join(' · ');
    // An OAuth integration is reconnected by signing in, which the browser
    // cannot do (G-D13): it says where to do it instead of offering an action
    // that can only fail.
    final reconnectHere = i.authKind != 'oauth' || canSignIn;
    final reconnectLabel = switch (i.authKind) {
      'oauth' => 'Sign in again',
      'static' => 'Replace token',
      _ => 'Reconnect',
    };
    final (pill, pillColor) = i.builtin
        ? ('Built in', t.text3)
        : switch (i.status) {
            'connected' => ('Connected', t.text2),
            'needs_reauth' => ('Needs sign-in', t.danger),
            'pending_auth' => ('Not signed in', t.danger),
            'error' => ('Error', t.danger),
            _ => (i.statusLabel, t.text3),
          };

    // Beside the text at desk width; under it on a phone, where the name
    // and the pill together are wider than the row.
    final wide = context.isExpanded;
    final status = Wrap(
      spacing: t.sp * 1.5,
      runSpacing: t.sp * 0.5,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        StatusPill(key: Key('status-${i.id}'), label: pill, color: pillColor),
        if (!i.builtin && attention && reconnectHere)
          TextAction(
            key: Key('reconnect-${i.id}'),
            label: reconnectLabel,
            onTap: busy ? null : onReconnect,
          )
        else if (!i.builtin && i.status == 'connected')
          TextAction(
            key: Key('tools-${i.id}'),
            label: 'Choose tools',
            onTap: busy ? null : onTools,
          ),
      ],
    );

    return Container(
      key: Key('integration-${i.id}'),
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
                  i.displayName,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.uiSize,
                    color: t.text,
                  ),
                ),
                SizedBox(height: t.sp * 0.25),
                Text(
                  detail,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.codeSize,
                    color: t.text3,
                    height: 1.45,
                  ),
                ),
                if (!i.builtin && attention && !reconnectHere)
                  Padding(
                    padding: EdgeInsets.only(top: t.sp * 0.5),
                    child: Text(
                      'Reconnect from a Storm app',
                      key: Key('reconnect-elsewhere-${i.id}'),
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.codeSize,
                        color: t.text2,
                      ),
                    ),
                  ),
                // Spec §9: the owner is told, and nothing is turned on.
                if (!i.builtin && !i.disabled && i.newTools.isNotEmpty)
                  Padding(
                    padding: EdgeInsets.only(top: t.sp * 0.5),
                    child: TextAction(
                      key: Key('review-tools-${i.id}'),
                      label:
                          '${i.newTools.length} new '
                          '${i.newTools.length == 1 ? 'tool' : 'tools'} — review',
                      accent: true,
                      onTap: busy ? null : onTools,
                    ),
                  ),
                if (!wide) ...[SizedBox(height: t.sp), status],
              ],
            ),
          ),
          if (wide) ...[SizedBox(width: t.sp * 1.5), status],
          if (!i.builtin)
            PopupMenuButton<String>(
              key: Key('menu-${i.id}'),
              enabled: !busy,
              tooltip: 'More for ${i.displayName}',
              icon: Icon(LucideIcons.ellipsis, size: t.uiSize, color: t.text3),
              onSelected: (v) {
                switch (v) {
                  case 'test':
                    onTest();
                  case 'tools':
                    onTools();
                  case 'reconnect':
                    onReconnect();
                  case 'toggle':
                    onToggle();
                  default:
                    onDisconnect();
                }
              },
              itemBuilder: (_) => [
                if (!i.disabled)
                  const PopupMenuItem(value: 'test', child: Text('Test')),
                if (!i.disabled)
                  const PopupMenuItem(
                    value: 'tools',
                    child: Text('Choose tools'),
                  ),
                PopupMenuItem(
                  value: 'reconnect',
                  enabled: reconnectHere,
                  child: Text(
                    i.authKind != 'oauth'
                        ? 'Replace token'
                        : reconnectHere
                        ? 'Sign in again'
                        : 'Sign in again from a Storm app',
                  ),
                ),
                PopupMenuItem(
                  value: 'toggle',
                  child: Text(i.disabled ? 'Enable' : 'Disable'),
                ),
                const PopupMenuItem(
                  value: 'disconnect',
                  child: Text('Disconnect'),
                ),
              ],
            ),
        ],
      ),
    );
  }
}

/// What the add dialog collected.
class _Draft {
  _Draft({
    required this.name,
    required this.url,
    required this.kind,
    this.header,
    this.value,
  });

  final String name;
  final String url;
  final String kind;
  final String? header;
  final String? value;
}

/// A token as the header carries it: `Bearer <token>` for `Authorization`
/// unless the owner already wrote a scheme.
(String, String) _credential(String header, String token) {
  final h = header.trim().isEmpty ? 'Authorization' : header.trim();
  final v = token.trim();
  final value = h.toLowerCase() == 'authorization' && !v.contains(' ')
      ? 'Bearer $v'
      : v;
  return (h, value);
}

class _AddDialog extends StatefulWidget {
  const _AddDialog({required this.oauthSupported});

  final bool oauthSupported;

  @override
  State<_AddDialog> createState() => _AddDialogState();
}

class _AddDialogState extends State<_AddDialog> {
  final _name = TextEditingController();
  final _url = TextEditingController();
  final _header = TextEditingController(text: 'Authorization');
  final _token = TextEditingController();
  late String _kind = widget.oauthSupported ? 'oauth' : 'static';
  String? _problem;

  @override
  void dispose() {
    for (final c in [_name, _url, _header, _token]) {
      c.dispose();
    }
    super.dispose();
  }

  void _submit() {
    final name = _name.text.trim();
    final url = _url.text.trim();
    if (name.isEmpty) return setState(() => _problem = 'Give it a name.');
    if (!url.startsWith('https://')) {
      return setState(() => _problem = 'The address must start with https://');
    }
    if (_kind == 'static' && _token.text.trim().isEmpty) {
      return setState(() => _problem = 'Paste the token.');
    }
    final (header, value) = _kind == 'static'
        ? _credential(_header.text, _token.text)
        : (null, null);
    Navigator.of(context).pop(
      _Draft(name: name, url: url, kind: _kind, header: header, value: value),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return AlertDialog(
      title: const Text('Add integration'),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            TextField(
              key: const Key('integration-name'),
              controller: _name,
              autofocus: true,
              decoration: const InputDecoration(
                labelText: 'Name',
                hintText: 'Notion, Linear, GitHub (work)',
              ),
            ),
            SizedBox(height: t.sp),
            TextField(
              key: const Key('integration-url'),
              controller: _url,
              keyboardType: TextInputType.url,
              decoration: const InputDecoration(
                labelText: 'MCP server address',
                hintText: 'https://mcp.notion.com/mcp',
              ),
            ),
            SizedBox(height: t.sp * 2),
            // One section: what the choice is, the choice, and what the
            // chosen option means — so it reads as part of connecting to the
            // address above, not as a stray control.
            Text(
              'How Storm connects',
              style: TextStyle(
                fontSize: t.labelSize,
                fontWeight: FontWeight.w600,
              ),
            ),
            SizedBox(height: t.sp),
            // As wide as the fields above it, so the section reads as one.
            SizedBox(
              width: double.infinity,
              child: SegmentedButton<String>(
                key: const Key('integration-auth'),
                segments: [
                  // Shown everywhere, so the web's choices are visibly the same
                  // set with one unavailable (G-D13), not a different feature.
                  // Unavailable must not look selectable: greyed by the disabled
                  // state, a lock where the others have none, and a tooltip.
                  ButtonSegment(
                    value: 'oauth',
                    label: const Text('Sign in', maxLines: 1, softWrap: false),
                    enabled: widget.oauthSupported,
                    icon: widget.oauthSupported
                        ? null
                        : const Icon(LucideIcons.lock, size: 14),
                    tooltip: widget.oauthSupported
                        ? null
                        : 'Available in the Storm apps',
                  ),
                  const ButtonSegment(
                    value: 'static',
                    label: Text('Token', maxLines: 1, softWrap: false),
                  ),
                  const ButtonSegment(
                    value: 'none',
                    label: Text('No sign-in', maxLines: 1, softWrap: false),
                  ),
                ],
                selected: {_kind},
                onSelectionChanged: (s) => setState(() => _kind = s.first),
                // Three choices across a phone-width dialog: no check mark (the
                // selected one is filled already) and compact labels, so no
                // label ever wraps mid-word.
                showSelectedIcon: false,
                style: ButtonStyle(
                  visualDensity: VisualDensity.compact,
                  // The family is set, not inherited: a style here replaces the
                  // theme's label style whole (the StatusChip rule).
                  textStyle: WidgetStatePropertyAll(
                    TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.labelSize,
                    ),
                  ),
                ),
              ),
            ),
            SizedBox(height: t.sp * 0.75),
            Text(
              switch (_kind) {
                'oauth' =>
                  'Your browser opens to sign in; Storm keeps the sign-in.',
                'static' =>
                  'An API token or personal access token, sent as a header.',
                _ => 'For public servers that need no credential.',
              },
              key: const Key('integration-auth-help'),
              style: TextStyle(fontSize: t.labelSize, color: t.text3),
            ),
            if (!widget.oauthSupported) ...[
              SizedBox(height: t.sp * 0.5),
              Text(
                'Sign-in is available in the Storm apps (Android, macOS, '
                'Linux, Windows). In the browser, use a token, or add this '
                'integration from an app.',
                key: const Key('integration-web-note'),
                style: TextStyle(fontSize: t.labelSize, color: t.text3),
              ),
            ],
            if (_kind == 'static') ...[
              SizedBox(height: t.sp),
              TextField(
                key: const Key('integration-token'),
                controller: _token,
                obscureText: true,
                decoration: const InputDecoration(labelText: 'Token'),
              ),
              SizedBox(height: t.sp),
              TextField(
                controller: _header,
                decoration: const InputDecoration(labelText: 'Header'),
              ),
              SizedBox(height: t.sp),
              Text(
                'For GitHub, use a fine-grained personal access token with a '
                'short expiry. Storm keeps it encrypted and sends it only to '
                'this address, but it cannot narrow or revoke it: that is done '
                'at GitHub.',
                style: TextStyle(fontSize: t.labelSize, color: t.text3),
              ),
            ],
            if (_problem != null) ...[
              SizedBox(height: t.sp),
              Text(_problem!, style: TextStyle(color: t.danger)),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('integration-connect'),
          onPressed: _submit,
          child: const Text('Connect'),
        ),
      ],
    );
  }
}

class _TokenDialog extends StatefulWidget {
  const _TokenDialog({required this.integration});

  final Integration integration;

  @override
  State<_TokenDialog> createState() => _TokenDialogState();
}

class _TokenDialogState extends State<_TokenDialog> {
  final _token = TextEditingController();
  final _header = TextEditingController(text: 'Authorization');

  @override
  void dispose() {
    _token.dispose();
    _header.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text('Replace the token for ${widget.integration.displayName}'),
    content: Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        TextField(
          key: const Key('replace-token'),
          controller: _token,
          obscureText: true,
          autofocus: true,
          decoration: const InputDecoration(labelText: 'New token'),
        ),
        TextField(
          controller: _header,
          decoration: const InputDecoration(labelText: 'Header'),
        ),
      ],
    ),
    actions: [
      TextButton(
        onPressed: () => Navigator.of(context).pop(),
        child: const Text('Cancel'),
      ),
      FilledButton(
        onPressed: () {
          if (_token.text.trim().isEmpty) return;
          Navigator.of(context).pop(_credential(_header.text, _token.text));
        },
        child: const Text('Save'),
      ),
    ],
  );
}

class _ToolsDialog extends StatefulWidget {
  const _ToolsDialog({required this.integration, required this.tools});

  final Integration integration;
  final List<IntegrationTool> tools;

  @override
  State<_ToolsDialog> createState() => _ToolsDialogState();
}

class _ToolsDialogState extends State<_ToolsDialog> {
  late final Set<String> _on = {
    for (final tool in widget.tools)
      if (tool.allowed) tool.name,
  };

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return AlertDialog(
      title: Text('Tools of ${widget.integration.displayName}'),
      content: SizedBox(
        width: 420,
        child: ListView(
          shrinkWrap: true,
          children: [
            Text(
              'Agents can call only the tools that are on. A tool the service '
              'adds later starts off.',
              style: TextStyle(fontSize: t.labelSize, color: t.text3),
            ),
            for (final tool in widget.tools)
              CheckboxListTile(
                key: Key('tool-${tool.name}'),
                value: _on.contains(tool.name),
                onChanged: (v) => setState(
                  () => v == true ? _on.add(tool.name) : _on.remove(tool.name),
                ),
                title: Text(
                  tool.isNew
                      ? '${displayToolName(tool.name)}  (new)'
                      : displayToolName(tool.name),
                ),
                subtitle: tool.description == null
                    ? null
                    : Text(
                        displayToolName(tool.description!, max: 200),
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                      ),
              ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('save-tools'),
          onPressed: () => Navigator.of(context).pop(_on.toList()..sort()),
          child: const Text('Save'),
        ),
      ],
    );
  }
}

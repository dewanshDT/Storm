import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:url_launcher/url_launcher.dart';

import '../router.dart';
import '../ui/states.dart';
import '../ui/tokens.dart';
import 'agent_state.dart' show sessionCredentialsProvider;
import 'hosts_screen.dart' show ChipTone, StatusChip;
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
      _say(e is StateError ? e.message : describeFailure(e));
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
    final t = context.tokens;
    final items = _items;
    return Scaffold(
      appBar: AppBar(
        leading: IconButton(
          icon: const Icon(LucideIcons.arrow_left),
          onPressed: () => context.canPop()
              ? context.pop()
              : context.go(Routes.serverSettings),
        ),
        title: const Text('Integrations'),
      ),
      floatingActionButton: items == null
          ? null
          : FloatingActionButton.extended(
              key: const Key('add-integration'),
              onPressed: _loading ? null : _add,
              icon: const Icon(LucideIcons.plus),
              label: const Text('Add integration'),
            ),
      body: RefreshIndicator(
        onRefresh: _load,
        child: ListView(
          padding: EdgeInsets.fromLTRB(t.sp * 2, t.sp, t.sp * 2, t.sp * 12),
          children: [
            Text(
              'Connect a service once, here. Every agent session except a '
              'shell can then use it, through Storm: its credential stays on '
              'this server and never reaches a host or an agent. Sessions '
              "have their host's network access, so anything an agent can "
              'read, it can send elsewhere.',
              style: TextStyle(fontSize: t.labelSize, color: t.text3),
            ),
            SizedBox(height: t.sp * 2),
            if (_error != null)
              Text(_error!, style: TextStyle(color: t.danger))
            else if (items == null)
              const SkeletonRows(rows: 3)
            else
              for (final i in items)
                _IntegrationTile(
                  integration: i,
                  busy: _loading,
                  onTest: () =>
                      _with((api) async => _reportCheck(await api.test(i.id))),
                  onTools: () => _tools(i),
                  onReconnect: () => _reconnect(i),
                  onToggle: () =>
                      _with((api) => api.update(i.id, enabled: i.disabled)),
                  onDisconnect: () => _disconnect(i),
                ),
          ],
        ),
      ),
    );
  }
}

class _IntegrationTile extends StatelessWidget {
  const _IntegrationTile({
    required this.integration,
    required this.busy,
    required this.onTest,
    required this.onTools,
    required this.onReconnect,
    required this.onToggle,
    required this.onDisconnect,
  });

  final Integration integration;
  final bool busy;
  final VoidCallback onTest;
  final VoidCallback onTools;
  final VoidCallback onReconnect;
  final VoidCallback onToggle;
  final VoidCallback onDisconnect;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final i = integration;
    final detail = i.builtin
        ? (i.vaultWritesAvailable ?? false)
              ? 'Your vaults. Agents read them; they write only when a '
                    'session allows it.'
              : 'Your vaults, read only. Turn on MCP writes to let a session '
                    'allow writing.'
        : [
            i.slug,
            switch (i.authKind) {
              'oauth' => 'signed in',
              'static' => 'token',
              _ => 'no auth',
            },
            if (i.lastError != null && !i.disabled)
              describeIntegrationError(i.lastError),
          ].join(' · ');
    final attention = i.needsReconnect || i.status == 'error';
    return Container(
      key: Key('integration-${i.id}'),
      margin: EdgeInsets.only(bottom: t.sp),
      padding: EdgeInsets.all(t.sp * 1.5),
      decoration: BoxDecoration(
        color: t.surface,
        border: Border.all(color: t.border, width: t.bw),
        borderRadius: BorderRadius.circular(t.rCard),
      ),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Flexible(
                      child: Text(
                        i.displayName,
                        style: const TextStyle(fontWeight: FontWeight.w600),
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                    SizedBox(width: t.sp),
                    StatusChip(
                      key: Key('status-${i.id}'),
                      label: i.builtin ? 'Built in' : i.statusLabel,
                      tone: attention ? ChipTone.warn : ChipTone.muted,
                    ),
                  ],
                ),
                SizedBox(height: t.sp * 0.25),
                Text(
                  detail,
                  style: TextStyle(fontSize: t.labelSize, color: t.text3),
                ),
                // Spec §9: the owner is told, and nothing is turned on.
                if (!i.builtin && !i.disabled && i.newTools.isNotEmpty)
                  Align(
                    alignment: Alignment.centerLeft,
                    child: TextButton(
                      key: Key('review-tools-${i.id}'),
                      onPressed: busy ? null : onTools,
                      child: Text(
                        '${i.newTools.length} new '
                        '${i.newTools.length == 1 ? 'tool' : 'tools'} — review',
                      ),
                    ),
                  ),
              ],
            ),
          ),
          if (!i.builtin && attention)
            TextButton(
              key: Key('reconnect-${i.id}'),
              onPressed: busy ? null : onReconnect,
              child: const Text('Reconnect'),
            ),
          if (!i.builtin)
            PopupMenuButton<String>(
              key: Key('menu-${i.id}'),
              enabled: !busy,
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
                  child: Text(
                    i.authKind == 'oauth' ? 'Sign in again' : 'Replace token',
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
            SegmentedButton<String>(
              segments: [
                if (widget.oauthSupported)
                  const ButtonSegment(value: 'oauth', label: Text('Sign in')),
                const ButtonSegment(value: 'static', label: Text('Token')),
                const ButtonSegment(value: 'none', label: Text('None')),
              ],
              selected: {_kind},
              onSelectionChanged: (s) => setState(() => _kind = s.first),
            ),
            SizedBox(height: t.sp),
            if (_kind == 'oauth')
              Text(
                'Your browser opens to sign in; Storm keeps the sign-in.',
                style: TextStyle(fontSize: t.labelSize, color: t.text3),
              ),
            if (_kind == 'static') ...[
              TextField(
                key: const Key('integration-token'),
                controller: _token,
                obscureText: true,
                decoration: const InputDecoration(labelText: 'Token'),
              ),
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
            if (!widget.oauthSupported) ...[
              SizedBox(height: t.sp),
              Text(
                'Signing in is available in the desktop and mobile apps. In '
                'the browser, connect with a token.',
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

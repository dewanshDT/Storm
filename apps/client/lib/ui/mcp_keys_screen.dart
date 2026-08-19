import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../api/storm_api.dart';
import '../state/app_state.dart';
import 'tokens.dart';

/// Manage the MCP keys this account holds (A14).
///
/// A key is a credential for a *machine* — an MCP client on a laptop, a
/// script, an agent — and it acts as you. Minting one is session tier, so
/// being on this screen at all is the proof of who is asking.
///
/// **The secret is shown once.** Not because it is hard to show again, but
/// because the server does not have it: it stores a blake3 hash and nothing
/// else. That is the same reason this screen never writes the secret to prefs
/// or the keychain — it is the other machine's credential, not this device's,
/// and it lives in memory only while the reveal sheet is open.
class McpKeysScreen extends ConsumerStatefulWidget {
  const McpKeysScreen({super.key});

  @override
  ConsumerState<McpKeysScreen> createState() => _McpKeysScreenState();
}

class _McpKeysScreenState extends ConsumerState<McpKeysScreen> {
  List<McpKey>? _keys;
  String? _error;
  bool _loading = false;

  @override
  void initState() {
    super.initState();
    _load();
  }

  StormApi? _api() {
    final settings = ref.read(settingsProvider).value;
    if (settings == null || !settings.hasSession) return null;
    return StormApi(baseUrl: settings.baseUrl, token: settings.accessToken);
  }

  Future<void> _load() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    final api = _api();
    if (api == null) {
      setState(() {
        _error = 'Sign in first — keys belong to an account.';
        _loading = false;
      });
      return;
    }
    try {
      final keys = await api.mcpKeys();
      if (!mounted) return;
      setState(() {
        _keys = keys;
        _loading = false;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _error = '$e';
        _loading = false;
      });
    } finally {
      api.dispose();
    }
  }

  Future<void> _create() async {
    final name = await showDialog<String>(
      context: context,
      builder: (_) => const _NameDialog(),
    );
    if (name == null || name.trim().isEmpty || !mounted) return;

    final api = _api();
    if (api == null) return;
    try {
      final created = await api.createMcpKey(name: name.trim());
      if (!mounted) return;
      // Straight into the reveal, with no await in between that could drop it:
      // this value cannot be fetched again.
      await showDialog<void>(
        context: context,
        barrierDismissible: false,
        builder: (_) => RevealMcpKeyDialog(created: created),
      );
      if (!mounted) return;
      await _load();
    } catch (e) {
      if (!mounted) return;
      setState(() => _error = '$e');
    } finally {
      api.dispose();
    }
  }

  Future<void> _revoke(McpKey key) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Revoke “${key.name}”?'),
        content: const Text(
          'Whatever is using this key stops working on its next request. '
          'This cannot be undone — mint a new key instead.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Revoke'),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;

    final api = _api();
    if (api == null) return;
    try {
      await api.revokeMcpKey(key.id);
      if (!mounted) return;
      await _load();
    } catch (e) {
      if (!mounted) return;
      setState(() => _error = '$e');
    } finally {
      api.dispose();
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final keys = _keys;

    return Scaffold(
      appBar: AppBar(
        title: const Text('MCP keys'),
        actions: [
          IconButton(
            tooltip: 'New key',
            onPressed: _loading ? null : _create,
            icon: const Icon(Icons.add),
          ),
        ],
      ),
      body: RefreshIndicator(
        onRefresh: _load,
        child: ListView(
          padding: EdgeInsets.all(t.sp * 2),
          children: [
            Text(
              'A key lets a program — an MCP client, a script, an agent — reach '
              'this server as you. Each one is separate, so you can revoke a '
              'single machine without touching anything else.',
              style: TextStyle(fontSize: t.labelSize, color: t.text3),
            ),
            SizedBox(height: t.sp * 2),
            if (_error != null) ...[
              Text(_error!, style: TextStyle(color: t.danger)),
              SizedBox(height: t.sp * 2),
            ],
            if (_loading && keys == null)
              const Center(child: CircularProgressIndicator())
            else if (keys != null && keys.isEmpty)
              Text(
                'No keys yet.',
                style: TextStyle(fontSize: t.labelSize, color: t.text3),
              )
            else
              for (final key in keys ?? <McpKey>[])
                _KeyTile(apiKey: key, onRevoke: () => _revoke(key)),
          ],
        ),
      ),
      floatingActionButton: FloatingActionButton.extended(
        onPressed: _loading ? null : _create,
        icon: const Icon(Icons.key),
        label: const Text('New key'),
      ),
    );
  }
}

class _KeyTile extends StatelessWidget {
  const _KeyTile({required this.apiKey, required this.onRevoke});

  /// Named `apiKey` rather than `key`, which `Widget` already owns.
  final McpKey apiKey;
  final VoidCallback onRevoke;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final revoked = apiKey.isRevoked;

    // "Never used" is worth saying out loud: it is the difference between a
    // key that was set up wrong and one that simply is not busy.
    final used = apiKey.lastUsed == null
        ? 'never used'
        : 'last used ${_day(apiKey.lastUsed!)}';
    final expiry = apiKey.expires == null
        ? null
        : 'expires ${_day(apiKey.expires!)}';

    return Opacity(
      opacity: revoked ? 0.55 : 1,
      child: Container(
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
                  Text(
                    apiKey.name,
                    style: TextStyle(
                      fontWeight: FontWeight.w600,
                      decoration: revoked ? TextDecoration.lineThrough : null,
                    ),
                  ),
                  SizedBox(height: t.sp * 0.25),
                  Text(
                    revoked ? 'revoked' : [used, ?expiry].join(' · '),
                    style: TextStyle(fontSize: t.labelSize, color: t.text3),
                  ),
                ],
              ),
            ),
            if (!revoked)
              TextButton(onPressed: onRevoke, child: const Text('Revoke')),
          ],
        ),
      ),
    );
  }

  /// Just the date. A timestamp to the second is noise on a list you scan.
  static String _day(String iso) =>
      DateTime.tryParse(iso)?.toLocal().toString().split(' ').first ?? iso;
}

class _NameDialog extends StatefulWidget {
  const _NameDialog();

  @override
  State<_NameDialog> createState() => _NameDialogState();
}

class _NameDialogState extends State<_NameDialog> {
  final _controller = TextEditingController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('New MCP key'),
    content: Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const Text(
          'Name it after the machine that will hold it — that name is how you '
          'will know what you are revoking later.',
        ),
        const SizedBox(height: 12),
        TextField(
          controller: _controller,
          autofocus: true,
          decoration: const InputDecoration(
            labelText: 'Name',
            hintText: 'Claude Code, work laptop',
          ),
          onSubmitted: (v) => Navigator.of(context).pop(v),
        ),
      ],
    ),
    actions: [
      TextButton(
        onPressed: () => Navigator.of(context).pop(),
        child: const Text('Cancel'),
      ),
      FilledButton(
        onPressed: () => Navigator.of(context).pop(_controller.text),
        child: const Text('Create'),
      ),
    ],
  );
}

/// The one-time reveal.
///
/// **Deliberately hard to dismiss by accident**: no barrier dismiss, and the
/// only way out is a button labelled with what it means. The server keeps a
/// hash, so "show it again" is not a feature anyone can build — losing this
/// value means revoking and minting another.
class RevealMcpKeyDialog extends ConsumerWidget {
  const RevealMcpKeyDialog({super.key, required this.created});

  final CreatedMcpKey created;

  /// A ready-to-paste MCP client entry.
  ///
  /// Assembling this by hand is where keys get mangled — the same way pairing
  /// URIs did when people retyped them — so the screen does it.
  static String configSnippet(String baseUrl, String secret) =>
      '{\n'
      '  "mcpServers": {\n'
      '    "storm": {\n'
      '      "type": "http",\n'
      '      "url": "$baseUrl/mcp",\n'
      '      "headers": {\n'
      '        "Authorization": "Bearer $secret"\n'
      '      }\n'
      '    }\n'
      '  }\n'
      '}';

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final baseUrl = ref.read(settingsProvider).value?.baseUrl ?? '';
    final snippet = configSnippet(baseUrl, created.secret);

    return AlertDialog(
      title: const Text('Copy it now'),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              'This is the only time this key is shown. Storm keeps only a '
              'hash of it, so it cannot be shown again — if you lose it, '
              'revoke this key and make another.',
              style: TextStyle(color: t.amber),
            ),
            SizedBox(height: t.sp * 1.5),
            _Copyable(label: 'Key', value: created.secret),
            SizedBox(height: t.sp * 1.5),
            Text(
              'Or paste this straight into your MCP client config:',
              style: TextStyle(fontSize: t.labelSize, color: t.text3),
            ),
            SizedBox(height: t.sp * 0.5),
            _Copyable(label: 'Config', value: snippet),
          ],
        ),
      ),
      actions: [
        FilledButton(
          key: const Key('reveal-done'),
          onPressed: () => Navigator.of(context).pop(),
          child: const Text("I've copied it"),
        ),
      ],
    );
  }
}

class _Copyable extends StatelessWidget {
  const _Copyable({required this.label, required this.value});

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Container(
          width: double.infinity,
          padding: EdgeInsets.all(t.sp),
          decoration: BoxDecoration(
            color: t.codePlate,
            borderRadius: BorderRadius.circular(t.rControl),
          ),
          child: SelectableText(
            value,
            style: TextStyle(
              fontFamily: StormTokens.monoFamily,
              fontSize: t.codeSize,
              color: t.onCodePlate,
            ),
          ),
        ),
        Align(
          alignment: Alignment.centerRight,
          child: TextButton.icon(
            key: Key('copy-${label.toLowerCase()}'),
            onPressed: () async {
              await Clipboard.setData(ClipboardData(text: value));
              if (context.mounted) {
                ScaffoldMessenger.of(
                  context,
                ).showSnackBar(SnackBar(content: Text('$label copied')));
              }
            },
            icon: const Icon(Icons.copy, size: 16),
            label: Text('Copy $label'),
          ),
        ),
      ],
    );
  }
}

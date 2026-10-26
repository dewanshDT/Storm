import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../state/app_state.dart';
import '../state/health.dart' show integrationsSummaryProvider;
import '../ui/controls.dart';
import '../ui/states.dart';
import '../ui/surfaces.dart';
import '../ui/tokens.dart';
import '../ui/widgets.dart' show PopoverItem;
import 'agent_api.dart' show AgentRefusal;
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart';
import 'claude_login_api.dart';
import 'crew_widgets.dart' show RoleMenu, popoverRoom;
import 'integrations_api.dart' show integrationNamesOn;
import 'spawn_models.dart' show kSpawnProviders;

typedef NoteRef = ({String vaultId, String noteId});

/// What the launcher starts from: nothing, a note, an agent card, or an
/// ended session to run again (handoff §3.5, §3.7).
class LaunchPrefill {
  const LaunchPrefill({
    this.hostId,
    this.workspace,
    this.provider,
    this.context,
    this.contextTitle,
    this.writes,
    this.writeVaultId,
    this.crewId,
  });

  /// Everything the session was launched with. Its host may be offline by
  /// now; the launcher then falls back to the first online one.
  factory LaunchPrefill.runAgain(AgentSession s) => LaunchPrefill(
    hostId: s.hostId,
    workspace: s.workspace,
    provider: s.provider,
    context: s.context == null
        ? null
        : (vaultId: s.context!.vaultId, noteId: s.context!.noteId),
    contextTitle: s.context?.title,
    writes: s.writeVaultId != null,
    writeVaultId: s.writeVaultId,
  );

  final String? hostId;
  final String? workspace;
  final String? provider;
  final NoteRef? context;
  final String? contextTitle;

  /// Null: on when Settings allows agent writes (handoff §4.5).
  final bool? writes;
  final String? writeVaultId;

  /// The crew to join: preset when opened from a crew page (138h).
  final String? crewId;
}

/// The New session launcher: a centred modal at desk width, a bottom sheet
/// on a phone. Pops the launched session, or nothing.
class NewSessionLauncher extends ConsumerStatefulWidget {
  const NewSessionLauncher({
    super.key,
    required this.hosts,
    this.prefill = const LaunchPrefill(),
  });

  final List<AgentHost> hosts;
  final LaunchPrefill prefill;

  @override
  ConsumerState<NewSessionLauncher> createState() => _LauncherState();
}

class _LauncherState extends ConsumerState<NewSessionLauncher> {
  AgentHost? _host;
  List<AgentWorkspace>? _workspaces;
  String? _workspace;
  String? _provider;

  /// The agent asked for — by Run again, a card or a pick — over the
  /// default, wherever the host offers it.
  String? _chosen;
  String? _default;
  NoteRef? _context;
  bool? _writes;
  String? _writeVault;

  /// [_noCrew], [_newCrew] or a crew's id (138h).
  late String _crew = widget.prefill.crewId ?? _noCrew;
  RoleNote? _role;

  /// "Allow this session to start agents" (141e): off by default.
  bool _spawn = false;
  String? _error;
  bool _busy = false;

  List<AgentHost> get _online => [
    for (final h in widget.hosts)
      if (h.online) h,
  ];

  @override
  void initState() {
    super.initState();
    final p = widget.prefill;
    _context = p.context;
    _writes = p.writes;
    _writeVault = p.writeVaultId ?? p.context?.vaultId;
    _chosen = p.provider;
    _preselectHost();
    _loadDefault();
  }

  /// The session's own host when running one again, else the one this
  /// device launched on last, else the first online (decision 78).
  Future<void> _preselectHost() async {
    final online = _online;
    if (online.isEmpty) return;
    final wanted = widget.prefill.hostId ?? await lastLaunchHost();
    if (!mounted || _host != null) return;
    await _pickHost(
      online.where((h) => h.id == wanted).firstOrNull ?? online.first,
      keepWorkspace: widget.prefill.workspace,
    );
  }

  Future<void> _loadDefault() async {
    final api = agentApi(ref);
    if (api == null) return;
    try {
      final d = await api.defaultProvider();
      if (!mounted) return;
      setState(() {
        _default = d;
        _settleProvider();
      });
    } catch (_) {
    } finally {
      api.dispose();
    }
  }

  /// Keeps the chosen agent if this host offers it, else the default, else
  /// the first one installed.
  void _settleProvider() {
    final host = _host;
    if (host == null) return;
    final installed = [
      for (final p in host.providers)
        if (p.available) p.id,
    ];
    _provider = installed.contains(_chosen)
        ? _chosen
        : installed.contains(_default)
        ? _default
        : installed.firstOrNull;
  }

  Future<void> _pickHost(AgentHost host, {String? keepWorkspace}) async {
    setState(() {
      _host = host;
      _workspaces = null;
      _workspace = null;
      _settleProvider();
    });
    final api = agentApi(ref);
    if (api == null) return;
    try {
      final ws = await api.workspaces(host.id);
      if (!mounted || _host?.id != host.id) return;
      setState(() {
        _workspaces = ws;
        _workspace =
            ws.where((w) => w.name == keepWorkspace).firstOrNull?.name ??
            ws.firstOrNull?.name;
      });
    } catch (e) {
      if (mounted) setState(() => _error = launchErrorText(e));
    } finally {
      api.dispose();
    }
  }

  /// "New workspace…" (decision 141c): made on the host, then picked.
  Future<void> _newWorkspace(AgentHost host) async {
    final name = await showDialog<String>(
      context: context,
      builder: (_) => NewWorkspaceDialog(host: host),
    );
    if (name == null || !mounted || _host?.id != host.id) return;
    setState(() => _error = null);
    await _pickHost(host, keepWorkspace: name);
  }

  Future<void> _launch(
    bool writes,
    String? writeVault,
    LaunchCrew? crew,
  ) async {
    final host = _host, ws = _workspace;
    if (host == null || ws == null) return;
    final api = agentApi(ref);
    if (api == null) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final size = MediaQuery.sizeOf(context);
      final session = await api.launch(
        hostId: host.id,
        workspace: ws,
        provider: _provider,
        // A first guess; the terminal sends its real size once laid out.
        cols: (size.width / 9).clamp(40, 240).floor(),
        rows: (size.height / 20).clamp(12, 80).floor(),
        context: _context,
        writeVaultId: writes && _reachesStorm ? writeVault : null,
        crew: crew,
        // An agent that can't read Storm notes is never sent one (138g).
        role: _role == null || !_reachesStorm
            ? null
            : (vaultId: _role!.vaultId, noteId: _role!.noteId),
        spawn: _spawn && _spawnable,
      );
      await rememberLaunchHost(host.id);
      if (!mounted) return;
      final notice = session.launchNotice;
      if (notice != null) {
        ScaffoldMessenger.maybeOf(
          context,
        )?.showSnackBar(SnackBar(content: Text(notice)));
      }
      Navigator.of(context).pop(session);
    } catch (e) {
      if (mounted) setState(() => _error = launchErrorText(e));
    } finally {
      api.dispose();
      if (mounted) setState(() => _busy = false);
    }
  }

  /// The chosen agent's sessions can read your vaults and use your
  /// integrations (decision 130a). An older host says nothing, so only
  /// `shell` is left out.
  bool get _reachesStorm =>
      _host?.providers
          .where((p) => p.id == _provider)
          .firstOrNull
          ?.reachesStorm ??
      _provider != 'shell';

  /// The launch can carry the switch: the host passes C0 and the agent can
  /// start agents with Storm's tools (141e). The server decides.
  bool get _spawnable =>
      (_host?.offersSpawning ?? false) &&
      kSpawnProviders.contains(_provider) &&
      _reachesStorm;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final host = _host;
    final shell = !_reachesStorm;
    final allowed = ref.watch(agentWritesSettingProvider).value == true;
    final vaults = [
      for (final v in ref.watch(vaultsProvider).value ?? const <VaultInfo>[])
        if (!v.missing) v,
    ];
    final writeVault =
        vaults.where((v) => v.id == _writeVault).firstOrNull?.id ??
        vaults.firstOrNull?.id;
    final writes = allowed && (_writes ?? true) && writeVault != null;
    final integrations = integrationNamesOn(
      ref.watch(integrationsSummaryProvider).value,
      host?.id,
    );
    final hostName = host?.name ?? 'the host';
    final claudeLogin = ref.watch(claudeLoginSummaryProvider).value;
    final ctx = _context;
    final crews = ref.watch(agentOverviewProvider).value?.crews;
    final roles = ref.watch(rolesProvider).value ?? const <RoleNote>[];
    final ctxNote = ctx == null
        ? null
        : ref.watch(agentNoteProvider(ctx)).value;
    final ctxTitle = ctx == null
        ? null
        : widget.prefill.context == ctx && widget.prefill.contextTitle != null
        ? widget.prefill.contextTitle
        : ctxNote == null
        ? null
        : noteTitleOf(ctxNote.meta.title, ctxNote.meta.path);
    final crewOptions = [
      const _Opt(_noCrew, 'None'),
      if (ctxTitle != null && ctxTitle.isNotEmpty)
        _Opt(_newCrew, 'New crew: $ctxTitle'),
      for (final c in crews ?? const <Crew>[]) _Opt(c.id, c.name),
    ];
    // A preset crew that has since gone, or a new crew without its note.
    final crewKey = crewOptions.any((o) => o.value == _crew) ? _crew : _noCrew;
    final crew = switch (crewKey) {
      _noCrew => null,
      _newCrew => LaunchCrew.create(ctxTitle!),
      final id => LaunchCrew.join(id),
    };
    final roleKey = _role == null ? _noRole : _roleKey(_role!);

    final children = <Widget>[
      Row(
        children: [
          Expanded(
            child: Text(
              'New session',
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.headingSize * 0.9,
                fontWeight: FontWeight.w600,
                color: t.text,
              ),
            ),
          ),
          _Close(onTap: () => Navigator.of(context).pop()),
        ],
      ),
      if (ctx != null)
        _ContextBox(
          at: ctx,
          title: widget.prefill.context == ctx
              ? widget.prefill.contextTitle
              : null,
          vaults: vaults,
          onClear: () => setState(() => _context = null),
        ),
      if (_online.isEmpty)
        Text(
          'No host is online. Start storm-runtime on one, or enroll a host '
          'in Settings › Hosts.',
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.codeSize,
            color: t.text3,
          ),
        )
      else ...[
        _Field(
          label: 'Host',
          child: _Select<String>(
            key: const Key('launcher-host'),
            label: 'Host',
            value: host?.id,
            options: [
              for (final h in _online) _Opt(h.id, '${h.name} · online'),
            ],
            onChanged: (id) => _pickHost(_online.firstWhere((h) => h.id == id)),
          ),
        ),
        _Field(
          label: 'Workspace',
          child: _Select<String>(
            key: const Key('launcher-workspace'),
            label: 'Workspace',
            value: _workspace,
            placeholder: _workspaces == null
                ? '…'
                : 'No workspaces on this host',
            options: [
              for (final w in _workspaces ?? const <AgentWorkspace>[])
                _Opt(
                  w.name,
                  w.liveSessions > 0
                      ? '${w.name}  (another session is working here)'
                      : w.name,
                ),
              if (host != null && _workspaces != null)
                const _Opt(_newWorkspaceKey, 'New workspace…'),
            ],
            onChanged: (w) => w == _newWorkspaceKey
                ? _newWorkspace(host!)
                : setState(() => _workspace = w),
          ),
        ),
        if (host != null && _workspaces != null && _workspaces!.isEmpty)
          _NoWorkspaces(host: host, onRetry: () => _pickHost(host)),
        _Field(
          label: 'Agent',
          child: _Select<String>(
            key: const Key('launcher-agent'),
            label: 'Agent',
            value: _provider,
            options: [
              for (final p in host?.providers ?? const <ProviderCap>[])
                if (p.available)
                  _Opt(
                    p.id,
                    p.id == _default ? '${p.label} · default' : p.label,
                  ),
            ],
            disabled: [
              for (final p in host?.providers ?? const <ProviderCap>[])
                if (!p.available) '${p.label} (not installed)',
            ],
            onChanged: (p) => setState(() => _provider = _chosen = p),
          ),
        ),
        if (_provider == 'claude-code' && host != null && claudeLogin != null)
          _ClaudeLoginLine(login: claudeLogin, host: host),
        if (!shell)
          _Field(
            label: 'Can write to',
            child: Row(
              children: [
                Semantics(
                  label: 'Can write to',
                  child: StormToggle(
                    key: const Key('launcher-writes'),
                    value: writes,
                    enabled: allowed && writeVault != null,
                    onChanged: (v) => setState(() => _writes = v),
                  ),
                ),
                SizedBox(width: t.sp * 1.25),
                Expanded(
                  child: writes
                      ? _Select<String>(
                          key: const Key('launcher-write-vault'),
                          label: 'Write vault',
                          value: writeVault,
                          compact: true,
                          options: [for (final v in vaults) _Opt(v.id, v.name)],
                          onChanged: (v) => setState(() => _writeVault = v),
                        )
                      : Text(
                          allowed ? 'Read only' : 'Off in Settings › AI access',
                          key: const Key('launcher-read-only'),
                          style: TextStyle(
                            fontFamily: StormTokens.sansFamily,
                            fontSize: t.codeSize,
                            color: t.text3,
                          ),
                        ),
                ),
              ],
            ),
          ),
        if (crews != null)
          _Field(
            label: 'Crew',
            child: _Select<String>(
              key: const Key('launcher-crew'),
              label: 'Crew',
              value: crewKey,
              options: crewOptions,
              onChanged: (c) => setState(() => _crew = c),
            ),
          ),
        _Field(
          label: 'Role',
          under: shell
              ? _Under(
                  key: const Key('launcher-role-label-only'),
                  text:
                      "${providerLabel(_provider ?? '')} can't read Storm notes. "
                      'A role would only be a label.',
                )
              : writes && _role != null
              ? const _Under(
                  key: Key('launcher-role-may-not-write'),
                  text: 'The role note may tell the agent not to write.',
                )
              : null,
          child: _Select<String>(
            key: const Key('launcher-role'),
            label: 'Role',
            value: shell ? _noRole : roleKey,
            enabled: !shell,
            options: [
              const _Opt(_noRole, 'None'),
              for (final r in roles) _Opt(_roleKey(r), r.title),
            ],
            menu: (popover, room) => RoleMenu(
              roles: roles,
              room: room,
              selected: _role == null
                  ? null
                  : (vaultId: _role!.vaultId, noteId: _role!.noteId),
              onPick: (r) =>
                  Navigator.of(popover).pop(r == null ? _noRole : _roleKey(r)),
            ),
            onChanged: (k) => setState(
              () => _role = roles.where((r) => _roleKey(r) == k).firstOrNull,
            ),
          ),
        ),
        if (_spawnable)
          _Field(
            label: 'Agents',
            under: const _Under(
              key: Key('launcher-spawn-note'),
              text: 'You approve each start, message and end.',
            ),
            child: Row(
              children: [
                Semantics(
                  label: 'Allow this session to start agents',
                  child: StormToggle(
                    key: const Key('launcher-spawn'),
                    value: _spawn,
                    onChanged: (v) => setState(() => _spawn = v),
                  ),
                ),
                SizedBox(width: t.sp * 1.25),
                Expanded(
                  child: Text(
                    'Allow this session to start agents',
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.codeSize,
                      color: t.text,
                    ),
                  ),
                ),
              ],
            ),
          ),
      ],
      Container(
        key: const Key('launcher-risk'),
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 1.5,
          vertical: t.sp * 1.25,
        ),
        decoration: BoxDecoration(
          color: t.surface2,
          borderRadius: BorderRadius.circular(t.rControl),
        ),
        child: Text(
          _provider == 'shell'
              ? 'Network: inherits $hostName’s policy. A shell has no access to '
                    'your vaults or integrations.'
              : shell
              ? 'Network: inherits $hostName’s policy. '
                    '${providerLabel(_provider ?? '')} runs without access to your '
                    'vaults or integrations until Storm has checked it with a '
                    'signed-in account.'
              : integrations.isEmpty
              ? 'Network: inherits $hostName’s policy. This session can use '
                    'your integrations and read your vaults, with that network '
                    'access.'
              : 'Network: inherits $hostName’s policy. This session can use '
                    'your integrations (${integrations.join(', ')}) and read '
                    'your vaults, with that network access.',
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.labelSize * 1.09,
            height: 1.55,
            color: t.text3,
          ),
        ),
      ),
      if (_error != null)
        Text(
          _error!,
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.codeSize,
            color: t.danger,
          ),
        ),
      Row(
        children: [
          Expanded(
            child: ctx == null
                ? const SizedBox.shrink()
                : Text(
                    'The agent reads the note through Storm.',
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.labelSize * 1.09,
                      color: t.text3,
                    ),
                  ),
          ),
          StormButton.primary(
            key: const Key('launch'),
            label: _busy ? 'Starting' : 'Launch',
            onPressed: _busy || host == null || _workspace == null
                ? null
                : () => _launch(writes, writeVault, crew),
          ),
        ],
      ),
    ];

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (var i = 0; i < children.length; i++) ...[
          if (i > 0) SizedBox(height: t.sp * 1.75),
          children[i],
        ],
      ],
    );
  }
}

class _Close extends StatelessWidget {
  const _Close({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Semantics(
      button: true,
      label: 'Close',
      excludeSemantics: true,
      onTap: onTap,
      child: InkWell(
        key: const Key('launcher-close'),
        onTap: onTap,
        borderRadius: BorderRadius.circular(t.rControl),
        child: Padding(
          padding: EdgeInsets.all(t.sp * 0.5),
          child: Icon(LucideIcons.x, size: t.bodySize, color: t.text3),
        ),
      ),
    );
  }
}

/// CONTEXT, then the note in an `accentSoft` box with its crumb and ×.
class _ContextBox extends ConsumerWidget {
  const _ContextBox({
    required this.at,
    required this.title,
    required this.vaults,
    required this.onClear,
  });

  final NoteRef at;
  final String? title;
  final List<VaultInfo> vaults;
  final VoidCallback onClear;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final note = ref.watch(agentNoteProvider(at)).value;
    final vault =
        vaults.where((v) => v.id == at.vaultId).firstOrNull?.name ?? '';
    // The launch snapshot when there is one: a rename does not change what
    // a session was started from.
    final shown =
        title ??
        (note == null ? '' : noteTitleOf(note.meta.title, note.meta.path));
    return Column(
      key: const Key('launcher-context'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        const AgentsLabel('Context'),
        SizedBox(height: t.sp * 0.75),
        Container(
          padding: EdgeInsets.symmetric(
            horizontal: t.sp * 1.5,
            vertical: t.sp * 1.25,
          ),
          decoration: BoxDecoration(
            color: t.accentSoft,
            borderRadius: BorderRadius.circular(t.rControl),
            border: Border.all(color: t.border, width: t.bw),
          ),
          child: Row(
            children: [
              Icon(LucideIcons.file, size: t.uiSize, color: t.accent),
              SizedBox(width: t.sp * 1.25),
              ConstrainedBox(
                constraints: BoxConstraints(maxWidth: t.sp * 25),
                child: Text(
                  shown,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.uiSize,
                    color: t.text,
                  ),
                ),
              ),
              if (note != null) ...[
                SizedBox(width: t.sp),
                Expanded(
                  child: Text(
                    noteCrumb(vault, note.meta.path),
                    textAlign: TextAlign.right,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontFamily: StormTokens.monoFamily,
                      fontSize: t.labelSize,
                      color: t.text3,
                    ),
                  ),
                ),
              ],
              SizedBox(width: t.sp),
              Semantics(
                button: true,
                label: 'Remove context',
                excludeSemantics: true,
                onTap: onClear,
                child: InkWell(
                  key: const Key('launcher-context-clear'),
                  onTap: onClear,
                  child: Icon(LucideIcons.x, size: t.codeSize, color: t.text3),
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

/// Which Claude login the session will use (decision 124, spec §4), by the
/// server's own rules, under the Agent field.
class _ClaudeLoginLine extends StatelessWidget {
  const _ClaudeLoginLine({required this.login, required this.host});

  final ClaudeLogin login;
  final AgentHost host;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final route = claudeLoginRoute(login, host);
    // Decision 136f: an isolated session never uses the host's own login.
    final own = host.isolationState == 'verified'
        ? 'Claude: needs your synced Claude login or an API key, since this '
              'host isolates sessions'
        : "Claude: this host's own login";
    final soon =
        route == ClaudeLoginRoute.synced &&
        login.warn == '7d' &&
        login.daysLeft != null;
    final text = switch (route) {
      ClaudeLoginRoute.synced =>
        soon
            ? 'Claude: your synced login · expires in ${login.daysLeft} '
                  '${login.daysLeft == 1 ? 'day' : 'days'}, replace it in Settings'
            : 'Claude: your synced login',
      ClaudeLoginRoute.own => own,
      ClaudeLoginRoute.oldHost =>
        "$own · ${host.name} can't receive your synced login, update "
            'storm-runtime',
      ClaudeLoginRoute.expired =>
        '$own · your synced login has expired, re-sync it in Settings',
      ClaudeLoginRoute.needsResync =>
        "$own · your synced login can't be read, sync it again in Settings",
    };
    final warn =
        soon ||
        route == ClaudeLoginRoute.expired ||
        route == ClaudeLoginRoute.needsResync ||
        route == ClaudeLoginRoute.oldHost;
    return Padding(
      padding: EdgeInsets.only(left: t.sp * 13.25),
      child: Text(
        text,
        key: Key('launcher-claude-login-${route.name}'),
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontSize: t.labelSize * 1.09,
          height: 1.45,
          color: warn ? t.danger : t.text3,
        ),
      ),
    );
  }
}

/// A host with nothing to work in yet: where a workspace goes, and a way to
/// look again once one is there.
class _NoWorkspaces extends StatelessWidget {
  const _NoWorkspaces({required this.host, required this.onRetry});

  final AgentHost host;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final root = host.workspaceRoots.firstOrNull;
    final style = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      color: t.text3,
    );
    return Column(
      key: const Key('launcher-no-workspaces'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          'A workspace is a folder in the host\'s workspaces folder. '
          'Make one, or clone a repository there, on ${host.name}:',
          style: style,
        ),
        SizedBox(height: t.sp * 0.75),
        SelectableText(
          root ?? 'the workspace_roots folder in its config.toml',
          style: style.copyWith(
            fontFamily: StormTokens.monoFamily,
            color: t.text2,
          ),
        ),
        SizedBox(height: t.sp * 0.75),
        StormButton.text(
          key: const Key('launcher-workspaces-retry'),
          label: 'Check again',
          onPressed: onRetry,
        ),
      ],
    );
  }
}

class _Field extends StatelessWidget {
  const _Field({required this.label, required this.child, this.under});

  final String label;
  final Widget child;

  /// A line under [child], lined up with it.
  final Widget? under;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final row = Row(
      children: [
        SizedBox(
          width: t.sp * 12,
          child: Text(
            label,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.codeSize,
              color: t.text2,
            ),
          ),
        ),
        SizedBox(width: t.sp * 1.25),
        Expanded(child: child),
      ],
    );
    if (under == null) return row;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        row,
        Padding(
          padding: EdgeInsets.only(left: t.sp * 13.25, top: t.sp * 0.75),
          child: under,
        ),
      ],
    );
  }
}

/// No workspace name starts with a dot, so this is never one.
const _newWorkspaceKey = '.new';
const _noCrew = '';
const _newCrew = 'new';
const _noRole = '';

String _roleKey(RoleNote r) => '${r.vaultId}/${r.noteId}';

/// A quiet line under a field's control.
class _Under extends StatelessWidget {
  const _Under({super.key, required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Text(
      text,
      style: TextStyle(
        fontFamily: StormTokens.sansFamily,
        fontSize: t.labelSize * 1.09,
        height: 1.45,
        color: t.text3,
      ),
    );
  }
}

/// One choice in a [_Select].
class _Opt<T> {
  const _Opt(this.value, this.label);

  final T value;
  final String label;
}

/// A select: `surface2`, a hairline, `rControl`, a trailing chevron (138h),
/// and a popover of options.
class _Select<T> extends StatefulWidget {
  const _Select({
    super.key,
    required this.label,
    required this.value,
    required this.options,
    required this.onChanged,
    this.disabled = const [],
    this.placeholder = '',
    this.compact = false,
    this.enabled = true,
    this.menu,
  });

  final String label;
  final T? value;
  final List<_Opt<T>> options;
  final ValueChanged<T> onChanged;

  /// Shown in the menu, never choosable (an agent not installed).
  final List<String> disabled;
  final String placeholder;
  final bool compact;

  /// False draws it muted and opens nothing.
  final bool enabled;

  /// The popover in place of the plain list, given the height it has; it
  /// pops the value picked.
  final Widget Function(BuildContext popover, double room)? menu;

  @override
  State<_Select<T>> createState() => _SelectState<T>();
}

class _SelectState<T> extends State<_Select<T>> {
  final _anchor = GlobalKey();

  Future<void> _open() async {
    final box = _anchor.currentContext?.findRenderObject() as RenderBox?;
    final room = popoverRoom(context, _anchor);
    final picked = await showStormPopover<T>(
      context: context,
      anchorKey: _anchor,
      width: box?.size.width ?? 240,
      builder: (popover) =>
          widget.menu?.call(popover, room) ??
          StormPopover(
            children: [
              for (final o in widget.options)
                PopoverItem(
                  label: o.label,
                  selected: o.value == widget.value,
                  onTap: () => Navigator.of(popover).pop(o.value),
                ),
              for (final label in widget.disabled) PopoverItem(label: label),
            ],
          ),
    );
    if (picked != null && picked != widget.value) widget.onChanged(picked);
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final shown =
        widget.options
            .where((o) => o.value == widget.value)
            .firstOrNull
            ?.label ??
        widget.placeholder;
    final live = widget.enabled && widget.options.isNotEmpty;
    return Semantics(
      button: true,
      enabled: live,
      label: '${widget.label}: $shown',
      excludeSemantics: true,
      onTap: live ? _open : null,
      child: Material(
        key: _anchor,
        color: t.surface2,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(t.rControl),
          side: BorderSide(color: t.border, width: t.bw),
        ),
        child: InkWell(
          onTap: live ? _open : null,
          borderRadius: BorderRadius.circular(t.rControl),
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 1.5,
              vertical: t.sp * (widget.compact ? 0.875 : 1.125),
            ),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    shown,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.uiSize,
                      color: live ? t.text : t.text3,
                    ),
                  ),
                ),
                SizedBox(width: t.sp),
                Icon(
                  LucideIcons.chevron_down,
                  key: const Key('select-chevron'),
                  size: t.codeSize,
                  color: live ? t.text2 : t.text3,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// Raises the launcher in the shape the width calls for: a 460 modal at
/// [kExpandedWidth] and wider, a bottom sheet below it.
Future<AgentSession?> showNewSessionLauncher(
  BuildContext context, {
  required List<AgentHost> hosts,
  required bool wide,
  LaunchPrefill prefill = const LaunchPrefill(),
}) {
  final t = context.tokens;
  final launcher = NewSessionLauncher(hosts: hosts, prefill: prefill);
  if (!wide) {
    return showModalBottomSheet<AgentSession>(
      context: context,
      isScrollControlled: true,
      backgroundColor: t.surface,
      barrierColor: const Color(0x59000000),
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(t.rCard)),
        side: BorderSide(color: t.border, width: t.bw),
      ),
      builder: (sheet) => Padding(
        padding: EdgeInsets.only(bottom: MediaQuery.viewInsetsOf(sheet).bottom),
        child: SafeArea(
          top: false,
          child: SingleChildScrollView(
            padding: EdgeInsets.fromLTRB(
              t.sp * 2.5,
              t.sp * 2.5,
              t.sp * 2.5,
              t.sp * 4,
            ),
            child: launcher,
          ),
        ),
      ),
    );
  }
  return showGeneralDialog<AgentSession>(
    context: context,
    barrierDismissible: true,
    barrierLabel: 'Close',
    barrierColor: const Color(0x66000000),
    transitionDuration: t.duration,
    pageBuilder: (dialog, _, _) => Align(
      alignment: Alignment.topCenter,
      child: Padding(
        padding: EdgeInsets.only(top: t.sp * 12, bottom: t.sp * 3),
        child: SizedBox(
          width: t.sp * 57.5,
          child: DecoratedBox(
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(t.rCard),
              boxShadow: t.shadow,
            ),
            child: Material(
              color: t.surface,
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(t.rCard),
                side: BorderSide(color: t.border, width: t.bw),
              ),
              // Scrolls in a short window rather than overflowing it.
              child: SingleChildScrollView(
                padding: EdgeInsets.all(t.sp * 2.75),
                child: launcher,
              ),
            ),
          ),
        ),
      ),
    ),
  );
}

/// A launch's or a workspace list's failure as a sentence (decision 141d):
/// a refusal code is never shown bare.
String launchErrorText(Object e) {
  final code = e is AgentRefusal ? e.code : null;
  if (code == null || !_isCode(code)) return describeFailure(e);
  return switch (code) {
    'workspace_held' => 'Another session is working in this workspace.',
    'credential_unavailable' =>
      'This host isolates sessions, so Claude Code needs your synced Claude '
          'login or an API key.',
    'isolation_unavailable' || 'isolation_unverified' =>
      "This host's session isolation isn't working, so it isn't starting "
          'new sessions.',
    'owner_only' => 'Only the server’s owner can do that.',
    'spawn_unavailable' =>
      "This host or agent can't offer starting agents now. Turn off "
          '"Allow this session to start agents" to launch.',
    _ => "Couldn't launch: $code",
  };
}

/// What a refused "New workspace…" says (decision 141c).
String newWorkspaceErrorText(Object e, String name) {
  final code = e is AgentRefusal ? e.code : null;
  if (e is AgentRefusal && e.statusCode == 503) return 'The host is offline.';
  return switch (code) {
    'bad_name' =>
      'A workspace name is one folder name: no “/”, “\\” or “:”, and not '
          'starting with a dot.',
    'exists' => 'There is already a folder named “$name” on this host.',
    'no_root' =>
      "This host's workspace folder is missing. Check workspace_roots in its "
          'runtime.toml.',
    'not_writable' =>
      "storm-runtime can't write to this host's workspace "
          'folder.',
    'git_failed' =>
      "It couldn't be started as a git repository (is git installed on the "
          'host?), so it wasn’t kept.',
    'old_host' =>
      "This host's storm-runtime can't make workspaces. Update it, or make "
          'the folder on the host.',
    'timeout' => "The host didn't answer in time. Check the list again.",
    'owner_only' => 'Only the server’s owner can make workspaces.',
    'failed' => "The host couldn't make the folder.",
    null => describeFailure(e),
    _ => "Couldn't make the workspace: $code",
  };
}

bool _isCode(String s) => RegExp(r'^[a-z][a-z0-9_]*$').hasMatch(s);

/// "New workspace…": a name and whether to start it as a git repository
/// (on by default). It stays open on a refusal, saying why; it pops the
/// workspace's name once the host has made it.
class NewWorkspaceDialog extends ConsumerStatefulWidget {
  const NewWorkspaceDialog({super.key, required this.host});

  final AgentHost host;

  @override
  ConsumerState<NewWorkspaceDialog> createState() => _NewWorkspaceState();
}

class _NewWorkspaceState extends ConsumerState<NewWorkspaceDialog> {
  late final _name = TextEditingController();
  bool _git = true;
  bool _busy = false;
  String? _error;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _create() async {
    final name = _name.text.trim();
    if (name.isEmpty || _busy) return;
    final api = agentApi(ref);
    if (api == null) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final made = await api.createWorkspace(widget.host.id, name, git: _git);
      if (mounted) Navigator.of(context).pop(made);
    } catch (e) {
      if (mounted) setState(() => _error = newWorkspaceErrorText(e, name));
    } finally {
      api.dispose();
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final style = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      color: t.text2,
    );
    return AlertDialog(
      title: Text('New workspace on ${widget.host.name}'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          TextField(
            key: const Key('new-workspace-name'),
            controller: _name,
            autofocus: true,
            autocorrect: false,
            maxLength: 128,
            decoration: const InputDecoration(labelText: 'Name'),
            onChanged: (_) => setState(() {}),
            onSubmitted: (_) => _create(),
          ),
          SizedBox(height: t.sp),
          Row(
            children: [
              Semantics(
                label: 'Start as a git repository',
                child: StormToggle(
                  key: const Key('new-workspace-git'),
                  value: _git,
                  onChanged: _busy ? null : (v) => setState(() => _git = v),
                ),
              ),
              SizedBox(width: t.sp),
              Flexible(child: Text('Start as a git repository', style: style)),
            ],
          ),
          if (_error != null) ...[
            SizedBox(height: t.sp * 1.5),
            Text(
              _error!,
              key: const Key('new-workspace-error'),
              style: style.copyWith(color: t.danger),
            ),
          ],
        ],
      ),
      actions: [
        TextButton(
          onPressed: _busy ? null : () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('new-workspace-create'),
          onPressed: _busy || _name.text.trim().isEmpty ? null : _create,
          child: Text(_busy ? 'Creating' : 'Create'),
        ),
      ],
    );
  }
}

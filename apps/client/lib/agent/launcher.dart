import 'dart:math' as math;

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
import 'spawn_models.dart' show kHostTooOldToSpawn, kSpawnProviders;

part 'launcher_parts.dart';

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

/// The New session launcher (153-5): From, Agent and Where, what the
/// session can do as a row of chips, then More in place. A centred modal at
/// desk width, a bottom sheet on a phone. Pops the launched session, or
/// nothing.
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

  /// Each online host's workspaces, null until listed.
  final _spaces = <String, List<AgentWorkspace>?>{};
  final _spacesFailed = <String>{};
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

  // On by default (operator, 2026-11-10); the person can still turn it off.
  bool _spawn = true;

  /// Its children write to its write vault (151m): off by default.
  bool _childrenWrite = false;

  /// The model (decision 145b): empty is the agent's own default.
  final _model = TextEditingController();

  /// The permission mode (decision 146b), one [permissionModesOf] offers.
  String _mode = 'default';
  String? _error;
  bool _busy = false;

  /// More is open in place (153-5), from the start when a crew is preset
  /// so the person sees the crew the session joins.
  late bool _more = widget.prefill.crewId != null;

  /// Why the chip last tapped can't be changed for this agent.
  String? _chipReason;

  /// The host whose inline New workspace field is open.
  String? _newWorkspaceOn;

  final _writesAnchor = GlobalKey();
  final _spawnAnchor = GlobalKey();
  final _riskAnchor = GlobalKey();

  @override
  void dispose() {
    _model.dispose();
    super.dispose();
  }

  List<AgentHost> get _online => [
    for (final h in widget.hosts)
      if (h.online) h,
  ];

  List<AgentWorkspace>? get _workspaces =>
      _host == null ? null : _spaces[_host!.id];

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
  /// device launched on last, else the first online (decision 78). Every
  /// other online host's workspaces are listed too, for Where.
  Future<void> _preselectHost() async {
    final online = _online;
    if (online.isEmpty) return;
    final wanted = widget.prefill.hostId ?? await lastLaunchHost();
    if (!mounted || _host != null) return;
    final first =
        online.where((h) => h.id == wanted).firstOrNull ?? online.first;
    final picked = _pickHost(first, keepWorkspace: widget.prefill.workspace);
    for (final h in online) {
      if (h.id != first.id) _list(h);
    }
    await picked;
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

  /// Lists [host]'s workspaces; only the chosen host's failure is said.
  Future<List<AgentWorkspace>?> _list(AgentHost host) async {
    final api = agentApi(ref);
    if (api == null) return null;
    try {
      final ws = await api.workspaces(host.id);
      if (!mounted) return null;
      setState(() {
        _spaces[host.id] = ws;
        _spacesFailed.remove(host.id);
      });
      return ws;
    } catch (e) {
      if (!mounted) return null;
      setState(() {
        _spacesFailed.add(host.id);
        if (_host?.id == host.id) _error = launchErrorText(e);
      });
      return null;
    } finally {
      api.dispose();
    }
  }

  Future<void> _pickHost(AgentHost host, {String? keepWorkspace}) async {
    setState(() {
      _host = host;
      _spaces[host.id] = null;
      _workspace = null;
      _settleProvider();
    });
    final ws = await _list(host);
    if (ws == null || !mounted || _host?.id != host.id) return;
    setState(() {
      _workspace =
          ws.where((w) => w.name == keepWorkspace).firstOrNull?.name ??
          ws.firstOrNull?.name;
    });
  }

  /// A workspace × host from Where's picker.
  void _pickWhere(String value) {
    final (hostId, name) = _splitWhere(value);
    final host = _online.where((h) => h.id == hostId).firstOrNull;
    if (host == null) return;
    if (name == _newWorkspaceKey) {
      _openNewWorkspace(host);
      return;
    }
    setState(() {
      _error = null;
      _newWorkspaceOn = null;
    });
    final listed = _spaces[host.id];
    if (_host?.id == host.id || listed != null) {
      setState(() {
        if (_host?.id != host.id) {
          _host = host;
          _settleProvider();
        }
        _workspace = name;
      });
    } else {
      _pickHost(host, keepWorkspace: name);
    }
  }

  /// "New workspace…" (decision 141c), inline under Where (153-5).
  void _openNewWorkspace(AgentHost host) {
    if (_host?.id != host.id) {
      if (_spaces[host.id] == null) {
        _pickHost(host);
      } else {
        setState(() {
          _host = host;
          _workspace = _spaces[host.id]!.firstOrNull?.name;
          _settleProvider();
        });
      }
    }
    setState(() => _newWorkspaceOn = host.id);
  }

  Future<void> _workspaceMade(AgentHost host, String name) async {
    setState(() {
      _newWorkspaceOn = null;
      _error = null;
    });
    if (_host?.id != host.id) return;
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
        childrenWrite:
            _childrenWrite &&
            _spawn &&
            _spawnable &&
            writes &&
            writeVault != null,
        model: modelHint(_provider) == null ? null : _model.text.trim(),
        permissionMode: _permissionMode,
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

  String get _agentName => providerLabel(_provider ?? '');

  /// Why this agent on this host can't start agents, or null when it can.
  String? get _spawnBlocked {
    if (!kSpawnProviders.contains(_provider)) {
      return "$_agentName can't start agents";
    }
    if (!_reachesStorm) return "$_agentName can't reach Storm";
    final host = _host;
    if (host == null) return null;
    if (host.execution == null) return kHostTooOldToSpawn;
    if (!host.offersSpawning) {
      return "${host.name}'s session isolation isn't working";
    }
    return null;
  }

  /// The chosen mode while the agent offers it, else its own.
  String get _permissionMode =>
      permissionModesOf(_provider).contains(_mode) ? _mode : 'default';

  /// Opens More with [anchor]'s row in view: a chip's setting.
  void _openAt(GlobalKey anchor) {
    setState(() {
      _more = true;
      _chipReason = null;
    });
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final at = anchor.currentContext;
      if (at == null || !at.mounted) return;
      Scrollable.ensureVisible(
        at,
        duration: context.tokens.duration,
        alignment: 0.2,
      );
    });
  }

  Widget _toggleLine({
    required Key key,
    required String text,
    required bool value,
    required bool enabled,
    required ValueChanged<bool> onChanged,
    Widget? trailing,
  }) {
    final t = context.tokens;
    return Row(
      children: [
        Semantics(
          label: text,
          child: StormToggle(
            key: key,
            value: value,
            enabled: enabled,
            onChanged: onChanged,
          ),
        ),
        SizedBox(width: t.sp * 1.25),
        Expanded(
          child:
              trailing ??
              Text(
                text,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  color: enabled ? t.text : t.text3,
                ),
              ),
        ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, box) =>
          _form(context, stacked: box.maxWidth < context.tokens.sp * 50),
    );
  }

  Widget _form(BuildContext context, {required bool stacked}) {
    final t = context.tokens;
    final host = _host;
    final shell = !_reachesStorm;
    final agent = _agentName;
    final allowed = ref.watch(agentWritesSettingProvider).value == true;
    final vaults = [
      for (final v in ref.watch(vaultsProvider).value ?? const <VaultInfo>[])
        if (!v.missing) v,
    ];
    final writeVault =
        vaults.where((v) => v.id == _writeVault).firstOrNull?.id ??
        vaults.firstOrNull?.id;
    final writeVaultName = vaults
        .where((v) => v.id == writeVault)
        .firstOrNull
        ?.name;
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
    final spawnBlocked = _spawnBlocked;
    final spawnOn = _spawn && spawnBlocked == null;
    final limits = host?.limits ?? const HostLimits();
    final held =
        (_workspaces
                ?.where((w) => w.name == _workspace)
                .firstOrNull
                ?.liveSessions ??
            0) >
        0;

    Widget field(
      String row,
      String? label, {
      required Widget child,
      Widget? under,
      bool underGap = true,
      GlobalKey? anchor,
    }) => _Field(
      key: Key('launcher-row-$row'),
      anchor: anchor,
      label: label,
      stacked: stacked,
      under: under,
      underGap: underGap,
      child: child,
    );

    final writesReason = shell ? "$agent can't reach Storm" : null;
    final childrenReason = spawnOn
        ? (writes && !shell ? null : 'Turn on Write to first')
        : 'Turn on Start agents first';

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
        field(
          'from',
          'From',
          child: _ContextChip(
            at: ctx,
            title: widget.prefill.context == ctx
                ? widget.prefill.contextTitle
                : null,
            vaults: vaults,
            onClear: () => setState(() => _context = null),
          ),
        ),
      if (_online.isEmpty)
        Text(
          'No host is online. Start storm-runtime on one, or enroll a host '
          'in Settings › Agents & hosts.',
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.codeSize,
            color: t.text3,
          ),
        )
      else ...[
        field(
          'agent',
          'Agent',
          underGap: false,
          // Claude's login line has a slot for every agent, empty for the
          // others, so nothing below moves when the agent changes.
          under: KeyedSubtree(
            key: const Key('launcher-agent-sub'),
            child:
                _provider == 'claude-code' &&
                    host != null &&
                    claudeLogin != null
                ? Padding(
                    padding: EdgeInsets.only(top: t.sp * 0.75),
                    child: _ClaudeLoginLine(login: claudeLogin, host: host),
                  )
                : const SizedBox.shrink(),
          ),
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
            onChanged: (p) => setState(() {
              _provider = _chosen = p;
              _mode = _permissionMode;
              _chipReason = null;
            }),
          ),
        ),
        field(
          'where',
          'Where',
          under: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              if (host != null)
                _Under(
                  key: Key(
                    held
                        ? 'launcher-host-execution-held'
                        : 'launcher-host-execution',
                  ),
                  text: held
                      ? '${host.executionLine} · another session is working '
                            'here'
                      : host.executionLine,
                ),
              if (host != null &&
                  _workspaces != null &&
                  _workspaces!.isEmpty &&
                  _newWorkspaceOn != host.id) ...[
                SizedBox(height: t.sp),
                _NoWorkspaces(host: host, onRetry: () => _pickHost(host)),
              ],
              if (host != null && _newWorkspaceOn == host.id) ...[
                SizedBox(height: t.sp),
                _NewWorkspaceForm(
                  key: ValueKey('new-workspace-${host.id}'),
                  host: host,
                  onMade: (name) => _workspaceMade(host, name),
                  onCancel: () => setState(() => _newWorkspaceOn = null),
                ),
              ] else if (host != null && _workspaces != null) ...[
                SizedBox(height: t.sp * 0.5),
                _Link(
                  key: const Key('launcher-new-workspace'),
                  text: 'New workspace…',
                  onTap: () => _openNewWorkspace(host),
                ),
              ],
            ],
          ),
          child: _Select<String>(
            key: const Key('launcher-where'),
            label: 'Where',
            value: host == null ? null : _whereKey(host.id, _workspace ?? ''),
            placeholder: host == null
                ? ''
                : _workspaces == null
                ? '… on ${host.name}'
                : 'No workspace on ${host.name}',
            options: [
              for (final h in _online)
                for (final w in _spaces[h.id] ?? const <AgentWorkspace>[])
                  _Opt(_whereKey(h.id, w.name), '${w.name} on ${h.name}'),
            ],
            alwaysOpens: true,
            menu: (popover, room) => _WhereMenu(
              hosts: _online,
              spaces: _spaces,
              failed: _spacesFailed,
              selected: host == null
                  ? null
                  : _whereKey(host.id, _workspace ?? ''),
              room: room,
              onPick: (v) => Navigator.of(popover).pop(v),
            ),
            onChanged: _pickWhere,
          ),
        ),
        if (!_more) ...[
          const AgentsLabel('This session can'),
          Wrap(
            key: const Key('launcher-chips'),
            spacing: t.sp,
            runSpacing: t.sp,
            children: [
              LauncherChip(
                key: const Key('launcher-chip-writes'),
                label: 'Writes to ${writeVaultName ?? 'a vault'}',
                state: shell || !allowed || writeVault == null
                    ? LauncherChipState.unavailable
                    : writes
                    ? LauncherChipState.on
                    : LauncherChipState.off,
                onTap: () => shell || !allowed || writeVault == null
                    ? setState(
                        () => _chipReason =
                            writesReason ??
                            (allowed
                                ? 'There is no vault to write to'
                                : 'Off in Settings › AI access'),
                      )
                    : _openAt(_writesAnchor),
              ),
              LauncherChip(
                key: const Key('launcher-chip-spawn'),
                label: 'Starts agents',
                state: spawnBlocked != null
                    ? LauncherChipState.unavailable
                    : spawnOn
                    ? LauncherChipState.on
                    : LauncherChipState.off,
                onTap: () => spawnBlocked != null
                    ? setState(() => _chipReason = spawnBlocked)
                    : _openAt(_spawnAnchor),
              ),
              LauncherChip(
                key: const Key('launcher-chip-integrations'),
                label: switch (integrations.length) {
                  0 => 'Uses no integrations',
                  1 => 'Uses 1 integration',
                  final n => 'Uses $n integrations',
                },
                state: shell
                    ? LauncherChipState.unavailable
                    : integrations.isEmpty
                    ? LauncherChipState.off
                    : LauncherChipState.on,
                onTap: () => shell
                    ? setState(() => _chipReason = "$agent can't reach Storm")
                    : _openAt(_riskAnchor),
              ),
              LauncherChip(
                key: const Key('launcher-chip-network'),
                label: 'Network: $hostName’s policy',
                state: LauncherChipState.off,
                onTap: () => _openAt(_riskAnchor),
              ),
            ],
          ),
          _Under(
            key: Key(
              _chipReason == null
                  ? 'launcher-chips-hint'
                  : 'launcher-chip-reason',
            ),
            text:
                _chipReason ??
                'Tap one to change it. Everything else uses the agent’s '
                    'defaults.',
          ),
          Align(
            alignment: Alignment.centerLeft,
            child: _Link(
              key: const Key('launcher-more'),
              text: 'More: model, permission mode, crew, role ›',
              onTap: () => setState(() {
                _more = true;
                _chipReason = null;
              }),
            ),
          ),
        ] else ...[
          const AgentsLabel('This session can'),
          field(
            'writes',
            'Write to',
            anchor: _writesAnchor,
            under: writesReason == null
                ? null
                : _Under(
                    key: const Key('launcher-writes-reason'),
                    text: writesReason,
                  ),
            child: _toggleLine(
              key: const Key('launcher-writes'),
              text: 'Write to',
              value: writes && !shell,
              enabled: allowed && writeVault != null && !shell,
              onChanged: (v) => setState(() => _writes = v),
              trailing: writes && !shell
                  ? Align(
                      alignment: Alignment.centerLeft,
                      child: _Select<String>(
                        key: const Key('launcher-write-vault'),
                        label: 'Write vault',
                        value: writeVault,
                        compact: true,
                        options: [for (final v in vaults) _Opt(v.id, v.name)],
                        onChanged: (v) => setState(() => _writeVault = v),
                      ),
                    )
                  : Text(
                      allowed || shell
                          ? 'Read only'
                          : 'Off in Settings › AI access',
                      key: const Key('launcher-read-only'),
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.codeSize,
                        color: t.text3,
                      ),
                    ),
            ),
          ),
          field(
            'spawn',
            'Start agents',
            anchor: _spawnAnchor,
            under: spawnBlocked == null
                ? const _Under(
                    key: Key('launcher-spawn-note'),
                    text:
                        'Its starts, messages and ends act at once and show '
                        'in About.',
                  )
                : _Under(
                    key: const Key('launcher-spawn-reason'),
                    text: spawnBlocked,
                  ),
            child: _toggleLine(
              key: const Key('launcher-spawn'),
              text:
                  'Up to ${limits.maxChildren} at once, ${limits.maxTree} in '
                  'its tree',
              value: spawnOn,
              enabled: spawnBlocked == null,
              onChanged: (v) => setState(() => _spawn = v),
            ),
          ),
          field(
            'children-write',
            null,
            under: childrenReason == null
                ? null
                : _Under(
                    key: const Key('launcher-children-write-reason'),
                    text: childrenReason,
                  ),
            child: _toggleLine(
              key: const Key('launcher-children-write'),
              text:
                  'Let the agents it starts write to '
                  '${writeVaultName ?? 'its vault'}',
              value: _childrenWrite && childrenReason == null,
              enabled: childrenReason == null,
              onChanged: (v) => setState(() => _childrenWrite = v),
            ),
          ),
          KeyedSubtree(
            key: _riskAnchor,
            child: Text(
              key: const Key('launcher-risk'),
              _provider == 'shell'
                  ? 'Network: inherits $hostName’s policy. A shell has no access to '
                        'your vaults or integrations.'
                  : shell
                  ? 'Network: inherits $hostName’s policy. '
                        '$agent runs without access to your '
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
          const AgentsLabel('More'),
          field(
            'model',
            'Model',
            under: _ModelUnder(
              agent: agent,
              hint: modelHint(_provider),
              suggestions: modelSuggestions(_provider),
              onPick: (m) => setState(() => _model.text = m),
            ),
            child: TextField(
              key: const Key('launcher-model'),
              controller: _model,
              enabled: modelHint(_provider) != null,
              autocorrect: false,
              enableSuggestions: false,
              maxLength: 128,
              style: TextStyle(
                fontFamily: StormTokens.monoFamily,
                fontSize: t.codeSize,
                color: t.text,
              ),
              decoration: InputDecoration(
                isDense: true,
                counterText: '',
                hintText: 'The agent’s default',
                hintStyle: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.uiSize,
                  color: t.text3,
                ),
              ),
            ),
          ),
          field(
            'permission-mode',
            'Permission mode',
            under: _Under(
              key: const Key('launcher-permission-note'),
              text: permissionModesOf(_provider).length > 1
                  ? kPermissionModeNote
                  : '$agent has only its own setting. Claude Code and '
                        'OpenCode only.',
            ),
            child: _Select<String>(
              key: const Key('launcher-permission-mode'),
              label: 'Permission mode',
              value: _permissionMode,
              enabled: permissionModesOf(_provider).length > 1,
              options: [
                for (final m in permissionModesOf(_provider))
                  _Opt(m, permissionModeLabel(m)),
              ],
              onChanged: (m) => setState(() => _mode = m),
            ),
          ),
          if (crews != null)
            field(
              'crew',
              'Crew',
              child: _Select<String>(
                key: const Key('launcher-crew'),
                label: 'Crew',
                value: crewKey,
                options: crewOptions,
                onChanged: (c) => setState(() => _crew = c),
              ),
            ),
          field(
            'role',
            'Role',
            under: shell
                ? _Under(
                    key: const Key('launcher-role-label-only'),
                    text:
                        "$agent can't read Storm notes. "
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
                const _Opt(_noRole, 'None · a plain agent'),
                for (final r in roles) _Opt(_roleKey(r), r.title),
              ],
              menu: (popover, room) => RoleMenu(
                roles: roles,
                room: room,
                selected: _role == null
                    ? null
                    : (vaultId: _role!.vaultId, noteId: _role!.noteId),
                onPick: (r) => Navigator.of(
                  popover,
                ).pop(r == null ? _noRole : _roleKey(r)),
              ),
              onChanged: (k) => setState(
                () => _role = roles.where((r) => _roleKey(r) == k).firstOrNull,
              ),
            ),
          ),
          Align(
            alignment: Alignment.centerLeft,
            child: _Link(
              key: const Key('launcher-fewer'),
              text: 'Fewer options',
              onTap: () => setState(() => _more = false),
            ),
          ),
        ],
      ],
      if (_error != null)
        Text(
          _error!,
          style: TextStyle(
            fontFamily: StormTokens.sansFamily,
            fontSize: t.codeSize,
            color: t.danger,
          ),
        ),
      Align(
        alignment: Alignment.centerRight,
        child: StormButton.primary(
          key: const Key('launch'),
          label: _busy ? 'Starting' : 'Launch',
          expand: stacked,
          onPressed: _busy || host == null || _workspace == null
              ? null
              : () => _launch(writes, writeVault, crew),
        ),
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

/// What Permission mode says under it (153-5).
const kPermissionModeNote =
    'Claude Code and OpenCode only. Tab and ⇧Tab change it in the terminal.';

/// What the launcher's Model field says for [provider], or null where Storm
/// passes no model (decision 145b).
String? modelHint(String? provider) => switch (provider) {
  'claude-code' => 'haiku, sonnet, opus or a full id like claude-sonnet-4-5',
  'opencode' => 'provider/model, e.g. anthropic/claude-sonnet-4-5',
  'codex' => 'a model id, e.g. gpt-5-codex',
  _ => null,
};

/// The models the launcher offers to fill in over the agent's default
/// (153-5); the field still takes any the agent accepts.
List<String> modelSuggestions(String? provider) => switch (provider) {
  'claude-code' => const ['opus', 'sonnet', 'haiku'],
  _ => const [],
};

/// No workspace name starts with a dot, so this is never one.
const _newWorkspaceKey = '.new';
const _noCrew = '';
const _newCrew = 'new';
const _noRole = '';

String _roleKey(RoleNote r) => '${r.vaultId}/${r.noteId}';

/// Where's value: a host id and a workspace, which never holds a NUL.
String _whereKey(String hostId, String workspace) => '$hostId\u0000$workspace';

(String, String) _splitWhere(String v) {
  final i = v.indexOf('\u0000');
  return (v.substring(0, i), v.substring(i + 1));
}

/// Raises the launcher in the shape the width calls for: a 480 modal at
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
          child: ConstrainedBox(
            constraints: BoxConstraints(
              maxHeight: MediaQuery.sizeOf(sheet).height * 0.9,
            ),
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
          width: math.min(t.sp * 60, MediaQuery.sizeOf(dialog).width),
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
  // 145b: the server names the forms the agent accepts in a sentence.
  const model = 'model_unavailable';
  if (code != null && code.startsWith(model)) {
    final why = code.substring(model.length).replaceFirst(RegExp(r'^:\s*'), '');
    return why.isEmpty ? 'That model isn’t one this agent takes.' : why;
  }
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
          'Start agents under More to launch.',
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

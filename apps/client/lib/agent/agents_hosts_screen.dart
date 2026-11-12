import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../ui/breakpoints.dart';
import '../ui/controls.dart';
import '../ui/settings/settings_widgets.dart';
import '../ui/states.dart';
import '../ui/surfaces.dart' show showStormSheet;
import '../ui/tokens.dart';
import 'agent_api.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'agents_hosts_parts.dart';
import 'agents_settings_screen.dart';
import 'hosts_screen.dart';

/// Settings › Agents & hosts (153-7): the hosts as rows and the agents as
/// columns, each cell one word for that agent on that host. It merges
/// Hosts & default agent with Agents (decision 133). Owner only.
class AgentsHostsScreen extends ConsumerStatefulWidget {
  const AgentsHostsScreen({
    super.key,
    this.pollEvery = const Duration(seconds: 1),
  });

  /// How often a running install is asked about.
  final Duration pollEvery;

  @override
  ConsumerState<AgentsHostsScreen> createState() => _AgentsHostsScreenState();
}

typedef _Cell = ({String host, String agent});

class _AgentsHostsScreenState extends ConsumerState<AgentsHostsScreen> {
  List<AgentHost>? _hosts;
  String? _defaultProvider;
  String? _error;
  bool _loading = false;
  String? _selected;

  /// By `<host id>/<agent>`.
  final _runs = <String, AgentRun>{};
  final _polls = <String, Timer>{};

  final _popover = OverlayPortalController();
  final _links = <String, LayerLink>{};
  _Cell? _open;

  /// The server answered Remove or Re-enroll 404 or 405: it is older than
  /// decision 153, so neither is offered.
  bool _noHostAdmin = false;

  /// Hosts that refused a typed Shell `host_cannot_type` (153-S3).
  final _cannotType = <String>{};

  /// What the phone's sheet rebuilds from: it is outside this widget.
  final _tick = ValueNotifier(0);
  bool _sheetOpen = false;

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    for (final t in _polls.values) {
      t.cancel();
    }
    _tick.dispose();
    super.dispose();
  }

  @override
  void setState(VoidCallback fn) {
    super.setState(fn);
    _tick.value++;
  }

  Future<T?> _with<T>(Future<T> Function(AgentApi api) run) async {
    final api = agentApi(ref);
    if (api == null) {
      setState(() => _error = 'Sign in first.');
      return null;
    }
    try {
      return await run(api);
    } catch (e) {
      if (mounted) setState(() => _error = describeFailure(e));
      return null;
    } finally {
      api.dispose();
    }
  }

  Future<void> _load() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    final result = await _with(
      (api) async => (await api.hosts(), await api.defaultProvider()),
    );
    if (!mounted) return;
    setState(() {
      if (result != null) {
        _hosts = result.$1;
        _defaultProvider = result.$2;
        final live = result.$1.where((h) => h.status != 'revoked');
        if (!live.any((h) => h.id == _selected)) {
          _selected = live.firstOrNull?.id;
        }
      }
      _loading = false;
    });
    // The sessions sidebar beside this page shows the same hosts.
    ref.invalidate(agentOverviewProvider);
  }

  Future<void> _enroll() async {
    final issued = await _with((api) => api.enroll());
    if (issued == null || !mounted) return;
    await showDialog<void>(
      context: context,
      barrierDismissible: false,
      builder: (_) =>
          EnrollDialog(enrollment: issued.enrollment, expires: issued.expires),
    );
    if (mounted) await _load();
  }

  bool _olderServer(Object e) =>
      e is AgentRefusal && (e.statusCode == 404 || e.statusCode == 405);

  Future<void> _reenroll(AgentHost host) async {
    final api = agentApi(ref);
    if (api == null) return;
    try {
      final issued = await api.reenrollHost(host.id);
      if (!mounted) return;
      await showDialog<void>(
        context: context,
        barrierDismissible: false,
        builder: (_) => EnrollDialog(
          enrollment: issued.enrollment,
          expires: issued.expires,
          title: 'Re-enroll ${host.name}',
        ),
      );
      if (mounted) await _load();
    } catch (e) {
      if (!mounted) return;
      setState(() {
        if (_olderServer(e)) {
          _noHostAdmin = true;
        } else {
          _error = describeFailure(e);
        }
      });
    } finally {
      api.dispose();
    }
  }

  Future<void> _remove(AgentHost host) async {
    final api = agentApi(ref);
    if (api == null) return;
    try {
      await api.removeHost(host.id);
      if (mounted) await _load();
    } catch (e) {
      if (!mounted) return;
      setState(() {
        if (_olderServer(e)) {
          _noHostAdmin = true;
        } else {
          _error = describeFailure(e);
        }
      });
    } finally {
      api.dispose();
    }
  }

  /// Remove… on a host that may still be live: revoke it, then remove it.
  /// A remove refused after the revoke leaves it revoked, with its row's
  /// own Remove.
  Future<void> _revokeAndRemove(AgentHost host) async {
    final api = agentApi(ref);
    if (api == null) return;
    try {
      if (host.status != 'revoked') {
        try {
          await api.revokeHost(host.id);
        } catch (e) {
          if (mounted) setState(() => _error = describeFailure(e));
          return;
        }
      }
      try {
        await api.removeHost(host.id);
      } catch (e) {
        if (!mounted) return;
        final older = _olderServer(e);
        if (older) setState(() => _noHostAdmin = true);
        final reason = older
            ? 'this server is too old to remove hosts'
            : describeFailure(e);
        ScaffoldMessenger.maybeOf(context)?.showSnackBar(
          SnackBar(
            content: Text(
              "Revoked ${host.name}, but couldn't remove it: "
              '$reason',
            ),
          ),
        );
      }
    } finally {
      api.dispose();
      if (mounted) await _load();
    }
  }

  Future<void> _rename(AgentHost host, String name) async {
    if (name.trim().isEmpty || name.trim() == host.name) return;
    await _with((api) => api.renameHost(host.id, name.trim()));
    if (mounted) await _load();
  }

  Future<void> _revoke(AgentHost host) async {
    await _with((api) => api.revokeHost(host.id));
    if (mounted) await _load();
  }

  Future<String?> _saveLimits(
    AgentHost host,
    int children,
    int tree,
    int? sessions,
  ) async {
    final api = agentApi(ref);
    if (api == null) return 'Sign in first.';
    try {
      await api.setHostLimits(
        host.id,
        maxChildren: children,
        maxTree: tree,
        maxSessions: sessions,
      );
    } catch (e) {
      return hostLimitsRefusal(e);
    } finally {
      api.dispose();
    }
    if (mounted) await _load();
    return null;
  }

  Future<void> _setDefault(String id) async {
    if (id == _defaultProvider) return;
    final was = _defaultProvider;
    setState(() => _defaultProvider = id);
    final api = agentApi(ref);
    if (api == null) return;
    try {
      await api.setDefaultProvider(id);
    } catch (e) {
      if (mounted) {
        setState(() {
          _defaultProvider = was;
          _error = describeFailure(e);
        });
      }
    } finally {
      api.dispose();
    }
  }

  // ---- installs (decision 133) ----------------------------------------

  String _failure(Object e, String agent) => switch (e) {
    AgentRefusal(statusCode: 503) => 'The host is offline.',
    AgentRefusal(:final code?) => describeAgentJobError(code, agent: agent),
    _ => describeFailure(e),
  };

  Future<void> _start(AgentHost host, String agent, String op) async {
    final key = '${host.id}/$agent';
    final api = agentApi(ref);
    if (api == null) return;
    setState(() => _runs[key] = AgentRun(op: op));
    try {
      final job = await api.startAgentJob(host.id, agent, op);
      if (mounted) _adopt(key, host.id, job);
    } catch (e) {
      if (mounted) {
        setState(
          () => _runs[key] = AgentRun(op: op, error: _failure(e, agent)),
        );
      }
    } finally {
      api.dispose();
    }
  }

  void _adopt(String key, String hostId, AgentJob job) {
    if (job.done) {
      setState(() => _runs.remove(key));
      _load();
      return;
    }
    setState(
      () => _runs[key] = job.failed
          ? AgentRun(
              op: job.op,
              error: describeAgentJobError(job.error, agent: job.agent),
            )
          : AgentRun(op: job.op, step: job.step),
    );
    if (job.failed) {
      _load();
      return;
    }
    _polls[key] = Timer(widget.pollEvery, () => _poll(key, hostId, job));
  }

  Future<void> _poll(String key, String hostId, AgentJob job) async {
    _polls.remove(key);
    final api = agentApi(ref);
    if (api == null) return;
    try {
      final next = await api.agentJob(hostId, job.id);
      if (mounted) _adopt(key, hostId, next);
    } on AgentRefusal catch (e) {
      if (mounted) {
        setState(
          () =>
              _runs[key] = AgentRun(op: job.op, error: _failure(e, job.agent)),
        );
      }
    } catch (_) {
      // The server could not be asked; the job carries on without us.
      if (mounted) {
        _polls[key] = Timer(widget.pollEvery, () => _poll(key, hostId, job));
      }
    } finally {
      api.dispose();
    }
  }

  // ---- Shells on a host (153-S3) --------------------------------------

  /// Opens a Shell on [host], with [typed] typed in when given, and goes to
  /// it. Answers why not, or null once it is open.
  Future<String?> _shell(AgentHost host, {String? typed}) async {
    final api = agentApi(ref);
    if (api == null) return 'Sign in first.';
    try {
      final workspace =
          host.workspaces.firstOrNull ??
          (await api.workspaces(host.id)).firstOrNull?.name;
      if (workspace == null) {
        return '${host.name} has no workspace yet. Start a session there '
            'from New session to make one.';
      }
      if (!mounted) return null;
      final size = MediaQuery.sizeOf(context);
      final session = await api.launch(
        hostId: host.id,
        workspace: workspace,
        provider: 'shell',
        cols: (size.width / 9).clamp(40, 240).floor(),
        rows: (size.height / 20).clamp(12, 80).floor(),
        typedInput: typed,
      );
      if (!mounted) return null;
      _close();
      if (_sheetOpen) Navigator.of(context).pop();
      ref.invalidate(agentOverviewProvider);
      openAgentSession(context, session.id);
      return null;
    } on AgentRefusal catch (e) {
      if (e.statusCode == 409 && e.code == 'host_cannot_type') {
        setState(() => _cannotType.add(host.id));
        return null;
      }
      return describeFailure(e);
    } catch (e) {
      return describeFailure(e);
    } finally {
      api.dispose();
    }
  }

  // ---- the cell popover ----------------------------------------------

  LayerLink _link(String hostId, String agent) =>
      _links.putIfAbsent('$hostId/$agent', LayerLink.new);

  void _openCell(AgentHost host, String agent) {
    if (!context.isExpanded) {
      _openSheet(host, agent);
      return;
    }
    setState(() => _open = (host: host.id, agent: agent));
    _popover.show();
  }

  void _close() {
    if (_open == null) return;
    _popover.hide();
    setState(() => _open = null);
  }

  Future<void> _openSheet(AgentHost host, String agent) async {
    _sheetOpen = true;
    await showStormSheet<void>(
      context: context,
      title: '${providerLabel(agent)} on ${host.name}',
      heightFactor: 0.8,
      builder: (_) => ValueListenableBuilder<int>(
        valueListenable: _tick,
        builder: (context, _, _) {
          final now = _hosts?.where((h) => h.id == host.id).firstOrNull ?? host;
          return SingleChildScrollView(child: _panel(now, agent, sheet: true));
        },
      ),
    );
    _sheetOpen = false;
  }

  Widget _panel(AgentHost host, String agent, {bool sheet = false}) =>
      CellPanel(
        key: ValueKey('panel-${host.id}/$agent'),
        host: host,
        agent: agent,
        run: _runs['${host.id}/$agent'],
        hostBusy: _runs.entries.any(
          (e) => e.value.active && e.key.startsWith('${host.id}/'),
        ),
        canType: host.typedInput && !_cannotType.contains(host.id),
        isDefault: agent == _defaultProvider,
        sheet: sheet,
        onAction: (op) => _start(host, agent, op),
        onSignIn: (command) => _shell(host, typed: command),
        onOpenShell: () => _shell(host),
        onMakeDefault: () => _setDefault(agent),
      );

  Widget _popoverLayer(BuildContext context) {
    final open = _open;
    final host = _hosts?.where((h) => h.id == open?.host).firstOrNull;
    if (open == null || host == null) return const SizedBox();
    final t = context.tokens;
    final column = kInstallableAgents.indexOf(open.agent);
    final right = column >= 3;
    return CallbackShortcuts(
      bindings: {const SingleActivator(LogicalKeyboardKey.escape): _close},
      child: Focus(
        autofocus: true,
        child: Stack(
          children: [
            Positioned.fill(
              child: GestureDetector(
                key: const Key('cell-popover-barrier'),
                behavior: HitTestBehavior.opaque,
                onTap: _close,
              ),
            ),
            Positioned(
              left: 0,
              top: 0,
              child: CompositedTransformFollower(
                link: _link(open.host, open.agent),
                showWhenUnlinked: false,
                targetAnchor: right
                    ? Alignment.bottomRight
                    : Alignment.bottomLeft,
                followerAnchor: right ? Alignment.topRight : Alignment.topLeft,
                offset: Offset(0, t.sp * 0.5),
                child: DecoratedBox(
                  decoration: BoxDecoration(
                    borderRadius: BorderRadius.circular(t.rCard),
                    boxShadow: t.shadow,
                  ),
                  child: Material(
                    key: const Key('cell-popover'),
                    color: t.surface2,
                    shape: RoundedRectangleBorder(
                      borderRadius: BorderRadius.circular(t.rCard),
                      side: BorderSide(color: t.border, width: t.bw),
                    ),
                    child: Container(
                      width: t.sp * 37.5,
                      padding: EdgeInsets.all(t.sp * 2),
                      child: _panel(host, open.agent),
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  // ---- the page -------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final hosts = _hosts;
    final wide = context.isExpanded;
    final selected = hosts?.where((h) => h.id == _selected).firstOrNull;

    final sessions =
        ref.watch(agentOverviewProvider).value?.sessions ??
        const <AgentSession>[];
    Widget details(AgentHost h) => HostDetails(
      key: ValueKey('details-${h.id}'),
      host: h,
      hostAdmin: !_noHostAdmin,
      running: sessions.where((s) => s.hostId == h.id && !s.ended).length,
      onRename: (name) => _rename(h, name),
      onRevoke: () => _revoke(h),
      onRemove: () => _revokeAndRemove(h),
      onSaveLimits: (c, tr, s) => _saveLimits(h, c, tr, s),
    );

    final defaultElsewhere =
        _defaultProvider != null &&
        !kInstallableAgents.contains(_defaultProvider);

    final page = SettingsPage(
      title: 'Agents & hosts',
      intro: 'The machines agents run on, and which agent CLIs each one has.',
      broad: true,
      action: StormButton.soft(
        key: const Key('enroll-host'),
        label: '＋ Enroll a host',
        onPressed: _loading ? null : _enroll,
      ),
      children: [
        if (_error != null) SettingsMuted(_error!, danger: true),
        if (_loading && hosts == null)
          const SkeletonRows(rows: 3)
        else if (hosts != null && hosts.isEmpty)
          NoHostsCard(onEnroll: _loading ? null : _enroll)
        else if (hosts != null && wide) ...[
          AgentsGrid(
            hosts: hosts,
            defaultProvider: _defaultProvider,
            runs: _runs,
            selected: _selected,
            open: _open,
            hostAdmin: !_noHostAdmin,
            link: _link,
            onSelect: (h) => setState(() => _selected = h.id),
            onOpen: _openCell,
            onMakeDefault: _setDefault,
            onReenroll: _reenroll,
            onRemove: _remove,
          ),
          SizedBox(height: t.sp * 1.25),
          Text(
            defaultElsewhere
                ? 'Click a cell to see details or act on it. The default agent '
                      'is ${providerLabel(_defaultProvider!)}; click an '
                      "agent's name to make it the default."
                : "Click a cell to see details or act on it. Click an agent's "
                      'name to make it the default.',
            key: const Key('grid-hint'),
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.labelSize * 1.09,
              color: t.text3,
            ),
          ),
          if (selected != null) ...[
            SizedBox(height: t.sp * 2.5),
            details(selected),
          ],
        ] else if (hosts != null)
          for (final h in hosts)
            HostSection(
              key: Key('host-section-${h.id}'),
              host: h,
              defaultProvider: _defaultProvider,
              runs: _runs,
              expanded: h.id == _selected,
              hostAdmin: !_noHostAdmin,
              details: h.status == 'revoked' ? null : details(h),
              onSelect: () =>
                  setState(() => _selected = _selected == h.id ? null : h.id),
              onOpen: (agent) => _openCell(h, agent),
              onReenroll: () => _reenroll(h),
              onRemove: () => _remove(h),
            ),
      ],
    );
    return OverlayPortal(
      controller: _popover,
      overlayChildBuilder: _popoverLayer,
      child: page,
    );
  }
}

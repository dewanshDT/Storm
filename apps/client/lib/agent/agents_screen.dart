import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import '../ui/breakpoints.dart';
import '../ui/states.dart';
import '../ui/tokens.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'hosts_screen.dart' show ChipTone, StatusChip;
import 'session_controller.dart';
import 'terminal_surface.dart';

/// Agent sessions: the list, the open tabs, and the terminal (freeze §10,
/// items 2–5; decision 77d).
///
/// **The phone layout is the default** (the M12 invariant): one session fills
/// the screen, with a switcher sheet. At [kExpandedWidth] and wider, the list
/// sits beside a tab strip. Nothing below the breakpoint changes because of
/// what renders above it.
class AgentsScreen extends ConsumerStatefulWidget {
  const AgentsScreen({super.key});

  @override
  ConsumerState<AgentsScreen> createState() => _AgentsScreenState();
}

class _AgentsScreenState extends ConsumerState<AgentsScreen> {
  List<AgentSession>? _sessions;
  List<AgentHost> _hosts = const [];
  String? _error;
  bool _loading = false;
  final _controllers = <String, SessionController>{};

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    for (final c in _controllers.values) {
      c.dispose();
    }
    super.dispose();
  }

  Future<void> _load() async {
    final api = agentApi(ref);
    if (api == null) return;
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final sessions = await api.sessions();
      final hosts = await api.hosts();
      if (!mounted) return;
      ref.read(agentTabsProvider.notifier).reconcile(sessions.map((s) => s.id));
      setState(() {
        _sessions = sessions.reversed.toList();
        _hosts = hosts;
      });
    } catch (e) {
      if (mounted) setState(() => _error = describeFailure(e));
    } finally {
      api.dispose();
      if (mounted) setState(() => _loading = false);
    }
  }

  SessionController _controller(String id) => _controllers.putIfAbsent(id, () {
    final api = agentApi(ref)!;
    final open = ref.read(terminalStreamFactoryProvider);
    return SessionController(
      api: api,
      sessionId: id,
      open: open == null ? null : (offset) => open(id, offset),
    )..start();
  });

  void _open(String id) {
    ref.read(agentTabsProvider.notifier).open(id);
    ref.read(activeAgentTabProvider.notifier).state = id;
  }

  void _close(String id) {
    ref.read(agentTabsProvider.notifier).close(id);
    _controllers.remove(id)?.dispose();
    final active = ref.read(activeAgentTabProvider);
    if (active == id) {
      final tabs = ref.read(agentTabsProvider);
      ref.read(activeAgentTabProvider.notifier).state = tabs.isEmpty
          ? null
          : tabs.last;
    }
  }

  String _hostName(String id) => _hosts
      .firstWhere(
        (h) => h.id == id,
        orElse: () => AgentHost(
          id: id,
          name: 'host',
          status: 'offline',
          providers: const [],
          workspaces: const [],
          maxSessions: 0,
          lastSeen: null,
        ),
      )
      .name;

  Future<void> _launch() async {
    final launched = await showModalBottomSheet<AgentSession>(
      context: context,
      isScrollControlled: true,
      showDragHandle: true,
      builder: (_) => _Launcher(hosts: _hosts),
    );
    if (launched == null || !mounted) return;
    final fb = launched.fallback;
    if (fb != null) {
      // Never silent (freeze §6).
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
            "${providerLabel(fb.requested)} isn't installed on "
            '${_hostName(launched.hostId)}. Using ${providerLabel(fb.used)}.',
          ),
        ),
      );
    }
    _open(launched.id);
    await _load();
  }

  Future<void> _dismiss(AgentSession s) async {
    final api = agentApi(ref);
    if (api == null) return;
    try {
      await api.dismiss(s.id);
      _close(s.id);
      await _load();
    } catch (e) {
      if (mounted) setState(() => _error = describeFailure(e));
    } finally {
      api.dispose();
    }
  }

  @override
  Widget build(BuildContext context) {
    final tabs = ref.watch(agentTabsProvider);
    final active = ref.watch(activeAgentTabProvider);
    final wide = context.isExpanded;
    final current = active != null && tabs.contains(active) ? active : null;

    final actions = [
      IconButton(
        tooltip: 'Hosts',
        onPressed: () => context.push(Routes.agentHosts).then((_) => _load()),
        icon: const Icon(LucideIcons.server, size: 18),
      ),
      IconButton(
        key: const Key('new-session'),
        tooltip: 'New session',
        onPressed: _launch,
        icon: const Icon(LucideIcons.plus, size: 18),
      ),
    ];

    if (!wide && current != null) {
      // Phone, a session open: it fills the screen.
      return _SessionPage(
        controller: _controller(current),
        hostName: (id) => _hostName(id),
        tabCount: tabs.length,
        onSwitch: () => _showSwitcher(tabs),
        onBack: () => ref.read(activeAgentTabProvider.notifier).state = null,
        onDismiss: _dismiss,
      );
    }

    final list = _SessionList(
      sessions: _sessions,
      loading: _loading,
      error: _error,
      hostName: _hostName,
      hasHosts: _hosts.isNotEmpty,
      selected: current,
      onOpen: _open,
      onRefresh: _load,
      onLaunch: _launch,
      onHosts: () => context.push(Routes.agentHosts).then((_) => _load()),
    );

    if (!wide) {
      // On a phone the primary action is under the thumb, as on Hosts.
      return Scaffold(
        appBar: AppBar(title: const Text('Agents'), actions: [actions.first]),
        body: list,
        floatingActionButton: FloatingActionButton.extended(
          key: const Key('new-session-fab'),
          onPressed: _launch,
          icon: const Icon(LucideIcons.plus),
          label: const Text('New session'),
        ),
      );
    }

    final t = context.tokens;
    return Scaffold(
      appBar: AppBar(title: const Text('Agents'), actions: actions),
      body: Row(
        children: [
          SizedBox(width: 300, child: list),
          VerticalDivider(width: t.bw, color: t.border),
          Expanded(
            child: current == null
                ? const EmptyState(
                    icon: LucideIcons.square_terminal,
                    title: 'No session open',
                    detail: 'Open one from the list, or start a new one.',
                    fill: true,
                  )
                : Column(
                    children: [
                      _TabStrip(
                        tabs: tabs,
                        active: current,
                        sessions: _sessions ?? const [],
                        onSelect: (id) =>
                            ref.read(activeAgentTabProvider.notifier).state =
                                id,
                        onClose: _close,
                      ),
                      Expanded(
                        child: _SessionView(
                          key: ValueKey(current),
                          controller: _controller(current),
                          hostName: _hostName,
                          onDismiss: _dismiss,
                          keysRow: false,
                        ),
                      ),
                    ],
                  ),
          ),
        ],
      ),
    );
  }

  Future<void> _showSwitcher(List<String> tabs) async {
    final picked = await showModalBottomSheet<String>(
      context: context,
      showDragHandle: true,
      builder: (context) {
        final byId = {for (final s in _sessions ?? <AgentSession>[]) s.id: s};
        return SafeArea(
          child: ListView(
            shrinkWrap: true,
            children: [
              for (final id in tabs)
                ListTile(
                  leading: const Icon(LucideIcons.square_terminal, size: 18),
                  title: Text(_title(byId[id])),
                  subtitle: Text(byId[id]?.statusLabel ?? ''),
                  onTap: () => Navigator.of(context).pop(id),
                ),
              ListTile(
                leading: const Icon(LucideIcons.list, size: 18),
                title: const Text('All sessions'),
                onTap: () => Navigator.of(context).pop(''),
              ),
            ],
          ),
        );
      },
    );
    if (picked == null) return;
    ref.read(activeAgentTabProvider.notifier).state = picked.isEmpty
        ? null
        : picked;
  }
}

String _title(AgentSession? s) =>
    s == null ? 'Session' : '${providerLabel(s.provider)} in ${s.workspace}';

/// How long ago, coarsely: a list is scanned, not read.
String _ago(String iso) {
  final at = DateTime.tryParse(iso);
  if (at == null) return '';
  final d = DateTime.now().toUtc().difference(at.toUtc());
  if (d.inMinutes < 1) return 'just now';
  if (d.inHours < 1) return '${d.inMinutes} min ago';
  if (d.inDays < 1) return '${d.inHours} h ago';
  return '${d.inDays} d ago';
}

ChipTone _tone(AgentSession? s) => switch (s?.status) {
  'running' => ChipTone.good,
  'starting' || 'creating' || 'unknown' => ChipTone.warn,
  'failed' => ChipTone.bad,
  _ => ChipTone.muted,
};

class _SessionList extends StatelessWidget {
  const _SessionList({
    required this.sessions,
    required this.loading,
    required this.error,
    required this.hostName,
    required this.hasHosts,
    required this.selected,
    required this.onOpen,
    required this.onRefresh,
    required this.onLaunch,
    required this.onHosts,
  });

  final List<AgentSession>? sessions;
  final bool loading;
  final String? error;
  final String Function(String) hostName;
  final bool hasHosts;
  final String? selected;
  final ValueChanged<String> onOpen;
  final Future<void> Function() onRefresh;
  final VoidCallback onLaunch;
  final VoidCallback onHosts;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final list = sessions;
    return RefreshIndicator(
      onRefresh: onRefresh,
      child: ListView(
        padding: EdgeInsets.all(t.sp),
        children: [
          if (error != null)
            Padding(
              padding: EdgeInsets.all(t.sp),
              child: Text(error!, style: TextStyle(color: t.danger)),
            ),
          if (loading && list == null)
            const SkeletonRows(rows: 4)
          else if (list != null && list.isEmpty)
            hasHosts
                ? EmptyState(
                    icon: LucideIcons.square_terminal,
                    title: 'No sessions yet',
                    detail:
                        'Start an agent in a workspace on one of your hosts.',
                    action: 'New session',
                    onAction: onLaunch,
                  )
                : EmptyState(
                    icon: LucideIcons.server,
                    title: 'No hosts yet',
                    detail: 'Agents run on a host. Enroll one to get started.',
                    action: 'Hosts',
                    onAction: onHosts,
                  )
          else
            for (final s in list ?? <AgentSession>[])
              ListTile(
                key: Key('session-${s.id}'),
                selected: s.id == selected,
                contentPadding: EdgeInsets.symmetric(horizontal: t.sp),
                shape: RoundedRectangleBorder(
                  borderRadius: BorderRadius.circular(t.rControl),
                ),
                title: Text(_title(s)),
                subtitle: Text('${hostName(s.hostId)} · ${_ago(s.createdAt)}'),
                trailing: StatusChip(label: s.statusLabel, tone: _tone(s)),
                onTap: () => onOpen(s.id),
              ),
        ],
      ),
    );
  }
}

class _TabStrip extends StatelessWidget {
  const _TabStrip({
    required this.tabs,
    required this.active,
    required this.sessions,
    required this.onSelect,
    required this.onClose,
  });

  final List<String> tabs;
  final String active;
  final List<AgentSession> sessions;
  final ValueChanged<String> onSelect;
  final ValueChanged<String> onClose;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final byId = {for (final s in sessions) s.id: s};
    return Container(
      height: 40,
      decoration: BoxDecoration(
        border: Border(
          bottom: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      child: ListView(
        scrollDirection: Axis.horizontal,
        children: [
          for (final id in tabs)
            InkWell(
              key: Key('tab-$id'),
              onTap: () => onSelect(id),
              child: Container(
                padding: EdgeInsets.symmetric(horizontal: t.sp * 1.5),
                decoration: BoxDecoration(
                  border: Border(
                    bottom: BorderSide(
                      color: id == active ? t.accent : Colors.transparent,
                      width: 2,
                    ),
                  ),
                ),
                child: Row(
                  children: [
                    Text(
                      _title(byId[id]),
                      style: TextStyle(
                        color: id == active ? t.text : t.text2,
                        fontWeight: id == active
                            ? FontWeight.w600
                            : FontWeight.w400,
                      ),
                    ),
                    IconButton(
                      tooltip: 'Close tab',
                      visualDensity: VisualDensity.compact,
                      iconSize: 14,
                      onPressed: () => onClose(id),
                      icon: const Icon(LucideIcons.x),
                    ),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// A session alone on the phone screen.
class _SessionPage extends StatelessWidget {
  const _SessionPage({
    required this.controller,
    required this.hostName,
    required this.tabCount,
    required this.onSwitch,
    required this.onBack,
    required this.onDismiss,
  });

  final SessionController controller;
  final String Function(String) hostName;
  final int tabCount;
  final VoidCallback onSwitch;
  final VoidCallback onBack;
  final ValueChanged<AgentSession> onDismiss;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: controller,
      builder: (context, _) => Scaffold(
        appBar: AppBar(
          leading: IconButton(
            tooltip: 'All sessions',
            onPressed: onBack,
            icon: const Icon(LucideIcons.arrow_left),
          ),
          title: Text(_title(controller.session)),
          actions: [
            IconButton(
              key: const Key('switch-session'),
              tooltip: 'Switch session',
              onPressed: onSwitch,
              icon: Badge(
                label: Text('$tabCount'),
                isLabelVisible: tabCount > 1,
                child: const Icon(LucideIcons.layers, size: 18),
              ),
            ),
          ],
        ),
        body: _SessionView(
          controller: controller,
          hostName: hostName,
          onDismiss: onDismiss,
          keysRow: true,
        ),
      ),
    );
  }
}

class _SessionView extends StatefulWidget {
  const _SessionView({
    super.key,
    required this.controller,
    required this.hostName,
    required this.onDismiss,
    required this.keysRow,
  });

  final SessionController controller;
  final String Function(String) hostName;
  final ValueChanged<AgentSession> onDismiss;

  /// The phone's extra-keys row (freeze §10).
  final bool keysRow;

  @override
  State<_SessionView> createState() => _SessionViewState();
}

class _SessionViewState extends State<_SessionView> {
  final _focus = FocusNode();

  @override
  void initState() {
    super.initState();
    _focus.addListener(() {
      if (_focus.hasFocus) widget.controller.focus();
    });
  }

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  Future<void> _confirmEnd() async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('End this session?'),
        content: const Text(
          'The agent and everything it started are stopped on the host.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Keep running'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('End session'),
          ),
        ],
      ),
    );
    if (confirmed == true) {
      try {
        await widget.controller.end();
      } catch (e) {
        if (mounted) {
          ScaffoldMessenger.of(
            context,
          ).showSnackBar(SnackBar(content: Text(describeFailure(e))));
        }
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return ListenableBuilder(
      listenable: widget.controller,
      builder: (context, _) {
        final c = widget.controller;
        final s = c.session;
        return Column(
          children: [
            Padding(
              padding: EdgeInsets.symmetric(
                horizontal: t.sp * 1.5,
                vertical: t.sp * 0.75,
              ),
              child: Row(
                children: [
                  StatusChip(
                    label: s?.statusLabel ?? 'Connecting',
                    tone: _tone(s),
                  ),
                  SizedBox(width: t.sp),
                  Expanded(
                    child: Text(
                      s == null ? '' : 'on ${widget.hostName(s.hostId)}',
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(fontSize: t.labelSize, color: t.text3),
                    ),
                  ),
                  if (s != null && !s.ended)
                    TextButton(
                      key: const Key('end-session'),
                      onPressed: _confirmEnd,
                      child: const Text('End'),
                    ),
                  if (s != null && s.ended)
                    TextButton(
                      onPressed: () => widget.onDismiss(s),
                      child: const Text('Dismiss'),
                    ),
                ],
              ),
            ),
            if (c.inputError != null || (c.streamError != null && !c.connected))
              Container(
                width: double.infinity,
                padding: EdgeInsets.symmetric(
                  horizontal: t.sp * 1.5,
                  vertical: t.sp * 0.5,
                ),
                color: t.amberSoft,
                child: Text(
                  c.inputError ?? 'Reconnecting: ${c.streamError}',
                  style: TextStyle(fontSize: t.labelSize, color: t.text),
                ),
              ),
            Expanded(
              child: StormTerminalView(
                terminal: c.terminal,
                focusNode: _focus,
                autofocus: !widget.keysRow,
                readOnly: s?.ended ?? false,
              ),
            ),
            if (widget.keysRow && !(s?.ended ?? false))
              _ExtraKeys(terminal: c.terminal),
          ],
        );
      },
    );
  }
}

/// Esc, Tab, sticky Ctrl, arrows and Paste: the keys a phone keyboard lacks
/// (freeze §10, AC-F4).
class _ExtraKeys extends StatefulWidget {
  const _ExtraKeys({required this.terminal});

  final StormTerminal terminal;

  @override
  State<_ExtraKeys> createState() => _ExtraKeysState();
}

class _ExtraKeysState extends State<_ExtraKeys> {
  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final term = widget.terminal;
    // Equal keys across the full width: eight 44 px targets fit a 360 px
    // phone, so nothing scrolls out of view. Arrows and paste are icons,
    // because the mono face draws the arrow glyphs at uneven sizes.
    Widget key(
      Widget label,
      VoidCallback onTap, {
      bool on = false,
      Key? k,
      String? tip,
    }) => Expanded(
      child: Padding(
        padding: EdgeInsets.symmetric(horizontal: t.sp * 0.2),
        child: Tooltip(
          message: tip ?? '',
          child: OutlinedButton(
            key: k,
            style: OutlinedButton.styleFrom(
              minimumSize: const Size(0, 44),
              padding: EdgeInsets.zero,
              backgroundColor: on ? t.accentSoft : null,
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(t.rControl),
              ),
            ),
            onPressed: onTap,
            child: label,
          ),
        ),
      ),
    );
    Text word(String w) => Text(
      w,
      style: TextStyle(
        fontFamily: StormTokens.monoFamily,
        fontSize: t.labelSize,
      ),
    );
    const size = 16.0;
    return SafeArea(
      top: false,
      child: Container(
        padding: EdgeInsets.symmetric(
          vertical: t.sp * 0.5,
          horizontal: t.sp * 0.3,
        ),
        decoration: BoxDecoration(
          color: t.surface,
          border: Border(
            top: BorderSide(color: t.border, width: t.bw),
          ),
        ),
        child: Row(
          children: [
            key(word('Esc'), term.escape, k: const Key('key-esc')),
            key(word('Tab'), term.tab, k: const Key('key-tab')),
            key(
              word('Ctrl'),
              () => setState(() => term.stickyCtrl = !term.stickyCtrl),
              on: term.stickyCtrl,
              k: const Key('key-ctrl'),
            ),
            key(
              const Icon(LucideIcons.arrow_up, size: size),
              term.up,
              tip: 'Up',
            ),
            key(
              const Icon(LucideIcons.arrow_down, size: size),
              term.down,
              tip: 'Down',
            ),
            key(
              const Icon(LucideIcons.arrow_left, size: size),
              term.left,
              tip: 'Left',
            ),
            key(
              const Icon(LucideIcons.arrow_right, size: size),
              term.right,
              tip: 'Right',
            ),
            key(
              const Icon(LucideIcons.clipboard_paste, size: size),
              () async {
                final data = await Clipboard.getData(Clipboard.kTextPlain);
                final text = data?.text;
                if (text != null && text.isNotEmpty) term.paste(text);
              },
              k: const Key('key-paste'),
              tip: 'Paste',
            ),
          ],
        ),
      ),
    );
  }
}

/// Host, then workspace, then provider (freeze §10, item 2).
class _Launcher extends ConsumerStatefulWidget {
  const _Launcher({required this.hosts});

  final List<AgentHost> hosts;

  @override
  ConsumerState<_Launcher> createState() => _LauncherState();
}

class _LauncherState extends ConsumerState<_Launcher> {
  AgentHost? _host;
  List<AgentWorkspace>? _workspaces;
  AgentWorkspace? _workspace;
  String? _provider;
  String? _default;
  String? _error;
  bool _busy = false;

  List<AgentHost> get _online => [
    for (final h in widget.hosts)
      if (h.online) h,
  ];

  @override
  void initState() {
    super.initState();
    final online = _online;
    if (online.length == 1) _pickHost(online.first);
    _loadDefault();
  }

  Future<void> _loadDefault() async {
    final api = agentApi(ref);
    if (api == null) return;
    try {
      final d = await api.defaultProvider();
      if (mounted) setState(() => _default = _provider ??= d);
    } catch (_) {
    } finally {
      api.dispose();
    }
  }

  Future<void> _pickHost(AgentHost host) async {
    setState(() {
      _host = host;
      _workspaces = null;
      _workspace = null;
    });
    final api = agentApi(ref);
    if (api == null) return;
    try {
      final ws = await api.workspaces(host.id);
      if (mounted) setState(() => _workspaces = ws);
    } catch (e) {
      if (mounted) setState(() => _error = describeFailure(e));
    } finally {
      api.dispose();
    }
  }

  Future<void> _go() async {
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
        workspace: ws.name,
        provider: _provider,
        // A first guess; the terminal sends its real size once laid out.
        cols: (size.width / 9).clamp(40, 240).floor(),
        rows: (size.height / 20).clamp(12, 80).floor(),
      );
      if (mounted) Navigator.of(context).pop(session);
    } catch (e) {
      if (mounted) setState(() => _error = describeFailure(e));
    } finally {
      api.dispose();
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final online = _online;
    final host = _host;
    return SafeArea(
      child: Padding(
        padding: EdgeInsets.fromLTRB(
          t.sp * 2,
          0,
          t.sp * 2,
          t.sp * 2 + MediaQuery.viewInsetsOf(context).bottom,
        ),
        child: SingleChildScrollView(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                'New session',
                style: Theme.of(context).textTheme.titleLarge,
              ),
              SizedBox(height: t.sp * 2),
              _Label('Host'),
              if (online.isEmpty)
                Text(
                  'No host is online. Start storm-runtime on one, or enroll a '
                  'host first.',
                  style: TextStyle(color: t.text3),
                )
              else
                Wrap(
                  spacing: t.sp,
                  runSpacing: t.sp,
                  children: [
                    for (final h in online)
                      ChoiceChip(
                        label: Text(h.name),
                        selected: h.id == host?.id,
                        onSelected: (_) => _pickHost(h),
                      ),
                  ],
                ),
              if (host != null) ...[
                SizedBox(height: t.sp * 2),
                _Label('Workspace'),
                if (_workspaces == null)
                  const SkeletonRows(rows: 1)
                else if (_workspaces!.isEmpty)
                  Text(
                    'This host has no workspaces. Add a directory under one of '
                    'its workspace roots.',
                    style: TextStyle(color: t.text3),
                  )
                else
                  Wrap(
                    spacing: t.sp,
                    runSpacing: t.sp,
                    children: [
                      for (final w in _workspaces!)
                        ChoiceChip(
                          key: Key('workspace-${w.name}'),
                          label: Text(w.name),
                          selected: w.name == _workspace?.name,
                          onSelected: (_) => setState(() => _workspace = w),
                        ),
                    ],
                  ),
                if ((_workspace?.liveSessions ?? 0) > 0)
                  Padding(
                    padding: EdgeInsets.only(top: t.sp),
                    child: Text(
                      _workspace!.liveSessions == 1
                          ? 'Another session is already working here, on the '
                                'same files.'
                          : '${_workspace!.liveSessions} sessions are already '
                                'working here, on the same files.',
                      style: TextStyle(color: t.amber),
                    ),
                  ),
                SizedBox(height: t.sp * 2),
                _Label('Provider'),
                Wrap(
                  spacing: t.sp,
                  runSpacing: t.sp,
                  children: [
                    for (final p in host.providers)
                      ChoiceChip(
                        key: Key('provider-${p.id}'),
                        label: Text(
                          !p.available
                              ? '${p.label} (not installed)'
                              : p.id == _default
                              ? '${p.label} (default)'
                              : p.label,
                        ),
                        selected: p.id == _provider,
                        onSelected: p.available
                            ? (_) => setState(() => _provider = p.id)
                            : null,
                      ),
                  ],
                ),
                SizedBox(height: t.sp * 2),
                Text(
                  "Network: inherits ${host.name}'s policy",
                  style: TextStyle(fontSize: t.labelSize, color: t.text3),
                ),
              ],
              if (_error != null) ...[
                SizedBox(height: t.sp),
                Text(_error!, style: TextStyle(color: t.danger)),
              ],
              SizedBox(height: t.sp * 2),
              Align(
                alignment: Alignment.centerRight,
                child: FilledButton(
                  key: const Key('launch'),
                  onPressed: _busy || host == null || _workspace == null
                      ? null
                      : _go,
                  child: Text(_busy ? 'Starting' : 'Launch'),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _Label extends StatelessWidget {
  const _Label(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.only(bottom: t.sp * 0.75),
      child: Text(
        text,
        style: TextStyle(fontWeight: FontWeight.w600, color: t.text2),
      ),
    );
  }
}

/// The launcher, for tests.
@visibleForTesting
Widget launcherForTest(List<AgentHost> hosts) => _Launcher(hosts: hosts);

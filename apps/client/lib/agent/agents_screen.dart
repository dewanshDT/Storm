import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import '../ui/breakpoints.dart';
import '../ui/shell/nav_bubble.dart' show keyboardIsOpen;
import '../ui/shell/storm_scaffold.dart' show StormChrome;
import '../ui/states.dart';
import '../ui/tokens.dart';
import '../ui/widgets.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart';
import 'hosts_screen.dart' show ChipTone, StatusChip;
import 'session_controller.dart';
import 'terminal_surface.dart';

/// Agent sessions: the list, the open tabs, and the terminal (freeze §10,
/// items 2–5; decisions 77d and 78).
///
/// **The phone layout is the default** (the M12 invariant): the list, with
/// *New session* as a pill at the bottom; a session fills the screen, with a
/// switcher sheet. At [kExpandedWidth] and wider this is only the *pane* — the
/// list lives in `AgentsSidebar`, beside it in `AgentsShell` — and shows the
/// tab strip over the open session. Nothing below the breakpoint changes
/// because of what renders above it.
class AgentsScreen extends ConsumerStatefulWidget {
  const AgentsScreen({super.key});

  @override
  ConsumerState<AgentsScreen> createState() => _AgentsScreenState();
}

class _AgentsScreenState extends ConsumerState<AgentsScreen> {
  final _controllers = <String, SessionController>{};

  @override
  void dispose() {
    for (final c in _controllers.values) {
      c.dispose();
    }
    super.dispose();
  }

  SessionController _controller(String id) => _controllers.putIfAbsent(id, () {
    final api = agentApi(ref)!;
    final open = ref.read(terminalStreamFactoryProvider);
    final c = SessionController(
      api: api,
      sessionId: id,
      open: open == null ? null : (offset) => open(id, offset),
    )..start();
    // The name the agent gives the conversation, for every list that shows
    // this session.
    c.terminal.title.addListener(() {
      final name = agentChosenTitle(c.terminal.title.value);
      if (!mounted) return;
      final titles = ref.read(agentTitlesProvider);
      if (titles[id] == name) return;
      final next = {...titles};
      name == null ? next.remove(id) : next[id] = name;
      ref.read(agentTitlesProvider.notifier).state = next;
    });
    return c;
  });

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

  Future<void> _dismiss(AgentSession s) async {
    final api = agentApi(ref);
    if (api == null) return;
    try {
      await api.dismiss(s.id);
      _close(s.id);
      await reloadAgents(ref);
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(describeFailure(e))));
      }
    } finally {
      api.dispose();
    }
  }

  @override
  Widget build(BuildContext context) {
    // A dismissed session drops off the strip as soon as the list says so.
    ref.listen(agentOverviewProvider, (_, next) {
      final o = next.value;
      if (o != null && !o.unreachable) {
        ref
            .read(agentTabsProvider.notifier)
            .reconcile(o.sessions.map((s) => s.id));
      }
    });
    final overview =
        ref.watch(agentOverviewProvider).value ??
        const AgentOverview(sessions: [], hosts: []);
    final tabs = ref.watch(agentTabsProvider);
    final active = ref.watch(activeAgentTabProvider);
    final current = active != null && tabs.contains(active) ? active : null;

    if (context.isExpanded) return _pane(overview, tabs, current);

    if (current != null) {
      // Phone, a session open: it fills the screen. Back returns to the
      // list, as the arrow in its bar does, rather than leaving the space —
      // on Android the system back was the one way out that skipped it.
      return PopScope(
        canPop: false,
        onPopInvokedWithResult: (didPop, _) {
          if (!didPop) ref.read(activeAgentTabProvider.notifier).state = null;
        },
        child: _SessionPage(
          controller: _controller(current),
          hostName: overview.hostName,
          tabCount: tabs.length,
          onSwitch: () => _showSwitcher(tabs, overview),
          onBack: () => ref.read(activeAgentTabProvider.notifier).state = null,
          onDismiss: _dismiss,
        ),
      );
    }

    return Scaffold(
      body: StormChrome(
        showNav: false,
        header: Padding(
          padding: EdgeInsets.symmetric(
            horizontal: StormChrome.contentInset(context),
          ),
          child: Text(
            'Agents',
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: context.tokens.titleSize,
              fontWeight: FontWeight.w600,
              color: context.tokens.text,
            ),
          ),
        ),
        child: Stack(
          children: [
            Positioned.fill(
              child: AgentSessionList(
                onOpen: (id) => openAgentSession(ref, id),
                onLaunch: () => launchAgentSession(context, ref),
                onHosts: () => context.go(Routes.settingsPage('hosts')),
                bottomClearance: StormChrome.navClearance(context),
              ),
            ),
            // Under the thumb, in the nav bubble's grammar.
            Positioned(
              left: 0,
              right: 0,
              bottom: 0,
              child: NewSessionPill(
                onTap: () => launchAgentSession(context, ref),
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// The wide pane: the tab strip over the open session, or a quiet prompt.
  Widget _pane(AgentOverview overview, List<String> tabs, String? current) {
    final t = context.tokens;
    return Scaffold(
      backgroundColor: t.bg,
      body: SafeArea(
        left: false,
        child: current == null
            ? EmptyState(
                icon: LucideIcons.square_terminal,
                title: 'No session open',
                detail: 'Open one from the list, or start a new one.',
                action: overview.online.isEmpty ? null : 'New session',
                onAction: () => launchAgentSession(context, ref),
                fill: true,
              )
            : Column(
                children: [
                  _TabStrip(
                    tabs: tabs,
                    active: current,
                    sessions: overview.sessions,
                    onSelect: (id) =>
                        ref.read(activeAgentTabProvider.notifier).state = id,
                    onClose: _close,
                  ),
                  Expanded(
                    child: _SessionView(
                      key: ValueKey(current),
                      controller: _controller(current),
                      hostName: overview.hostName,
                      onDismiss: _dismiss,
                      keysRow: false,
                    ),
                  ),
                ],
              ),
      ),
    );
  }

  Future<void> _showSwitcher(List<String> tabs, AgentOverview overview) async {
    final picked = await showModalBottomSheet<String>(
      context: context,
      showDragHandle: true,
      builder: (context) {
        final byId = {for (final s in overview.sessions) s.id: s};
        final titles = ref.read(agentTitlesProvider);
        return SafeArea(
          child: ListView(
            shrinkWrap: true,
            children: [
              for (final id in tabs)
                ListTile(
                  leading: Icon(
                    providerIcon(byId[id]?.provider ?? ''),
                    size: 18,
                  ),
                  title: Text(sessionDisplayTitle(byId[id], titles)),
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

/// The launcher sheet, and everything that follows a launch: the fallback
/// announced (never silent, freeze §6), the session opened as the active tab,
/// the list refreshed. Returns the session, or null if nothing was launched.
Future<AgentSession?> launchAgentSession(
  BuildContext context,
  WidgetRef ref,
) async {
  final overview = ref.read(agentOverviewProvider).value;
  final launched = await showModalBottomSheet<AgentSession>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (_) => _Launcher(hosts: overview?.hosts ?? const []),
  );
  if (launched == null) return null;
  final fb = launched.fallback;
  if (fb != null && context.mounted) {
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(
          "${providerLabel(fb.requested)} isn't installed on "
          '${overview?.hostName(launched.hostId) ?? 'that host'}. '
          'Using ${providerLabel(fb.used)}.',
        ),
      ),
    );
  }
  openAgentSession(ref, launched.id);
  ref.invalidate(agentOverviewProvider);
  return launched;
}

ChipTone _tone(AgentSession? s) => switch (s?.status) {
  'running' => ChipTone.good,
  'starting' || 'creating' || 'unknown' => ChipTone.warn,
  'failed' => ChipTone.bad,
  _ => ChipTone.muted,
};

/// Every session, live ones first, as the phone screen and the wide sidebar
/// both list them.
class AgentSessionList extends ConsumerWidget {
  const AgentSessionList({
    super.key,
    required this.onOpen,
    required this.onLaunch,
    required this.onHosts,
    this.dense = false,
    this.selected,
    this.bottomClearance = 0,
  });

  final ValueChanged<String> onOpen;
  final VoidCallback onLaunch;
  final VoidCallback onHosts;
  final bool dense;
  final String? selected;

  /// Room left under the last row for whatever floats over the list.
  final double bottomClearance;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final o = ref.watch(agentOverviewProvider).value;

    final List<Widget> children;
    if (o == null) {
      children = const [SkeletonRows(rows: 4)];
    } else if (o.unreachable) {
      // Offline is a state (the ground rule), and agents cannot run without
      // the server — so say exactly that, calmly, with no failure text.
      children = [
        EmptyState(
          icon: LucideIcons.cloud_off,
          title: kAgentsOfflineTitle,
          detail: kAgentsOfflineDetail,
          action: 'Try again',
          onAction: () => reloadAgents(ref),
        ),
      ];
    } else if (o.sessions.isEmpty) {
      children = [
        o.hosts.isEmpty
            ? EmptyState(
                icon: LucideIcons.server,
                title: 'No hosts yet',
                detail: 'Agents run on a host. Enroll one to get started.',
                action: 'Hosts',
                onAction: onHosts,
              )
            : EmptyState(
                icon: LucideIcons.square_terminal,
                title: 'No sessions yet',
                detail: 'Start an agent in a workspace on one of your hosts.',
                action: o.online.isEmpty ? null : 'New session',
                onAction: onLaunch,
              ),
      ];
    } else {
      Widget section(String label, List<AgentSession> list) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: EdgeInsets.fromLTRB(
              t.sp * (dense ? 1 : 0.5),
              t.sp,
              0,
              t.sp,
            ),
            child: SectionLabel(label),
          ),
          for (final s in list)
            AgentSessionRow(
              key: Key('session-${s.id}'),
              session: s,
              hostName: o.hostName(s.hostId),
              selected: s.id == selected,
              dense: dense,
              onTap: () => onOpen(s.id),
            ),
        ],
      );
      children = [
        if (o.live.isNotEmpty) section('Running', o.live),
        if (o.ended.isNotEmpty) section('Ended', o.ended),
      ];
    }

    return RefreshIndicator(
      onRefresh: () => reloadAgents(ref),
      child: ListView(
        padding: EdgeInsets.fromLTRB(
          t.sp * (dense ? 1 : 2),
          t.sp * (dense ? 0.5 : 1),
          t.sp * (dense ? 1 : 2),
          t.sp + bottomClearance,
        ),
        children: children,
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
                    Consumer(
                      builder: (context, ref, _) => Text(
                        sessionDisplayTitle(
                          byId[id],
                          ref.watch(agentTitlesProvider),
                        ),
                        style: TextStyle(
                          color: id == active ? t.text : t.text2,
                          fontWeight: id == active
                              ? FontWeight.w600
                              : FontWeight.w400,
                        ),
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
    // Asked here, above the Scaffold, where the inset is still readable; the
    // note screen does the same for its formatting bar. See keyboardIsOpen.
    final keyboard = keyboardIsOpen(context);
    return ListenableBuilder(
      listenable: controller,
      builder: (context, _) => Scaffold(
        appBar: AppBar(
          leading: IconButton(
            tooltip: 'All sessions',
            onPressed: onBack,
            icon: const Icon(LucideIcons.arrow_left),
          ),
          title: Consumer(
            builder: (context, ref, _) => Text(
              sessionDisplayTitle(
                controller.session,
                ref.watch(agentTitlesProvider),
              ),
              overflow: TextOverflow.ellipsis,
            ),
          ),
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
          keysRow: keyboard,
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

  /// The phone's extra-keys row (freeze §10). Shown only while the
  /// on-screen keyboard is up, like the note editor's formatting bar: the
  /// keys stand in for ones that keyboard lacks, so without it they only
  /// take room from the terminal.
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
        // The phone gives the terminal every pixel it can: a tight status
        // row whose edges line up with the app bar's back arrow and icons,
        // and a compact End. Wide keeps its roomier row (decision 78's pass).
        final phone = !context.isExpanded;
        final rowButton = phone
            ? TextButton.styleFrom(
                minimumSize: const Size(0, 32),
                tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                padding: EdgeInsets.symmetric(horizontal: t.sp),
                visualDensity: VisualDensity.compact,
              )
            : null;
        return Column(
          children: [
            Padding(
              padding: phone
                  // 16 px on the left meets the back arrow; the button's own
                  // `sp` of padding puts its label 16 px from the right too.
                  ? EdgeInsets.fromLTRB(
                      t.sp * 2,
                      t.sp * 0.25,
                      t.sp,
                      t.sp * 0.25,
                    )
                  : EdgeInsets.symmetric(
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
                      style: rowButton,
                      onPressed: _confirmEnd,
                      child: const Text('End'),
                    ),
                  if (s != null && s.ended)
                    TextButton(
                      style: rowButton,
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
                // Not on the phone: focus opens the keyboard there, which
                // should wait for a tap on the terminal.
                autofocus: context.isExpanded,
                readOnly: s?.ended ?? false,
                // Edge to edge on the phone, as phone terminals are: a
                // full-screen agent paints its own background, and the
                // padding would frame it in the page colour.
                padding: phone
                    ? EdgeInsets.symmetric(vertical: t.sp * 0.5)
                    : null,
              ),
            ),
            if (widget.keysRow && !(s?.ended ?? false))
              _ExtraKeys(terminal: c.terminal, onDone: _focus.unfocus),
          ],
        );
      },
    );
  }
}

/// Esc, Tab, sticky Ctrl and Shift, arrows and Paste: the keys a phone
/// keyboard lacks (freeze §10, AC-F4).
///
/// Drawn like the note editor's formatting bar (`EditorToolbar`), so the two
/// rows that ride on the keyboard look like one family: same surface, rule,
/// height, quiet buttons, and a Done that puts the keyboard away.
class _ExtraKeys extends StatelessWidget {
  const _ExtraKeys({required this.terminal, required this.onDone});

  final StormTerminal terminal;

  /// Puts the keyboard away, which also hides this row.
  final VoidCallback onDone;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final term = terminal;
    return Material(
      color: t.surface,
      child: DecoratedBox(
        decoration: BoxDecoration(
          border: Border(
            top: BorderSide(color: t.border, width: t.bw),
          ),
        ),
        child: SafeArea(
          top: false,
          child: SizedBox(
            height: t.sp * 6.5,
            // Armed modifiers light up, and go dark again when the next key
            // consumes them, typed or tapped.
            child: ListenableBuilder(
              listenable: Listenable.merge([term.ctrlArmed, term.shiftArmed]),
              builder: (context, _) => Row(
                children: [
                  Expanded(
                    // Scrolls rather than squeezes, as the editor bar does.
                    child: ListView(
                      scrollDirection: Axis.horizontal,
                      padding: EdgeInsets.symmetric(horizontal: t.sp * 0.75),
                      children: [
                        _Key(
                          key: const Key('key-esc'),
                          label: 'Esc',
                          tooltip: 'Escape',
                          onTap: term.escape,
                        ),
                        _Key(
                          key: const Key('key-tab'),
                          label: 'Tab',
                          tooltip: 'Tab',
                          onTap: term.tab,
                        ),
                        _Key(
                          key: const Key('key-ctrl'),
                          label: 'Ctrl',
                          tooltip: 'Ctrl, for the next key',
                          active: term.stickyCtrl,
                          onTap: () => term.stickyCtrl = !term.stickyCtrl,
                        ),
                        _Key(
                          key: const Key('key-shift'),
                          icon: LucideIcons.arrow_big_up,
                          tooltip: 'Shift, for the next key',
                          active: term.stickyShift,
                          onTap: () => term.stickyShift = !term.stickyShift,
                        ),
                        _Key(
                          icon: LucideIcons.arrow_up,
                          tooltip: 'Up',
                          onTap: term.up,
                        ),
                        _Key(
                          icon: LucideIcons.arrow_down,
                          tooltip: 'Down',
                          onTap: term.down,
                        ),
                        _Key(
                          icon: LucideIcons.arrow_left,
                          tooltip: 'Left',
                          onTap: term.left,
                        ),
                        _Key(
                          icon: LucideIcons.arrow_right,
                          tooltip: 'Right',
                          onTap: term.right,
                        ),
                        _Key(
                          key: const Key('key-paste'),
                          icon: LucideIcons.clipboard_paste,
                          tooltip: 'Paste',
                          onTap: () async {
                            final data = await Clipboard.getData(
                              Clipboard.kTextPlain,
                            );
                            final text = data?.text;
                            if (text != null && text.isNotEmpty) {
                              term.paste(text);
                            }
                          },
                        ),
                      ],
                    ),
                  ),
                  TextButton(
                    key: const Key('keys-done'),
                    onPressed: onDone,
                    child: Text(
                      'Done',
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.codeSize,
                        fontWeight: FontWeight.w600,
                        color: t.accent,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// One key, in the editor bar's button style. A word where a glyph would be
/// cryptic (Esc, Tab, Ctrl), an icon for the rest; arrows are icons because
/// the mono face draws their glyphs at uneven sizes.
class _Key extends StatelessWidget {
  const _Key({
    super.key,
    this.label,
    this.icon,
    required this.tooltip,
    required this.onTap,
    this.active = false,
  }) : assert((label == null) != (icon == null));

  final String? label;
  final IconData? icon;
  final String tooltip;
  final VoidCallback onTap;
  final bool active;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final color = active ? t.accent : t.text2;
    return Tooltip(
      message: tooltip,
      child: InkWell(
        // InkWell takes no focus, so the terminal keeps it and the keyboard
        // stays up between taps.
        onTap: onTap,
        borderRadius: BorderRadius.circular(t.rControl * 0.8),
        child: Container(
          margin: EdgeInsets.symmetric(vertical: t.sp * 0.75),
          // A shade narrower than the editor bar's 1.25: nine keys and Done
          // then fit a 411 px phone, so Paste is never scrolled away.
          padding: EdgeInsets.symmetric(horizontal: t.sp),
          alignment: Alignment.center,
          decoration: BoxDecoration(
            color: active ? t.accentSoft : null,
            borderRadius: BorderRadius.circular(t.rControl * 0.8),
          ),
          child: icon != null
              ? Icon(icon, size: t.headingSize, color: color)
              : Text(
                  label!,
                  style: TextStyle(
                    fontFamily: StormTokens.monoFamily,
                    fontSize: t.codeSize,
                    fontWeight: FontWeight.w600,
                    color: color,
                  ),
                ),
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

  /// "Allow vault writes" (G-D5): off by default, for this launch only.
  bool _allowWrites = false;

  List<AgentHost> get _online => [
    for (final h in widget.hosts)
      if (h.online) h,
  ];

  @override
  void initState() {
    super.initState();
    _preselectHost();
    _loadDefault();
  }

  /// The host this device launched on last, if it is online; otherwise the
  /// only one there is. Most launches are then a single tap (decision 78).
  Future<void> _preselectHost() async {
    final online = _online;
    final last = await lastLaunchHost();
    if (!mounted || _host != null) return;
    final remembered = online.where((h) => h.id == last).firstOrNull;
    if (remembered != null) {
      await _pickHost(remembered);
    } else if (online.length == 1) {
      await _pickHost(online.first);
    }
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
      if (mounted) {
        setState(() {
          _workspaces = ws;
          // One workspace is no choice at all.
          if (ws.length == 1) _workspace = ws.first;
        });
      }
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
        allowVaultWrites: _allowWrites && _provider != 'shell',
      );
      await rememberLaunchHost(host.id);
      if (!mounted) return;
      // An old host gets no integrations; the launch says so (spec §6).
      final notice = session.launchNotice;
      if (notice != null) {
        ScaffoldMessenger.maybeOf(
          context,
        )?.showSnackBar(SnackBar(content: Text(notice)));
      }
      Navigator.of(context).pop(session);
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
                // The MCP Gateway (G-D5, G-D9): every session but a shell can
                // use the owner's integrations and read the vaults; writing
                // is this launch's choice, off by default.
                if (_provider != 'shell') ...[
                  SizedBox(height: t.sp),
                  SwitchListTile(
                    key: const Key('allow-vault-writes'),
                    contentPadding: EdgeInsets.zero,
                    value: _allowWrites,
                    onChanged: (v) => setState(() => _allowWrites = v),
                    title: const Text('Allow vault writes'),
                    subtitle: const Text(
                      'Agents can create and edit notes. Needs MCP writes on '
                      'in Server settings.',
                    ),
                  ),
                ],
                SizedBox(height: t.sp * 2),
                Text(
                  "Network: inherits ${host.name}'s policy",
                  style: TextStyle(fontSize: t.labelSize, color: t.text3),
                ),
                if (_provider != 'shell')
                  Text(
                    'Agents here can use your integrations and read your '
                    'vaults, with that network access (AM27).',
                    key: const Key('egress-integrations'),
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

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../api/models.dart';
import '../editor/frontmatter.dart' as fm;
import '../keyboard/storm_activators.dart' show stormUsesMetaModifier;
import '../router.dart';
import '../state/app_state.dart';
import '../state/terminal_prefs.dart';
import '../ui/breakpoints.dart';
import '../ui/markdown/storm_markdown_view.dart';
import '../ui/note_header.dart' show DrawerToggle;
import '../ui/panels.dart';
import '../ui/session_status.dart';
import '../ui/shell/nav_bubble.dart' show keyboardIsOpen;
import '../ui/shell/storm_scaffold.dart' show StormChrome;
import '../ui/states.dart';
import '../ui/decision_strip.dart' show DecisionLayout;
import '../ui/surfaces.dart' show showStormSheet;
import '../ui/tokens.dart';
import 'agent_api.dart' show AgentRefusal;
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart';
import 'agents_screen.dart' show launchAgentSession, resumeAgentSession;
import 'crew_changes.dart';
import 'crew_sheets.dart' show CrewRoleSections;
import 'crew_widgets.dart' show CrewChip, RoleTag;
import 'launcher.dart' show NoteRef;
import 'output_seen.dart';
import 'delete_request.dart';
import 'session_agents.dart';
import 'session_controller.dart';
import 'session_crew.dart';
import 'session_inspector.dart';
import 'spawn_state.dart';
import 'spawn_widgets.dart';
import 'spawn_work.dart';
import 'image_target.dart';
import 'terminal_surface.dart';

/// One session at `/agents/s/:id` (handoff §3.6).
///
/// At [kExpandedWidth] and wider: the terminal beside the inspector, an
/// icon rail of Context, Wrote, Agents, About and Work and the open tab's
/// panel (153i). Below it — the default — a full-screen terminal under a
/// compact header between the corner bubbles, the extra keys while it is
/// live, and a details sheet the header opens. Both are
/// the same route and share one stream (`sessionControllerProvider`), so a
/// window crossing the breakpoint keeps its terminal.
class SessionScreen extends ConsumerStatefulWidget {
  const SessionScreen({
    super.key,
    required this.sessionId,
    this.tab = SessionTab.context,
  });

  final String sessionId;
  final SessionTab tab;

  @override
  ConsumerState<SessionScreen> createState() => _SessionScreenState();
}

class _SessionScreenState extends ConsumerState<SessionScreen> {
  final _focus = FocusNode();
  bool _confirmEnd = false;
  bool _ending = false;

  bool _reloadedAtEnd = false;

  /// The delete request put aside here with Later (or Esc): its strip folds
  /// to one line with Review until the next one.
  String? _laterFor;

  /// A written note shown in the Wrote tab, with "‹ Wrote" back.
  NoteRef? _wroteNote;

  String get _id => widget.sessionId;

  @override
  void initState() {
    super.initState();
    _focus.addListener(() {
      if (_focus.hasFocus) ref.read(sessionControllerProvider(_id))?.focus();
    });
  }

  @override
  void didUpdateWidget(SessionScreen old) {
    super.didUpdateWidget(old);
    if (old.sessionId != widget.sessionId) {
      _confirmEnd = false;
      _reloadedAtEnd = false;
      _laterFor = null;
      _wroteNote = null;
    }
    if (old.tab != widget.tab) _wroteNote = null;
  }

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  void _pickTab(SessionTab tab) =>
      context.go(Routes.agentSession(_id, tab: tab.name));

  Future<void> _end(SessionController? c) async {
    if (c == null) return;
    setState(() => _ending = true);
    try {
      await c.end();
      if (mounted) setState(() => _confirmEnd = false);
      ref.invalidate(agentOverviewProvider);
    } catch (e) {
      // It has live descendants the list here didn't know of yet: show
      // them, and end only once the person has seen which (C1).
      if (e is AgentRefusal && e.code == 'confirm_descendants') {
        _showEndTree();
      } else {
        _say(describeFailure(e));
      }
    } finally {
      if (mounted) setState(() => _ending = false);
    }
  }

  /// Ending a parent lists its children first (C1); dismissing one of a
  /// tree dismisses the tree (C8). Both ask in the strip under the header
  /// at desk width, and in a sheet on a phone (153g).
  void _ask(SessionConfirm what) {
    if (!mounted) return;
    setState(() => _confirmEnd = false);
    if (context.isExpanded) {
      ref.read(sessionConfirmProvider(_id).notifier).state = what;
      return;
    }
    final s =
        ref.read(sessionDetailProvider(_id)).value ??
        ref.read(agentOverviewProvider).value?.byId(_id);
    if (s == null) return;
    showStormSheet<void>(
      context: context,
      title: 'Confirm',
      heightFactor: 0.85,
      builder: (sheet) {
        void done() {
          if (!sheet.mounted) return;
          final route = ModalRoute.of(sheet);
          if (route != null && route.isCurrent) Navigator.of(sheet).pop();
        }

        return what == SessionConfirm.dismissTree
            ? DismissTreeStrip(
                session: s,
                layout: DecisionLayout.sheet,
                onDone: done,
                onDismissed: () {
                  done();
                  _gone();
                },
              )
            : EndTreeStrip(
                session: s,
                children: what == SessionConfirm.endChildren,
                layout: DecisionLayout.sheet,
                onDone: done,
              );
      },
    );
  }

  void _showEndTree() {
    ref.invalidate(sessionDescendantsProvider(_id));
    _ask(SessionConfirm.endTree);
  }

  /// End, or End with its children listed when it has any running.
  void _askEnd(AgentSession s) {
    final o = ref.read(agentOverviewProvider).value;
    final live = o != null && childrenOf(o, s.id).any((k) => !k.ended);
    if (live) {
      _showEndTree();
    } else {
      setState(() => _confirmEnd = true);
    }
  }

  void _gone() {
    ref.invalidate(agentOverviewProvider);
    if (mounted) context.go(Routes.agents);
  }

  Future<void> _dismiss([AgentSession? s]) async {
    if (s != null && inTree(ref.read(agentOverviewProvider).value, s)) {
      _ask(SessionConfirm.dismissTree);
      return;
    }
    final api = agentApi(ref);
    if (api == null) return;
    final c = ProviderScope.containerOf(context, listen: false);
    final before = ref.read(agentOverviewProvider).value;
    final messenger = ScaffoldMessenger.maybeOf(context);
    try {
      final removed = await api.dismiss(_id);
      announceDismissedCrews(c, before, removed, fallback: messenger);
      _gone();
    } catch (e) {
      if (e is AgentRefusal &&
          (e.code == 'confirm_tree' || e.code == 'tree_live')) {
        _ask(SessionConfirm.dismissTree);
      } else {
        _say(describeFailure(e));
      }
    } finally {
      api.dispose();
    }
  }

  /// The one strip under the header at desk width (153g): a confirmation
  /// the person asked for, else End's own, else what waits for them. Never
  /// more than one, and never a dialog.
  Widget? _deskStrip(AgentSession s, SessionController? c, String host) {
    final t = context.tokens;
    void done() => ref.read(sessionConfirmProvider(_id).notifier).state = null;
    switch (ref.watch(sessionConfirmProvider(_id))) {
      case SessionConfirm.endTree || SessionConfirm.endChildren when !s.ended:
        return EndTreeStrip(
          session: s,
          children:
              ref.read(sessionConfirmProvider(_id)) ==
              SessionConfirm.endChildren,
          onDone: done,
        );
      case SessionConfirm.dismissTree:
        return DismissTreeStrip(
          session: s,
          onDone: done,
          onDismissed: () {
            done();
            _gone();
          },
        );
      default:
    }
    if (ref.watch(spawnOnAskProvider(_id)) && showsSpawnSwitch(s)) {
      return Container(
        padding: EdgeInsets.fromLTRB(
          t.sp * 2,
          t.sp * 1.5,
          t.sp * 2,
          t.sp * 1.5,
        ),
        decoration: BoxDecoration(
          border: Border(
            bottom: BorderSide(color: t.border, width: t.bw),
          ),
        ),
        child: SpawnOnConfirm(
          session: s,
          host: host,
          onDone: () =>
              ref.read(spawnOnAskProvider(_id).notifier).state = false,
        ),
      );
    }
    if (_confirmEnd && !s.ended) {
      return Padding(
        padding: EdgeInsets.fromLTRB(
          t.sp * 2,
          t.sp * 1.5,
          t.sp * 2,
          t.sp * 1.5,
        ),
        child: InlineConfirm(
          key: const Key('end-confirm'),
          title: 'End ${s.displayName}',
          message:
              'The agent and everything it started are stopped on the host.',
          confirmLabel: 'End session',
          onConfirm: _ending ? null : () => _end(c),
          onCancel: () => setState(() => _confirmEnd = false),
        ),
      );
    }
    final d = deleteDecisionOf(s);
    if (d == null) return null;
    return DeleteDecisionStrip(
      key: ValueKey('delete-${d.request.id}'),
      session: s,
      collapsed: _laterFor == d.request.id,
      onLater: () => setState(() => _laterFor = d.request.id),
      onReview: () => setState(() => _laterFor = null),
      // The terminal's own Esc is the agent's (147b).
      escapeAllowed: () => !_focus.hasFocus,
    );
  }

  void _runAgain(AgentSession s) =>
      launchAgentSession(context, ref, runAgain: s);

  bool _resuming = false;

  Future<void> _resume(AgentSession s) async {
    setState(() => _resuming = true);
    await resumeAgentSession(context, ref, s);
    if (mounted) setState(() => _resuming = false);
  }

  /// Resume, when the session's conversation can be reopened now.
  bool _canResume(AgentSession s) =>
      ref.read(agentOverviewProvider).value?.canResume(s) ?? false;

  /// Why a conversation can't be reopened now, said under the header.
  String? _resumeBlocker(AgentSession s) =>
      ref.read(agentOverviewProvider).value?.resumeBlocker(s);

  void _say(String text) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
  }

  /// The stream saw the session end before the record did: read the record
  /// and the lists again, once, so its count and the sidebar catch up.
  void _catchUpAtEnd(AgentSession? record, AgentSession? live) {
    if (_reloadedAtEnd || live == null || !live.ended) return;
    if (record != null && record.ended) return;
    _reloadedAtEnd = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      ref.invalidate(sessionDetailProvider(_id));
      ref.invalidate(agentOverviewProvider);
    });
  }

  @override
  Widget build(BuildContext context) {
    final controller = ref.watch(sessionControllerProvider(_id));
    // The lists' "wrote n" and the notes' dots follow this session's writes
    // now, not at the next list poll.
    ref.listen(sessionDetailProvider(_id).select((s) => s.value?.wroteCount), (
      before,
      now,
    ) {
      if (before != null && now != null && before != now) {
        ref.invalidate(agentOverviewProvider);
      }
    });
    // Open, so its output is read here up to wherever it has got (153e).
    void seen(int? end) {
      if (end == null) return;
      final n = ref.read(seenOutputProvider.notifier);
      scheduleMicrotask(() => n.markSeen(_id, end));
    }

    seen(
      ref.watch(sessionDetailProvider(_id).select((s) => s.value?.outputEnd)),
    );
    seen(
      ref.watch(
        agentOverviewProvider.select((o) => o.value?.byId(_id)?.outputEnd),
      ),
    );
    final detail = ref.watch(sessionDetailProvider(_id));
    final overview = ref.watch(agentOverviewProvider).value;
    return ListenableBuilder(
      listenable: Listenable.merge([?controller]),
      builder: (context, _) {
        final record = detail.value ?? overview?.byId(_id);
        final live = controller?.session;
        _catchUpAtEnd(record, live);
        final seen = record?.withLive(live) ?? live;
        // A crew or label change still pending is ahead of the record.
        final s = seen == null ? null : overview?.labelsOf(seen) ?? seen;
        final wide = context.isExpanded;
        if (s == null) {
          final settled =
              !detail.isLoading && overview != null && !overview.unreachable;
          return Scaffold(
            backgroundColor: context.tokens.bg,
            body: settled
                ? EmptyState(
                    icon: LucideIcons.square_terminal,
                    title: 'This session is gone',
                    detail: 'It was dismissed, or this server never had it.',
                    action: 'Agents',
                    onAction: () => context.go(Routes.agents),
                    fill: true,
                  )
                : const SkeletonRows(rows: 3),
          );
        }
        final host = overview?.hostName(s.hostId) ?? 'host';
        return wide
            ? _desk(context, s, controller, host)
            : _phone(context, s, controller, host);
      },
    );
  }

  /// The terminal, taking pasted and dropped images while the session runs,
  /// with the paste's chip over its bottom edge (D15 AM45).
  Widget _withPaste(SessionController c, AgentSession s, Widget terminal) {
    if (s.ended) return terminal;
    return Stack(
      children: [
        Positioned.fill(
          child: ImagePasteTarget(
            onImages: c.pasteImages,
            focused: () => _focus.hasFocus,
            child: terminal,
          ),
        ),
        Positioned(
          left: 0,
          right: 0,
          bottom: context.tokens.sp * 1.5,
          child: Center(child: ImagePasteChip(controller: c)),
        ),
      ],
    );
  }

  // ---- desk ---------------------------------------------------------------

  Widget _desk(
    BuildContext context,
    AgentSession s,
    SessionController? c,
    String host,
  ) {
    final t = context.tokens;
    final open =
        ref.watch(sessionInspectorOpenProvider) ?? context.drawerOpensByDefault;
    final terminal = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Expanded(
          child: c == null
              ? const SizedBox.shrink()
              : _withPaste(
                  c,
                  s,
                  StormTerminalView(
                    terminal: c.terminal,
                    focusNode: _focus,
                    autofocus: true,
                    readOnly: s.ended,
                    prefs: ref.watch(terminalPrefsProvider),
                  ),
                ),
        ),
        if (!s.ended)
          Padding(
            padding: EdgeInsets.fromLTRB(
              t.sp * 2.5,
              t.sp * 0.75,
              t.sp * 2.5,
              t.sp * 1.25,
            ),
            child: Text(
              terminalHint,
              key: const Key('terminal-hint'),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                color: t.text3,
              ),
            ),
          ),
        if (s.ended)
          Container(
            key: const Key('ended-footer'),
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 2.5,
              vertical: t.sp * 1.25,
            ),
            decoration: BoxDecoration(
              border: Border(
                top: BorderSide(color: t.border, width: t.bw),
              ),
            ),
            child: Text(
              '${endedLine(s)} · scrollback kept until you dismiss it',
              style: agentMono(t, size: t.labelSize * 1.09),
            ),
          ),
      ],
    );
    return Scaffold(
      backgroundColor: t.bg,
      body: SafeArea(
        left: false,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _header(context, s, host, inspectorOpen: open),
            ?_deskStrip(s, c, host),
            _Trouble(controller: c),
            Expanded(
              child: LayoutBuilder(
                builder: (context, box) => Row(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Expanded(child: terminal),
                    SessionInspector(
                      available: box.maxWidth,
                      open: open,
                      items: _railItems(s),
                      selected: _shownTab(s),
                      onPick: (tab) => _pickRail(tab, open: open),
                      panel: _panel(context, s, host),
                    ),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _header(
    BuildContext context,
    AgentSession s,
    String host, {
    required bool inspectorOpen,
  }) {
    final t = context.tokens;
    return Container(
      key: const Key('session-bar'),
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 2.5,
        vertical: t.sp * 1.75,
      ),
      decoration: BoxDecoration(
        border: Border(
          bottom: BorderSide(color: t.border, width: t.bw),
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              // One flexible run, so the actions sit at the far edge rather
              // than splitting the free space with the name.
              Expanded(
                child: Row(
                  children: [
                    Flexible(
                      child: Text(
                        s.displayName,
                        key: const Key('session-name'),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontFamily: StormTokens.sansFamily,
                          fontSize: t.bodySize,
                          fontWeight: FontWeight.w600,
                          color: t.text,
                        ),
                      ),
                    ),
                    SizedBox(width: t.sp * 1.25),
                    SessionStatusChip(status: s.status),
                    if (showsSpawnSwitch(s)) ...[
                      SizedBox(width: t.sp),
                      SpawnChip(session: s),
                    ],
                  ],
                ),
              ),
              DrawerToggle(
                key: const Key('inspector-toggle'),
                tooltip: 'Details',
                open: inspectorOpen,
                onTap: () =>
                    ref.read(sessionInspectorOpenProvider.notifier).state =
                        !inspectorOpen,
              ),
              SizedBox(width: t.sp),
              if (!s.ended)
                _HeaderButton(
                  key: const Key('end-session'),
                  label: 'End',
                  color: t.text2,
                  outlined: true,
                  onTap: () => _askEnd(s),
                )
              else ...[
                if (_canResume(s)) ...[
                  _HeaderButton(
                    key: const Key('resume-session'),
                    label: 'Resume',
                    color: t.accent,
                    outlined: true,
                    onTap: _resuming ? null : () => _resume(s),
                  ),
                  SizedBox(width: t.sp * 0.5),
                ],
                _HeaderButton(
                  key: const Key('run-again'),
                  label: 'Run again',
                  color: _canResume(s) ? t.text2 : t.accent,
                  outlined: true,
                  onTap: () => _runAgain(s),
                ),
                SizedBox(width: t.sp * 0.5),
                _HeaderButton(
                  key: const Key('dismiss-session'),
                  label: 'Dismiss',
                  color: t.text2,
                  onTap: () => _dismiss(s),
                ),
              ],
            ],
          ),
          SizedBox(height: t.sp * 0.5),
          Row(
            children: [
              Flexible(
                child: Text(
                  spawnMeta(
                    sessionMeta(s, host),
                    s,
                    ref.watch(agentOverviewProvider).value,
                  ),
                  key: const Key('session-meta'),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: agentMono(t, size: t.labelSize * 1.09),
                ),
              ),
              SessionMetaTags(session: s),
            ],
          ),
          if (_resumeBlocker(s) case final why?) ...[
            SizedBox(height: t.sp * 0.25),
            Text(
              why,
              key: const Key('resume-blocker'),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: agentMono(t, size: t.labelSize * 1.09),
            ),
          ],
          SpawnHeaderLine(
            session: s,
            onParent: (id) => openAgentSession(context, id),
          ),
        ],
      ),
    );
  }

  /// The rail's icons (153i): Agents where the session can start agents or
  /// has, Work for a child with a workspace of its own.
  List<InspectorRailItem> _railItems(AgentSession s) {
    final o = ref.watch(agentOverviewProvider).value;
    return [
      (
        tab: SessionTab.context,
        name: 'Context',
        icon: LucideIcons.file,
        badge: null,
      ),
      (
        tab: SessionTab.wrote,
        name: 'Wrote',
        icon: LucideIcons.pen_line,
        badge: s.wroteCount > 0 ? (count: s.wroteCount, loud: false) : null,
      ),
      if (showsAgentsTab(s, o))
        (
          tab: SessionTab.agents,
          name: 'Agents',
          icon: LucideIcons.git_fork,
          badge: agentsBadge(s, o),
        ),
      (
        tab: SessionTab.about,
        name: 'About',
        icon: LucideIcons.info,
        badge: null,
      ),
      if (s.hasOwnWorkspace)
        (
          tab: SessionTab.work,
          name: 'Work',
          icon: LucideIcons.git_commit_horizontal,
          badge: null,
        ),
    ];
  }

  /// The tab the route names, or About where this session has no such tab.
  SessionTab _shownTab(AgentSession s) {
    final tab = widget.tab;
    return switch (tab) {
      SessionTab.agents
          when !showsAgentsTab(s, ref.watch(agentOverviewProvider).value) =>
        SessionTab.about,
      // A shared-workspace child has no Work tab (151 N6).
      SessionTab.work when !s.hasOwnWorkspace => SessionTab.about,
      _ => tab,
    };
  }

  /// The open tab's icon closes the panel; any other opens its tab.
  void _pickRail(SessionTab tab, {required bool open}) {
    final inspector = ref.read(sessionInspectorOpenProvider.notifier);
    if (open && tab == widget.tab) {
      inspector.state = false;
      return;
    }
    inspector.state = true;
    if (tab == widget.tab) {
      setState(() => _wroteNote = null);
    } else {
      _pickTab(tab);
    }
  }

  Widget _panel(BuildContext context, AgentSession s, String host) {
    final t = context.tokens;
    final tab = _shownTab(s);
    final Widget body = switch (tab) {
      SessionTab.context =>
        s.context == null
            ? _Quiet(
                'Started without a note. Notes this session writes appear '
                'under Wrote.',
                padding: EdgeInsets.all(t.sp * 2),
              )
            : _NotePanel(
                key: ValueKey('context-${s.context!.noteId}'),
                at: (vaultId: s.context!.vaultId, noteId: s.context!.noteId),
                sessionId: s.id,
                title: s.context!.title,
              ),
      SessionTab.wrote =>
        _wroteNote != null
            ? _NotePanel(
                key: ValueKey('wrote-${_wroteNote!.noteId}'),
                at: _wroteNote!,
                sessionId: s.id,
                onBack: () => setState(() => _wroteNote = null),
              )
            : _WroteList(
                session: s,
                onOpen: (at) => setState(() => _wroteNote = at),
              ),
      SessionTab.work => ChildWorkPanel(session: s),
      SessionTab.agents => AgentsPanel(session: s),
      SessionTab.about => SingleChildScrollView(
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 2,
          vertical: t.sp * 0.75,
        ),
        child: _About(session: s, host: host),
      ),
    };
    final title = switch (tab) {
      SessionTab.context => 'Context',
      SessionTab.wrote => 'Wrote',
      SessionTab.agents => 'Agents',
      SessionTab.about => 'About',
      SessionTab.work => 'Work',
    };
    return ColoredBox(
      color: t.bg,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Container(
            padding: EdgeInsets.fromLTRB(
              t.sp * 2,
              t.sp * 0.75,
              t.sp * 0.75,
              t.sp * 0.75,
            ),
            decoration: BoxDecoration(
              border: Border(
                bottom: BorderSide(color: t.border, width: t.bw),
              ),
            ),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    title,
                    key: const Key('inspector-title'),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.codeSize,
                      fontWeight: FontWeight.w600,
                      color: t.text,
                    ),
                  ),
                ),
                IconButton(
                  key: const Key('inspector-close'),
                  tooltip: 'Close details',
                  icon: Icon(LucideIcons.x, size: t.bodySize, color: t.text3),
                  visualDensity: VisualDensity.compact,
                  onPressed: () =>
                      ref.read(sessionInspectorOpenProvider.notifier).state =
                          false,
                ),
              ],
            ),
          ),
          Expanded(child: body),
        ],
      ),
    );
  }

  // ---- phone --------------------------------------------------------------

  Widget _phone(
    BuildContext context,
    AgentSession s,
    SessionController? c,
    String host,
  ) {
    final t = context.tokens;
    final keyboard = keyboardIsOpen(context);
    void details() => _showDetails(context, s, host, c);
    final header = _PhoneHeader(session: s, onTap: details);

    return Scaffold(
      body: StormChrome(
        showNav: false,
        bar: header,
        onBack: () => context.go(Routes.agents),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            PhoneDeleteStrip(session: s),
            _Trouble(controller: c),
            Expanded(
              child: DecoratedBox(
                position: DecorationPosition.foreground,
                decoration: BoxDecoration(
                  border: Border(
                    top: BorderSide(color: t.border, width: t.bw),
                  ),
                ),
                child: c == null
                    ? const SizedBox.shrink()
                    : _withPaste(
                        c,
                        s,
                        StormTerminalView(
                          terminal: c.terminal,
                          focusNode: _focus,
                          // Focus opens the keyboard on a phone, which should
                          // wait for a tap on the terminal.
                          autofocus: false,
                          readOnly: s.ended,
                          phone: true,
                          prefs: ref.watch(terminalPrefsProvider),
                        ),
                      ),
              ),
            ),
            // A keyboard accessory: there only while the software keyboard
            // is, so a focused terminal with the keyboard down keeps the room.
            if (!s.ended && c != null)
              AnimatedSize(
                duration: t.duration,
                curve: Curves.easeOut,
                alignment: Alignment.topCenter,
                child: keyboard
                    ? _ExtraKeys(terminal: c.terminal, onDone: _focus.unfocus)
                    : const SafeArea(
                        top: false,
                        child: SizedBox(width: double.infinity),
                      ),
              ),
            if (s.ended)
              Container(
                padding: EdgeInsets.fromLTRB(
                  t.sp * 2,
                  t.sp * 1.5,
                  t.sp * 2,
                  t.sp * 1.5,
                ),
                decoration: BoxDecoration(
                  border: Border(
                    top: BorderSide(color: t.border, width: t.bw),
                  ),
                ),
                child: SafeArea(
                  top: false,
                  minimum: EdgeInsets.only(bottom: t.sp * 2),
                  child: Row(
                    children: [
                      if (_canResume(s)) ...[
                        Expanded(
                          child: _BarButton(
                            key: const Key('resume-session'),
                            label: 'Resume',
                            primary: true,
                            onTap: _resuming ? null : () => _resume(s),
                          ),
                        ),
                        SizedBox(width: t.sp),
                      ],
                      Expanded(
                        child: _BarButton(
                          key: const Key('run-again'),
                          label: 'Run again',
                          primary: !_canResume(s),
                          onTap: () => _runAgain(s),
                        ),
                      ),
                      SizedBox(width: t.sp),
                      Expanded(
                        child: _BarButton(
                          key: const Key('dismiss-session'),
                          label: 'Dismiss',
                          onTap: () => _dismiss(s),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }

  Future<void> _showDetails(
    BuildContext context,
    AgentSession s,
    String host,
    SessionController? c,
  ) {
    final t = context.tokens;
    return showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      backgroundColor: t.surface,
      barrierColor: const Color(0x59000000),
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(t.rCard)),
        side: BorderSide(color: t.border, width: t.bw),
      ),
      builder: (sheet) => ConstrainedBox(
        constraints: BoxConstraints(
          maxHeight: MediaQuery.sizeOf(sheet).height * 0.75,
        ),
        child: SessionDetailsSheet(
          sessionId: s.id,
          fallback: s,
          host: host,
          onOpenNote: (at) {
            Navigator.of(sheet).pop();
            context.push(Routes.note(at.vaultId, at.noteId, session: s.id));
          },
          onEnd: () async {
            await _end(c);
          },
        ),
      ),
    );
  }
}

/// Why input or the stream is not reaching the session, if it is not.
class _Trouble extends StatelessWidget {
  const _Trouble({required this.controller});

  final SessionController? controller;

  @override
  Widget build(BuildContext context) {
    final c = controller;
    if (c == null) return const SizedBox.shrink();
    final text =
        c.inputError ??
        (c.streamError != null && !c.connected
            ? 'Reconnecting: ${c.streamError}'
            : null);
    if (text == null) return const SizedBox.shrink();
    final t = context.tokens;
    return Container(
      padding: EdgeInsets.symmetric(
        horizontal: t.sp * 2.5,
        vertical: t.sp * 0.5,
      ),
      color: t.amberSoft,
      child: Text(
        text,
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontSize: t.labelSize,
          color: t.text,
        ),
      ),
    );
  }
}

class _Quiet extends StatelessWidget {
  const _Quiet(this.text, {required this.padding});

  final String text;
  final EdgeInsets padding;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: padding,
      child: Text(
        text,
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontSize: t.uiSize,
          height: 1.5,
          color: t.text3,
        ),
      ),
    );
  }
}

/// End, Run again and Dismiss in the session header: small, 13px.
class _HeaderButton extends StatelessWidget {
  const _HeaderButton({
    super.key,
    required this.label,
    required this.color,
    required this.onTap,
    this.outlined = false,
  });

  final String label;
  final Color color;
  final VoidCallback? onTap;
  final bool outlined;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final radius = BorderRadius.circular(t.rControl);
    return Semantics(
      button: true,
      label: label,
      excludeSemantics: true,
      onTap: onTap,
      child: Material(
        color: Colors.transparent,
        shape: RoundedRectangleBorder(
          borderRadius: radius,
          side: outlined
              ? BorderSide(color: t.border, width: t.bw)
              : BorderSide.none,
        ),
        child: InkWell(
          borderRadius: radius,
          onTap: onTap,
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 1.25,
              vertical: t.sp * 0.625,
            ),
            child: Text(
              label,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                color: color,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// "Running · Claude Code · Gateway spec · wrote 2": the phone header's
/// second line.
String phoneHeaderMeta(AgentSession s) => [
  s.statusLabel,
  providerLabel(s.provider),
  ?s.model,
  ?permissionModeWord(s.permissionMode),
  ?s.context?.title,
  if (s.wroteCount > 0) 'wrote ${s.wroteCount}',
].join(' · ');

/// The phone session's header, between the corner bubbles: status dot, name
/// and caret over a mono line. A tap opens the details sheet, where the full
/// name wraps.
class _PhoneHeader extends ConsumerWidget {
  const _PhoneHeader({required this.session, required this.onTap});

  final AgentSession session;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final s = session;
    final meta = phoneHeaderMeta(s);
    final crew = ref.watch(agentOverviewProvider).value?.crewOf(s);
    return Semantics(
      button: true,
      label: '${s.displayName}, $meta, details',
      excludeSemantics: true,
      onTap: onTap,
      child: GestureDetector(
        key: const Key('session-header'),
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            Row(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                SessionHeaderDot(session: s, size: t.sp * 0.875),
                SizedBox(width: t.sp * 0.75),
                Flexible(
                  child: Text(
                    s.displayName,
                    key: const Key('session-name'),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.bodySize * 0.94,
                      fontWeight: FontWeight.w600,
                      height: 1.25,
                      color: t.text,
                    ),
                  ),
                ),
                SizedBox(width: t.sp * 0.75),
                Text(
                  '▾',
                  style: TextStyle(
                    fontSize: t.labelSize,
                    height: 1,
                    color: t.text3,
                  ),
                ),
              ],
            ),
            SizedBox(height: t.sp * 0.25),
            // The crew chip and role tag ride the meta line (138-5); each
            // gives way in proportion when the three don't fit.
            Row(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                Flexible(
                  flex: 4,
                  child: Text(
                    meta,
                    key: const Key('session-meta'),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    textAlign: TextAlign.center,
                    style: agentMono(t, size: 11).copyWith(height: 1.3),
                  ),
                ),
                if (crew != null) ...[
                  SizedBox(width: t.sp * 0.75),
                  Flexible(
                    flex: 3,
                    child: LayoutBuilder(
                      key: const Key('phone-crew-chip'),
                      builder: (context, box) =>
                          CrewChip(crew: crew, maxWidth: box.maxWidth),
                    ),
                  ),
                ],
                if (s.role case final role?) ...[
                  SizedBox(width: t.sp * 0.75),
                  Flexible(
                    flex: 2,
                    child: RoleTag(role: role, launchRole: s.launchRole),
                  ),
                ],
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// A half-width phone button: Run again, Dismiss, Cancel, End session.
class _BarButton extends StatelessWidget {
  const _BarButton({
    super.key,
    required this.label,
    required this.onTap,
    this.primary = false,
    this.danger = false,
    this.dangerText = false,
  });

  final String label;
  final VoidCallback? onTap;
  final bool primary;
  final bool danger;
  final bool dangerText;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final radius = BorderRadius.circular(t.rControl);
    final fill = primary
        ? t.accent
        : danger
        ? t.danger
        : Colors.transparent;
    final ink = primary || danger
        ? t.onAccent
        : dangerText
        ? t.danger
        : t.text2;
    return Semantics(
      button: true,
      label: label,
      excludeSemantics: true,
      onTap: onTap,
      child: Material(
        color: fill,
        shape: RoundedRectangleBorder(
          borderRadius: radius,
          side: primary || danger
              ? BorderSide.none
              : BorderSide(color: t.border, width: t.bw),
        ),
        child: InkWell(
          borderRadius: radius,
          onTap: onTap,
          child: Padding(
            padding: EdgeInsets.symmetric(vertical: t.sp * 1.5),
            child: Text(
              label,
              textAlign: TextAlign.center,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.uiSize,
                fontWeight: primary ? FontWeight.w500 : FontWeight.w400,
                color: ink,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// A note in the panel, read only: crumb and Open in Notes, title, version
/// and body (handoff §3.6). [title] is the launch snapshot for a context
/// note; a written note shows its own.
class _NotePanel extends ConsumerWidget {
  const _NotePanel({
    super.key,
    required this.at,
    required this.sessionId,
    this.title,
    this.onBack,
  });

  final NoteRef at;
  final String sessionId;
  final String? title;
  final VoidCallback? onBack;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    // A write by this session may be to this very note.
    ref.listen(
      sessionDetailProvider(sessionId).select((s) => s.value?.wroteCount),
      (_, _) => ref.invalidate(agentNoteProvider(at)),
    );
    final note = ref.watch(agentNoteProvider(at));
    final vaults = ref.watch(vaultsProvider).value ?? const <VaultInfo>[];
    final vault =
        vaults.where((v) => v.id == at.vaultId).firstOrNull?.name ?? '';
    final n = note.value;
    void open() => context.go(Routes.note(at.vaultId, at.noteId));
    final accent = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      color: t.accent,
    );
    return SingleChildScrollView(
      padding: EdgeInsets.fromLTRB(t.sp * 2, t.sp * 2.75, t.sp * 2, t.sp * 5),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              if (onBack != null) ...[
                Semantics(
                  link: true,
                  label: 'Back to Wrote',
                  excludeSemantics: true,
                  onTap: onBack,
                  child: GestureDetector(
                    key: const Key('back-to-wrote'),
                    onTap: onBack,
                    child: Text('‹ Wrote', style: accent),
                  ),
                ),
                SizedBox(width: t.sp),
              ],
              Expanded(
                child: Text(
                  n == null ? vault : noteCrumb(vault, n.meta.path),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: agentMono(t, size: t.labelSize * 1.09),
                ),
              ),
              Flexible(
                child: Semantics(
                  link: true,
                  label: 'Open in Notes',
                  excludeSemantics: true,
                  onTap: open,
                  child: GestureDetector(
                    key: const Key('open-in-notes'),
                    onTap: open,
                    child: Text(
                      'Open in Notes ›',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: accent,
                    ),
                  ),
                ),
              ),
            ],
          ),
          SizedBox(height: t.sp * 1.25),
          Text(
            title ?? (n == null ? '' : noteTitleOf(n.meta.title, n.meta.path)),
            key: const Key('panel-note-title'),
            style: TextStyle(
              fontFamily: StormTokens.serifFamily,
              fontSize: t.headingSize * 1.3,
              fontWeight: FontWeight.w600,
              height: 1.2,
              color: t.text,
            ),
          ),
          if (n != null) ...[
            SizedBox(height: t.sp * 1.25),
            Text(
              panelVersionLine(n, sessionId),
              key: const Key('panel-version-line'),
              style: agentMono(t, size: t.labelSize * 1.09),
            ),
            SizedBox(height: t.sp * 1.25),
            StormMarkdownView(markdown: fm.split(n.content).body),
          ] else if (note.isLoading)
            const SkeletonRows(rows: 3)
          else
            Padding(
              padding: EdgeInsets.only(top: t.sp * 1.5),
              child: Text(
                'This note is no longer in its vault.',
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.uiSize,
                  color: t.text3,
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// `v14`, and whether [sessionId] is the note's latest agent writer.
String panelVersionLine(Note n, String sessionId) {
  final w = n.agentWrite;
  final by = w == null || w.sessionId != sessionId
      ? ''
      : w.created
      ? ' · created by this session'
      : ' · edited by this session';
  return 'v${n.meta.version}$by';
}

/// "new", or the version a session left a note at.
String _wroteMeta(SessionWrite w) => w.created
    ? 'new'
    : w.version != null
    ? 'v${w.version}'
    : 'edited';

String _wroteTitle(SessionWrite w) {
  final title = noteTitleOf(w.title, w.path);
  return title.isNotEmpty ? title : (w.path ?? w.kind);
}

String _wroteEmpty(AgentSession s) => s.isShell || s.writeVaultId == null
    ? 'This session can’t write to your vaults.'
    : 'Nothing written yet. Notes appear here as the session writes them.';

/// The notes this session created or edited, newest write first.
class _WroteList extends ConsumerWidget {
  const _WroteList({required this.session, required this.onOpen});

  final AgentSession session;
  final ValueChanged<NoteRef> onOpen;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final writes = ref.watch(sessionWritesProvider(session.id));
    final vaults = ref.watch(vaultsProvider).value ?? const <VaultInfo>[];
    final list = writes.value;
    final padding = EdgeInsets.symmetric(
      horizontal: t.sp * 2,
      vertical: t.sp * 2.25,
    );
    if (list == null) {
      return writes.isLoading
          ? Padding(padding: padding, child: const SkeletonRows(rows: 2))
          : _Quiet(_wroteEmpty(session), padding: padding * 1.5);
    }
    if (list.isEmpty) {
      return _Quiet(
        _wroteEmpty(session),
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 2,
          vertical: t.sp * 3.5,
        ),
      );
    }
    return ListView(
      padding: padding,
      children: [
        for (final w in list)
          _WroteRow(
            write: w,
            vault:
                vaults.where((v) => v.id == w.vaultId).firstOrNull?.name ?? '',
            onTap: w.note == null ? null : () => onOpen(w.note!),
          ),
      ],
    );
  }
}

class _WroteRow extends StatelessWidget {
  const _WroteRow({required this.write, required this.vault, this.onTap});

  final SessionWrite write;
  final String vault;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final w = write;
    final radius = BorderRadius.circular(t.rControl);
    return Semantics(
      button: onTap != null,
      label: '${_wroteTitle(w)}, ${_wroteMeta(w)}',
      excludeSemantics: true,
      onTap: onTap,
      child: Material(
        color: Colors.transparent,
        borderRadius: radius,
        child: InkWell(
          key: Key('wrote-${w.noteId ?? w.path}'),
          borderRadius: radius,
          hoverColor: t.surface,
          onTap: onTap,
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 1.5,
              vertical: t.sp * 1.25,
            ),
            child: Row(
              children: [
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        _wroteTitle(w),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontFamily: StormTokens.sansFamily,
                          fontSize: t.uiSize,
                          color: t.text,
                        ),
                      ),
                      SizedBox(height: t.sp * 0.25),
                      Text(
                        noteCrumb(vault, w.path),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: agentMono(t),
                      ),
                    ],
                  ),
                ),
                SizedBox(width: t.sp),
                Text(_wroteMeta(w), style: agentMono(t)),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// The way in and out of the terminal (147b), said under it at desk width
/// (153j), with this platform's Shift.
String get terminalHint =>
    'Tab and ${stormUsesMetaModifier ? '⇧Tab' : 'Shift+Tab'} go to the agent '
    '· Esc, then Tab, leaves the terminal';

/// About's plain rows after Crew, Role and Starts agents (153j): who it ran
/// as and who started it. What the header's meta line says, and the
/// internals (identity, isolation, network, vault access), are not here.
List<(String, String)> aboutRows(
  AgentSession s, {
  AgentOverview? overview,
  DateTime? now,
}) => [
  ('Ran as', ranAsLine(s.ranAs)),
  ('Started by', startedByLine(s, overview, now: now)),
];

/// "You, from Gateway spec, 40m ago", or the agent that started it.
String startedByLine(AgentSession s, AgentOverview? o, {DateTime? now}) {
  final age = shortAge(s.createdAt, now: now);
  final when = age.isEmpty
      ? null
      : age == 'now'
      ? 'just now'
      : '$age ago';
  final parent = s.spawn.parentSession;
  if (parent != null) {
    return [o?.byId(parent)?.displayName ?? parent, ?when].join(', ');
  }
  final from = s.resumedFrom == null ? null : o?.byId(s.resumedFrom!);
  final oldParent = from?.spawn.parentSession;
  return [
    'You',
    if (s.context != null) 'from ${s.context!.title}',
    if (oldParent != null)
      'resuming what ${o?.byId(oldParent)?.displayName ?? oldParent} '
          'started',
    ?when,
  ].join(', ');
}

/// "figma isn't loaded: it needs its address." (153j): its plugins' MCP
/// servers the host left out, in one line. Null when none was.
String? pluginProblemLine(AgentSession s) {
  final skipped = [
    for (final x in s.mcpSkipped)
      if (x.source.startsWith('plugin')) x,
  ];
  if (skipped.isEmpty) return null;
  if (skipped.length > 1) {
    return '${skipped.length} plugin servers aren’t loaded: '
        '${skipped.map((x) => x.name).join(', ')}.';
  }
  final x = skipped.single;
  final why = x.remote
      ? 'it reaches a session only as an integration'
      : x.reason.trim();
  return '${x.name} isn’t loaded${why.isEmpty ? '' : ': $why'}.';
}

/// The plugin problem, with where it is fixed.
class PluginProblem extends StatelessWidget {
  const PluginProblem({super.key, required this.session});

  final AgentSession session;

  @override
  Widget build(BuildContext context) {
    final line = pluginProblemLine(session);
    if (line == null) return const SizedBox.shrink();
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.only(top: t.sp * 2),
      child: Column(
        key: const Key('plugin-problem'),
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(line, style: agentSans(t, color: t.text3)),
          SizedBox(height: t.sp * 0.5),
          AccentLink(
            key: const Key('fix-in-integrations'),
            label: 'Fix in Integrations ›',
            onTap: () => context.go(Routes.integrations),
          ),
        ],
      ),
    );
  }
}

/// A mono label over its value, as About's rows are.
class _AboutRow extends StatelessWidget {
  const _AboutRow({required this.label, required this.child});

  final String label;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.only(top: t.sp * 1.75),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(label.toUpperCase(), style: agentKey(t)),
          SizedBox(height: t.sp * 0.375),
          child,
        ],
      ),
    );
  }
}

/// About, slimmed down (153j): who and what the session is. Crew, Role,
/// Starts agents, Ran as, Started by, and a plugin problem in one line.
class AboutSections extends ConsumerWidget {
  const AboutSections({
    super.key,
    required this.session,
    required this.host,
    this.crewRole = true,
  });

  final AgentSession session;
  final String host;

  /// False in the phone's sheet, which has its own Crew and Role (138n).
  final bool crewRole;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final rows = aboutRows(
      session,
      overview: ref.watch(agentOverviewProvider).value,
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (crewRole) AboutCrewRole(session: session),
        if (session.spawn.switchState != null)
          _AboutRow(
            label: 'Starts agents',
            child: StartsAgentsAbout(session: session, host: host),
          ),
        for (final (label, value) in rows)
          _AboutRow(
            label: label,
            child: Text(
              value,
              key: Key('about-${label.toLowerCase().replaceAll(' ', '-')}'),
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.uiSize,
                height: 1.45,
                color: t.text,
              ),
            ),
          ),
        PluginProblem(session: session),
      ],
    );
  }
}

class _About extends StatelessWidget {
  const _About({required this.session, required this.host});

  final AgentSession session;
  final String host;

  @override
  Widget build(BuildContext context) => KeyedSubtree(
    key: const Key('session-about'),
    child: AboutSections(session: session, host: host),
  );
}

/// The phone's details: where it started, what it wrote, About, and End
/// with its confirmation in place (handoff §3.6).
class SessionDetailsSheet extends ConsumerStatefulWidget {
  const SessionDetailsSheet({
    super.key,
    required this.sessionId,
    required this.fallback,
    required this.host,
    required this.onOpenNote,
    required this.onEnd,
  });

  final String sessionId;
  final AgentSession fallback;
  final String host;
  final ValueChanged<NoteRef> onOpenNote;
  final Future<void> Function() onEnd;

  @override
  ConsumerState<SessionDetailsSheet> createState() => _DetailsState();
}

class _DetailsState extends ConsumerState<SessionDetailsSheet> {
  bool _confirm = false;
  bool _ending = false;
  bool _endTree = false;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final live = ref.watch(sessionControllerProvider(widget.sessionId));
    final record = ref.watch(sessionDetailProvider(widget.sessionId)).value;
    final s = (record ?? widget.fallback).withLive(live?.session);
    final writes = ref.watch(sessionWritesProvider(s.id)).value;
    final overview = ref.watch(agentOverviewProvider).value;
    final label = TextStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: t.labelSize,
      letterSpacing: t.labelSize * 0.08,
      color: t.text3,
    );

    final children = <Widget>[
      Center(
        child: Container(
          width: t.sp * 4.5,
          height: t.sp * 0.5,
          decoration: BoxDecoration(
            color: t.border,
            borderRadius: BorderRadius.circular(999),
          ),
        ),
      ),
      Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            s.displayName,
            key: const Key('details-name'),
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.bodySize * 1.06,
              fontWeight: FontWeight.w600,
              height: 1.3,
              color: t.text,
            ),
          ),
          SizedBox(height: t.sp),
          SessionStatusChip(status: s.status),
          SpawnHeaderLine(session: s),
        ],
      ),
      if (showsAgentsTab(s, overview))
        KeyedSubtree(
          key: const Key('details-agents'),
          child: AgentsPanel(session: s, sheet: true),
        ),
      if (s.hasOwnWorkspace)
        Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('WORK', style: label),
            SizedBox(height: t.sp * 0.75),
            ChildWorkPanel(session: s, sheet: true),
          ],
        ),
      if (s.context != null)
        Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('STARTED FROM', style: label),
            SizedBox(height: t.sp * 0.75),
            Semantics(
              button: true,
              label: 'Open ${s.context!.title}',
              excludeSemantics: true,
              onTap: () => widget.onOpenNote((
                vaultId: s.context!.vaultId,
                noteId: s.context!.noteId,
              )),
              child: InkWell(
                key: const Key('details-context'),
                onTap: () => widget.onOpenNote((
                  vaultId: s.context!.vaultId,
                  noteId: s.context!.noteId,
                )),
                borderRadius: BorderRadius.circular(t.rControl),
                child: Container(
                  padding: EdgeInsets.symmetric(
                    horizontal: t.sp * 1.5,
                    vertical: t.sp * 1.25,
                  ),
                  decoration: BoxDecoration(
                    borderRadius: BorderRadius.circular(t.rControl),
                    border: Border.all(color: t.border, width: t.bw),
                  ),
                  child: Text(
                    '▤ ${s.context!.title} ›',
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.uiSize * 1.05,
                      color: t.text,
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text('WROTE', style: label),
          SizedBox(height: t.sp * 0.25),
          if (writes == null || writes.isEmpty)
            Padding(
              padding: EdgeInsets.symmetric(vertical: t.sp * 0.75),
              child: Text(
                _wroteEmpty(s),
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.uiSize,
                  color: t.text3,
                ),
              ),
            )
          else
            for (final w in writes)
              Semantics(
                button: w.note != null,
                label: '${_wroteTitle(w)}, ${_wroteMeta(w)}',
                excludeSemantics: true,
                onTap: w.note == null ? null : () => widget.onOpenNote(w.note!),
                child: InkWell(
                  onTap: w.note == null
                      ? null
                      : () => widget.onOpenNote(w.note!),
                  child: Container(
                    padding: EdgeInsets.symmetric(vertical: t.sp * 1.25),
                    decoration: BoxDecoration(
                      border: Border(
                        bottom: BorderSide(color: t.border, width: t.bw),
                      ),
                    ),
                    child: Row(
                      children: [
                        Expanded(
                          child: Text(
                            _wroteTitle(w),
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              fontFamily: StormTokens.sansFamily,
                              fontSize: t.uiSize * 1.05,
                              color: t.text,
                            ),
                          ),
                        ),
                        Text(_wroteMeta(w), style: agentMono(t)),
                      ],
                    ),
                  ),
                ),
              ),
        ],
      ),
      // The same Crew and Role a long-press on its row raises (138n).
      CrewRoleSections(session: s),
      AboutSections(session: s, host: widget.host, crewRole: false),
      if (!s.ended) ...[
        if (_endTree)
          EndTreeStrip(
            session: s,
            layout: DecisionLayout.sheet,
            onDone: () => setState(() => _endTree = false),
          )
        else if (_confirm) ...[
          Text(
            'The agent and everything it started are stopped on the host.',
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.codeSize,
              height: 1.45,
              color: t.text2,
            ),
          ),
          Row(
            children: [
              Expanded(
                child: _BarButton(
                  key: const Key('confirm-cancel'),
                  label: 'Cancel',
                  onTap: () => setState(() => _confirm = false),
                ),
              ),
              SizedBox(width: t.sp),
              Expanded(
                child: _BarButton(
                  key: const Key('confirm-action'),
                  label: 'End session',
                  danger: true,
                  onTap: _ending
                      ? null
                      : () async {
                          setState(() => _ending = true);
                          await widget.onEnd();
                          if (mounted) {
                            setState(() {
                              _ending = false;
                              _confirm = false;
                            });
                          }
                        },
                ),
              ),
            ],
          ),
        ] else
          _BarButton(
            key: const Key('end-session'),
            label: 'End session',
            dangerText: true,
            onTap: () {
              final o = ref.read(agentOverviewProvider).value;
              final kids =
                  o != null && childrenOf(o, s.id).any((k) => !k.ended);
              if (kids) ref.invalidate(sessionDescendantsProvider(s.id));
              setState(() => kids ? _endTree = true : _confirm = true);
            },
          ),
      ],
    ];

    return SafeArea(
      top: false,
      child: SingleChildScrollView(
        padding: EdgeInsets.fromLTRB(
          t.sp * 2.5,
          t.sp * 1.25,
          t.sp * 2.5,
          t.sp * 3.75,
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (var i = 0; i < children.length; i++) ...[
              if (i > 0) SizedBox(height: t.sp * 1.5),
              children[i],
            ],
          ],
        ),
      ),
    );
  }
}

/// Esc, Tab, Shift+Tab, sticky Ctrl and Shift, arrows and Paste: the keys a phone
/// keyboard lacks (freeze §10, AC-F4). Shown while the session is live;
/// Done appears with the keyboard and only puts it away (plan §5.1).
class _ExtraKeys extends StatelessWidget {
  const _ExtraKeys({required this.terminal, required this.onDone});

  final StormTerminal terminal;
  final VoidCallback onDone;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final term = terminal;
    return Container(
      decoration: BoxDecoration(
        color: t.surface2,
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
                    padding: EdgeInsets.symmetric(horizontal: t.sp * 1.25),
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
                        label: '⇧',
                        tooltip: 'Shift, for the next key',
                        active: term.stickyShift,
                        onTap: () => term.stickyShift = !term.stickyShift,
                      ),
                      // Claude Code's mode switch (decision 147b).
                      _Key(
                        key: const Key('key-backtab'),
                        label: '⇧Tab',
                        tooltip: 'Shift+Tab: switch the agent’s mode',
                        onTap: term.backTab,
                      ),
                      _Key(label: '←', tooltip: 'Left', onTap: term.left),
                      _Key(label: '→', tooltip: 'Right', onTap: term.right),
                      _Key(label: '↑', tooltip: 'Up', onTap: term.up),
                      _Key(label: '↓', tooltip: 'Down', onTap: term.down),
                    ],
                  ),
                ),
                _Key(
                  key: const Key('key-paste'),
                  label: 'Paste',
                  tooltip: 'Paste',
                  onTap: () async {
                    // An image first (D15 AM45), as the desktop's paste does.
                    if (await term.onImagePaste?.call() ?? false) return;
                    final data = await Clipboard.getData(Clipboard.kTextPlain);
                    final text = data?.text;
                    if (text != null && text.isNotEmpty) term.paste(text);
                  },
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
    );
  }
}

/// One key: mono, on `surface`, `accent` while armed.
class _Key extends StatelessWidget {
  const _Key({
    super.key,
    required this.label,
    required this.tooltip,
    required this.onTap,
    this.active = false,
  });

  final String label;
  final String tooltip;
  final VoidCallback onTap;
  final bool active;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final radius = BorderRadius.circular(t.rControl);
    return Padding(
      padding: EdgeInsets.symmetric(horizontal: t.sp * 0.375, vertical: t.sp),
      child: Tooltip(
        message: tooltip,
        child: Material(
          color: active ? t.accentSoft : t.surface,
          borderRadius: radius,
          // InkWell takes no focus, so the terminal keeps it and the keyboard
          // stays up between taps.
          child: InkWell(
            onTap: onTap,
            borderRadius: radius,
            child: Container(
              padding: EdgeInsets.symmetric(horizontal: t.sp * 1.25),
              alignment: Alignment.center,
              child: Text(
                label,
                style: TextStyle(
                  fontFamily: StormTokens.monoFamily,
                  fontSize: t.labelSize * 1.09,
                  color: active ? t.accent : t.text2,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// "Pasting image…" with a cancel while a paste takes a moment, or why it
/// failed; nothing at all otherwise (D15 AM45).
class ImagePasteChip extends StatelessWidget {
  const ImagePasteChip({super.key, required this.controller});

  final SessionController controller;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return ValueListenableBuilder<ImagePasteState?>(
      valueListenable: controller.imagePaste,
      builder: (context, state, _) {
        if (state == null) return const SizedBox.shrink();
        final error = state.error;
        final label =
            error ??
            (state.count > 1
                ? 'Pasting ${state.count} images…'
                : 'Pasting image…');
        return Container(
          key: const Key('image-paste-chip'),
          padding: EdgeInsets.fromLTRB(
            t.sp * 1.5,
            t.sp * 0.75,
            t.sp * 0.75,
            t.sp * 0.75,
          ),
          decoration: BoxDecoration(
            color: t.surface2,
            borderRadius: BorderRadius.circular(t.rControl),
            border: Border.all(
              color: error == null ? t.border : t.danger,
              width: t.bw,
            ),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Flexible(
                child: Text(
                  label,
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.labelSize + 1,
                    color: error == null ? t.text2 : t.danger,
                  ),
                ),
              ),
              SizedBox(width: t.sp),
              Semantics(
                button: true,
                label: error == null ? 'Cancel the paste' : 'Dismiss',
                excludeSemantics: true,
                onTap: controller.dismissPaste,
                child: InkWell(
                  key: const Key('image-paste-dismiss'),
                  onTap: controller.dismissPaste,
                  borderRadius: BorderRadius.circular(t.rControl * 0.6),
                  child: Padding(
                    padding: EdgeInsets.all(t.sp * 0.5),
                    child: Icon(
                      LucideIcons.x,
                      size: t.labelSize + 2,
                      color: t.text3,
                    ),
                  ),
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}

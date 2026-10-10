import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../api/models.dart';
import '../editor/frontmatter.dart' as fm;
import '../router.dart';
import '../state/app_state.dart';
import '../state/health.dart' show integrationsSummaryProvider;
import '../state/terminal_prefs.dart';
import '../ui/breakpoints.dart';
import '../ui/markdown/storm_markdown_view.dart';
import '../ui/note_header.dart' show DrawerToggle;
import '../ui/panels.dart';
import '../ui/session_status.dart';
import '../ui/shell/nav_bubble.dart' show keyboardIsOpen;
import '../ui/shell/storm_scaffold.dart' show StormChrome;
import '../ui/states.dart';
import '../ui/tokens.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart';
import 'agents_screen.dart' show launchAgentSession;
import 'launcher.dart' show NoteRef;
import 'session_controller.dart';
import 'session_inspector.dart';
import 'terminal_surface.dart';

/// One session at `/agents/s/:id` (handoff §3.6).
///
/// At [kExpandedWidth] and wider: the terminal beside a panel of Context,
/// Wrote and About. Below it — the default — a full-screen terminal under a
/// chip row, the extra keys while it is live, and a details sheet. Both are
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
      _say(describeFailure(e));
    } finally {
      if (mounted) setState(() => _ending = false);
    }
  }

  Future<void> _dismiss() async {
    final api = agentApi(ref);
    if (api == null) return;
    try {
      await api.dismiss(_id);
      ref.invalidate(agentOverviewProvider);
      if (mounted) context.go(Routes.agents);
    } catch (e) {
      _say(describeFailure(e));
    } finally {
      api.dispose();
    }
  }

  void _runAgain(AgentSession s) =>
      launchAgentSession(context, ref, runAgain: s);

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
    final detail = ref.watch(sessionDetailProvider(_id));
    final overview = ref.watch(agentOverviewProvider).value;
    return ListenableBuilder(
      listenable: Listenable.merge([?controller]),
      builder: (context, _) {
        final record = detail.value ?? overview?.byId(_id);
        final live = controller?.session;
        _catchUpAtEnd(record, live);
        final s = record?.withLive(live) ?? live;
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
              : StormTerminalView(
                  terminal: c.terminal,
                  focusNode: _focus,
                  autofocus: true,
                  readOnly: s.ended,
                  prefs: ref.watch(terminalPrefsProvider),
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
              style: _mono(t, t.labelSize * 1.09),
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
            if (_confirmEnd && !s.ended)
              Padding(
                padding: EdgeInsets.fromLTRB(t.sp * 2, t.sp * 1.5, t.sp * 2, 0),
                child: InlineConfirm(
                  key: const Key('end-confirm'),
                  title: 'End ${s.displayName}',
                  message:
                      'The agent and everything it started are stopped on '
                      'the host.',
                  confirmLabel: 'End session',
                  onConfirm: _ending ? null : () => _end(c),
                  onCancel: () => setState(() => _confirmEnd = false),
                ),
              ),
            _Trouble(controller: c),
            Expanded(
              child: open
                  ? LayoutBuilder(
                      builder: (context, box) => Row(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          Expanded(child: terminal),
                          SessionInspector(
                            available: box.maxWidth,
                            child: _panel(context, s, host),
                          ),
                        ],
                      ),
                    )
                  : terminal,
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
                  onTap: () => setState(() => _confirmEnd = true),
                )
              else ...[
                _HeaderButton(
                  key: const Key('run-again'),
                  label: 'Run again',
                  color: t.accent,
                  outlined: true,
                  onTap: () => _runAgain(s),
                ),
                SizedBox(width: t.sp * 0.5),
                _HeaderButton(
                  key: const Key('dismiss-session'),
                  label: 'Dismiss',
                  color: t.text2,
                  onTap: _dismiss,
                ),
              ],
            ],
          ),
          SizedBox(height: t.sp * 0.5),
          Text(
            sessionMeta(s, host),
            key: const Key('session-meta'),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: _mono(t, t.labelSize * 1.09),
          ),
        ],
      ),
    );
  }

  Widget _panel(BuildContext context, AgentSession s, String host) {
    final t = context.tokens;
    final tab = widget.tab;
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
      SessionTab.about => SingleChildScrollView(
        padding: EdgeInsets.symmetric(
          horizontal: t.sp * 2,
          vertical: t.sp * 2.75,
        ),
        child: _About(session: s, host: host),
      ),
    };
    return ColoredBox(
      color: t.bg,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Container(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 1.75,
              vertical: t.sp * 1.25,
            ),
            decoration: BoxDecoration(
              border: Border(
                bottom: BorderSide(color: t.border, width: t.bw),
              ),
            ),
            child: Row(
              children: [
                Expanded(
                  // Scrolls rather than overflowing at the inspector's
                  // narrowest.
                  child: SingleChildScrollView(
                    scrollDirection: Axis.horizontal,
                    child: PanelTabs<SessionTab>(
                      tabs: [
                        (SessionTab.context, 'Context'),
                        (SessionTab.wrote, 'Wrote ${s.wroteCount}'),
                        (SessionTab.about, 'About'),
                      ],
                      selected: tab,
                      onSelected: (next) {
                        if (next == tab) {
                          setState(() => _wroteNote = null);
                        } else {
                          _pickTab(next);
                        }
                      },
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
    final inset = StormChrome.contentInset(context);
    void details() => _showDetails(context, s, host, c);
    final header = Padding(
      padding: EdgeInsets.symmetric(horizontal: inset),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Semantics(
                link: true,
                label: 'Agents',
                excludeSemantics: true,
                onTap: () => context.go(Routes.agents),
                child: GestureDetector(
                  key: const Key('back-to-agents'),
                  behavior: HitTestBehavior.opaque,
                  onTap: () => context.go(Routes.agents),
                  child: Text(
                    '‹ Agents',
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.uiSize,
                      color: t.accent,
                    ),
                  ),
                ),
              ),
              const Spacer(),
              SessionStatusChip(status: s.status),
            ],
          ),
          SizedBox(height: t.sp),
          Text(
            s.displayName,
            key: const Key('session-name'),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontFamily: StormTokens.sansFamily,
              fontSize: t.headingSize,
              fontWeight: FontWeight.w600,
              color: t.text,
            ),
          ),
          SizedBox(height: t.sp),
          Wrap(
            spacing: t.sp,
            runSpacing: t.sp,
            children: [
              if (s.context != null)
                _Chip(
                  key: const Key('chip-context'),
                  label: '▤ ${s.context!.title}',
                  color: t.text,
                  onTap: () => context.push(
                    Routes.note(
                      s.context!.vaultId,
                      s.context!.noteId,
                      session: s.id,
                    ),
                  ),
                ),
              _Chip(
                key: const Key('chip-wrote'),
                label: 'Wrote ${s.wroteCount}',
                color: t.accent,
                onTap: details,
              ),
              _Chip(
                key: const Key('chip-details'),
                label: 'Details',
                color: t.text2,
                onTap: details,
              ),
            ],
          ),
        ],
      ),
    );

    return Scaffold(
      body: StormChrome(
        showNav: false,
        header: header,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
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
                    : StormTerminalView(
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
                      Expanded(
                        child: _BarButton(
                          key: const Key('run-again'),
                          label: 'Run again',
                          primary: true,
                          onTap: () => _runAgain(s),
                        ),
                      ),
                      SizedBox(width: t.sp),
                      Expanded(
                        child: _BarButton(
                          key: const Key('dismiss-session'),
                          label: 'Dismiss',
                          onTap: _dismiss,
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

TextStyle _mono(StormTokens t, double size, [Color? color]) => TextStyle(
  fontFamily: StormTokens.monoFamily,
  fontSize: size,
  color: color ?? t.text3,
);

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
  final VoidCallback onTap;
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

/// The phone's chip row: outlined, on `surface`.
class _Chip extends StatelessWidget {
  const _Chip({
    super.key,
    required this.label,
    required this.color,
    required this.onTap,
  });

  final String label;
  final Color color;
  final VoidCallback onTap;

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
        color: t.surface,
        shape: RoundedRectangleBorder(
          borderRadius: radius,
          side: BorderSide(color: t.border, width: t.bw),
        ),
        child: InkWell(
          borderRadius: radius,
          onTap: onTap,
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 1.25,
              vertical: t.sp * 0.75,
            ),
            child: ConstrainedBox(
              constraints: BoxConstraints(maxWidth: t.sp * 22),
              child: Text(
                label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontFamily: StormTokens.sansFamily,
                  fontSize: t.codeSize,
                  color: color,
                ),
              ),
            ),
          ),
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
                  style: _mono(t, t.labelSize * 1.09),
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
              style: _mono(t, t.labelSize * 1.09),
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
                        style: _mono(t, t.labelSize),
                      ),
                    ],
                  ),
                ),
                SizedBox(width: t.sp),
                Text(_wroteMeta(w), style: _mono(t, t.labelSize)),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// What the session is and may touch (handoff §3.6, About).
List<(String, String)> aboutRows(
  AgentSession s, {
  required String host,
  required String? writeVault,
  required List<String>? integrations,
}) => [
  ('Agent', providerLabel(s.provider)),
  ('Workspace', '${s.workspace} on $host'),
  (
    'Vault access',
    s.isShell
        ? 'None. Shell sessions have no Storm access.'
        : s.writeVaultId != null
        ? 'Reads all vaults. Writes to ${writeVault ?? 'one vault'}. Never '
              'deletes.'
        : 'Reads all vaults. No writes.',
  ),
  (
    'Integrations',
    s.isShell
        ? 'None'
        : integrations == null || integrations.isEmpty
        ? 'None'
        : '${integrations.join(', ')}. Fixed when the session started.',
  ),
  ('Network', 'Inherits $host’s policy'),
];

/// What the session was granted at launch (launch history); from a server
/// that does not say, the account's connections now.
List<String>? sessionIntegrations(WidgetRef ref, AgentSession s) {
  if (s.integrations != null) return s.integrations;
  final connections = ref.watch(integrationsSummaryProvider).value;
  return connections == null
      ? null
      : [
          for (final i in connections)
            if (!i.builtin && !i.disabled) i.displayName,
        ];
}

class _About extends ConsumerWidget {
  const _About({required this.session, required this.host});

  final AgentSession session;
  final String host;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final vaults = ref.watch(vaultsProvider).value ?? const <VaultInfo>[];
    return KeyValueList(
      key: const Key('session-about'),
      rows: aboutRows(
        session,
        host: host,
        writeVault: vaults
            .where((v) => v.id == session.writeVaultId)
            .firstOrNull
            ?.name,
        integrations: sessionIntegrations(ref, session),
      ),
    );
  }
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

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final live = ref.watch(sessionControllerProvider(widget.sessionId));
    final record = ref.watch(sessionDetailProvider(widget.sessionId)).value;
    final s = (record ?? widget.fallback).withLive(live?.session);
    final writes = ref.watch(sessionWritesProvider(s.id)).value;
    final vaults = ref.watch(vaultsProvider).value ?? const <VaultInfo>[];
    final label = TextStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: t.labelSize,
      letterSpacing: t.labelSize * 0.08,
      color: t.text3,
    );
    final rows = aboutRows(
      s,
      host: widget.host,
      writeVault: vaults.where((v) => v.id == s.writeVaultId).firstOrNull?.name,
      integrations: sessionIntegrations(ref, s),
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
                        Text(_wroteMeta(w), style: _mono(t, t.labelSize)),
                      ],
                    ),
                  ),
                ),
              ),
        ],
      ),
      KeyValueList(rows: rows),
      if (!s.ended) ...[
        if (_confirm) ...[
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
            onTap: () => setState(() => _confirm = true),
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

/// Esc, Tab, sticky Ctrl and Shift, arrows and Paste: the keys a phone
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

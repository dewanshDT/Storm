import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import '../state/app_state.dart' show sidebarCollapsedProvider;
import '../keyboard/storm_shortcut_maps.dart' show stormNewSessionChord;
import '../ui/breakpoints.dart';
import '../ui/shell/sidebar_frame.dart';
import '../ui/shell/app_bar.dart' show sidebarTopInset;
import '../ui/states.dart' show SkeletonRows;
import '../ui/tokens.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart';
import 'agents_screen.dart';
import 'crew_add.dart' show showSessionMenu;
import 'crew_changes.dart';
import 'crew_gestures.dart';
import 'crew_prefs.dart';
import 'crew_widgets.dart';
import 'output_seen.dart';
import 'sidebar_rules.dart';
import 'spawn_widgets.dart' show LineageIndent;
import 'spawn_models.dart' show kMovesWithParent;

/// The session a location shows, or null on the overview.
String? sessionIdOf(Uri location) {
  final seg = location.pathSegments;
  return seg.length == 3 && seg[0] == 'agents' && seg[1] == 's' ? seg[2] : null;
}

/// The crew a location shows, or null.
String? crewIdOf(Uri location) {
  final seg = location.pathSegments;
  return seg.length == 3 && seg[0] == 'agents' && seg[1] == 'c' ? seg[2] : null;
}

/// The frame every Agents screen sits in.
///
/// On a phone it is nothing at all: the child is the screen. On a wide
/// screen it puts [AgentsSidebar] beside it. Built by a `ShellRoute`, so the
/// sidebar keeps its state while the pane moves between the overview and
/// sessions.
class AgentsShell extends ConsumerWidget {
  const AgentsShell({super.key, required this.location, required this.child});

  final Uri location;
  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (!context.isExpanded) return child;
    final collapsed = ref.watch(sidebarCollapsedProvider);
    return Scaffold(
      body: Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (!collapsed)
            AgentsSidebar(
              selected: sessionIdOf(location),
              selectedCrew: crewIdOf(location),
            ),
          Expanded(child: PaneSemantics(child: child)),
        ],
      ),
    );
  }
}

/// Why ＋ cannot start a session now, or null when it can.
String? newSessionBlocker(AsyncValue<AgentOverview> overview) {
  final o = overview.value;
  if (o == null) return 'Loading hosts…';
  if (o.unreachable) return kAgentsOfflineTitle;
  if (o.online.isEmpty) {
    return o.hosts.isEmpty ? 'No host is set up yet' : 'No host is online';
  }
  return null;
}

/// The one header row, then CREWS, RUNNING and ENDED (handoff §2, Sidebar
/// Rules A; crews Round 4 §3). RUNNING and ENDED list only sessions in no
/// crew. Rows sort by [sidebarOrder], and the order holds while the pointer
/// is over the sidebar. Rows and crew headers drag, and ⌘/Ctrl- and
/// Shift-click select (138n); the sidebar is desktop only.
class AgentsSidebar extends ConsumerStatefulWidget {
  const AgentsSidebar({super.key, this.selected, this.selectedCrew});

  /// The open session, if any.
  final String? selected;

  /// The crew whose page is open, if any.
  final String? selectedCrew;

  @override
  ConsumerState<AgentsSidebar> createState() => _AgentsSidebarState();
}

class _AgentsSidebarState extends ConsumerState<AgentsSidebar> {
  /// The tiers rows had when the pointer came in: while it is over the
  /// sidebar nothing moves under it (§2 Sorting).
  Map<String, SidebarTier>? _held;

  void _hold() {
    final o = ref.read(agentOverviewProvider).value;
    if (o == null) return;
    setState(() => _held = tiersOf(o.sessions, DateTime.now()));
  }

  void _release() => setState(() => _held = null);

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final selected = widget.selected;
    final selectedCrew = widget.selectedCrew;
    final async = ref.watch(agentOverviewProvider);
    final o = async.value;
    final now = DateTime.now();
    // Held tiers place rows; what a row shows is always now's.
    final tier = tierLookup(now, _held);
    final shown = tierLookup(now);
    final prefs = ref.watch(crewPrefsProvider).value ?? const CrewPrefs();
    final unread = ref.watch(unreadSessionsProvider);
    final selection = ref.watch(sessionSelectionProvider);
    final dragging = ref.watch(crewDragActiveProvider);
    final renaming = ref.watch(crewRenameProvider);
    final overview = selected == null && selectedCrew == null;
    final showHost = namesHosts(o);
    // The open session's row shows what its screen shows: its own stream is
    // ahead of any list.
    final openLive = selected == null
        ? null
        : ref.watch(
            sessionControllerProvider(selected).select((c) => c?.session),
          );
    final openRecord = selected == null
        ? null
        : ref.watch(sessionDetailProvider(selected)).value;
    AgentSession drawn(AgentSession s) => s.id != selected || o == null
        ? s
        : o.labelsOf((openRecord ?? s).withLive(openLive));

    final (running: live, ended: ended) = looseSections(o);
    final crews = o?.crews ?? const <Crew>[];
    final sortedCrews = crewOrder(crews, (c) => o!.membersOf(c), tier);
    final shownCrews = prefs.section ? const <Crew>[] : sortedCrews;

    List<AgentSession> ordered(List<AgentSession> list) =>
        withoutCollapsed(sidebarOrder(list, tier), prefs.trees);
    List<AgentSession> membersShown(Crew c) =>
        prefs.collapsed(c.id) ? const [] : ordered(liveFirst(o!.membersOf(c)));
    final endedShown = prefs.ended ? const <AgentSession>[] : ordered(ended);

    // Every row, top to bottom: what a Shift-click range runs over.
    final order = [
      for (final c in shownCrews)
        for (final s in membersShown(c)) s.id,
      for (final s in ordered(live)) s.id,
      for (final s in endedShown) s.id,
    ];

    void click(AgentSession s) {
      final m = clickModifiers();
      final sel = ref.read(sessionSelectionProvider.notifier);
      if (m.toggle || m.range) {
        sel.state = selectWith(
          sel.state,
          s.id,
          order,
          toggle: m.toggle,
          range: m.range,
          current: selected,
        );
        return;
      }
      sel.state = const SessionSelection();
      openAgentSession(context, s.id);
    }

    Widget row(AgentSession s, List<AgentSession> listed) {
      final kids = descendantsIn(listed, s.id);
      final folded = prefs.treeCollapsed(s.id);
      final record = drawn(s);
      final outlined = SnapBackOutline(
        id: s.id,
        child: SessionRow(
          session: record,
          host: o?.hostName(s.hostId) ?? 'host',
          selected: s.id == selected || selection.contains(s.id),
          overview: o,
          tier: shown(record),
          now: now,
          fromParent: parentApart(s, listed, o),
          showHost: showHost,
          // The open session is being read: no dot for it.
          unread: s.id != selected && unread.contains(s.id),
          tree: kids.isEmpty
              ? null
              : TreeFold(
                  size: kids.length,
                  collapsed: folded,
                  rollUp: rollUpOf(kids, shown),
                  onToggle: () =>
                      ref.read(crewPrefsProvider.notifier).toggleTree(s.id),
                ),
          onMenu: o == null
              ? null
              : (anchor) => showSessionMenu(
                  context,
                  ref,
                  anchor: anchor,
                  session: s,
                  overview: o,
                  onOpen: () => openAgentSession(context, s.id),
                ),
          onTap: () => click(s),
        ),
      );
      // A child moves only with the agent that started it (145i).
      if (s.spawn.isChild) {
        return Tooltip(message: kMovesWithParent, child: outlined);
      }
      return CrewDraggable<SessionDrag>(data: SessionDrag(s), child: outlined);
    }

    // A child under its parent sits on the dashed indent; one listed apart
    // from it has none and says "from {parent}" (gap 3).
    Widget indented(AgentSession s, List<AgentSession> listed, Widget child) {
      final p = s.spawn.parentSession;
      return p != null && listed.any((m) => m.id == p)
          ? LineageIndent(child: child)
          : child;
    }

    // A loose row: another session dropped on it makes a crew of the two.
    Widget looseRow(AgentSession s, List<AgentSession> listed) =>
        ZoneDropTarget<SessionDrag>(
          zoneOf: (d, _) => d.session.id == s.id ? null : DropZone.onto,
          onDrop: (d, _) => makeCrew(context, [s.id, d.session.id]),
          child: row(s, listed),
        );

    // A member: onto it joins its crew; its top or bottom band is the gap
    // beside it, which places the session there.
    Widget memberRow(Crew c, List<AgentSession> members, AgentSession m) =>
        ZoneDropTarget<SessionDrag>(
          zoneOf: (d, f) {
            if (d.session.id == m.id) return null;
            final z = zoneAt(f);
            return z == DropZone.onto && d.session.crew?.id == c.id ? null : z;
          },
          onDrop: (d, z) {
            if (z == DropZone.onto) {
              placeInCrew(context, ref, d.session, c.id);
              return;
            }
            final rest = [
              for (final s in members)
                if (s.id != d.session.id) s.id,
            ];
            final i = rest.indexOf(m.id);
            final before = z == DropZone.before
                ? m.id
                : (i + 1 < rest.length ? rest[i + 1] : null);
            placeInCrew(
              context,
              ref,
              d.session,
              c.id,
              before: before,
              reorder: true,
            );
          },
          child: row(m, o!.membersOf(c)),
        );

    // RUNNING and ENDED: a member dropped on either leaves its crew.
    Widget sectionLabel(String label, double top, {Widget? trailing}) =>
        ZoneDropTarget<SessionDrag>(
          zoneOf: (d, _) => d.session.crew == null ? null : DropZone.onto,
          onDrop: (d, _) => leaveCrew(context, ref, d.session),
          child: Padding(
            key: Key('section-${label.toLowerCase()}'),
            padding: EdgeInsets.fromLTRB(t.sp, top, t.sp, t.sp * 0.75),
            child: trailing == null
                ? AgentsLabel(label)
                : Row(
                    children: [
                      Expanded(child: AgentsLabel(label)),
                      trailing,
                    ],
                  ),
          ),
        );

    Widget rows(List<AgentSession> list) => Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final (i, s) in ordered(list).indexed) ...[
          if (i > 0) SizedBox(height: t.sp * 0.25),
          indented(s, list, looseRow(s, list)),
        ],
      ],
    );

    Widget endedSection() {
      final dismissable = o == null ? 0 : dismissableEnded(o, ended).length;
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          ZoneDropTarget<SessionDrag>(
            zoneOf: (d, _) => d.session.crew == null ? null : DropZone.onto,
            onDrop: (d, _) => leaveCrew(context, ref, d.session),
            child: EndedLabel(
              count: ended.length,
              collapsed: prefs.ended,
              onToggle: () =>
                  ref.read(crewPrefsProvider.notifier).toggleEnded(),
              onDismissAll: dismissable == 0
                  ? null
                  : () => dismissEndedSessions(context, o!, ended),
            ),
          ),
          if (!prefs.ended) rows(ended),
        ],
      );
    }

    Widget header(Crew c, int i) {
      final next = i + 1 < crews.length ? crews[i + 1].id : null;
      final members = o!.membersOf(c);
      return CrewDraggable<CrewDrag>(
        data: CrewDrag(c),
        // The header's halves are the gaps above and below it, for a crew;
        // all of it is the crew, for a session.
        child: ZoneDropTarget<CrewDrag>(
          zoneOf: (d, f) => d.crew.id == c.id ? null : zoneAt(f, onto: false),
          onDrop: (d, z) => reorderCrew(
            context,
            ref,
            crews,
            d.crew,
            before: z == DropZone.before
                ? c.id
                : (next == d.crew.id
                      ? (i + 2 < crews.length ? crews[i + 2].id : null)
                      : next),
          ),
          child: ZoneDropTarget<SessionDrag>(
            zoneOf: (d, _) => d.session.crew?.id == c.id ? null : DropZone.onto,
            onDrop: (d, _) => placeInCrew(context, ref, d.session, c.id),
            child: SnapBackOutline(
              id: c.id,
              child: CrewHeader(
                crew: c,
                members: members,
                collapsed: prefs.collapsed(c.id),
                rollUp: rollUpOf(members, shown),
                selected: c.id == selectedCrew,
                nameField: renaming == c.id
                    ? CrewNameField(crew: c, style: crewHeaderNameStyle(t))
                    : null,
                onToggle: () =>
                    ref.read(crewPrefsProvider.notifier).toggleCrew(c.id),
                onOpen: () => context.go(Routes.agentCrew(c.id)),
                onMenu: (anchor) => showCrewMenu(
                  context,
                  ref,
                  anchor: anchor,
                  crew: c,
                  overview: o,
                ),
              ),
            ),
          ),
        ),
      );
    }

    Widget crewsSection() => Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        CrewsSectionLabel(
          count: crews.length,
          collapsed: prefs.section,
          onTap: () => ref.read(crewPrefsProvider.notifier).toggleSection(),
        ),
        for (final (i, c) in shownCrews.indexed) ...[
          if (i > 0) SizedBox(height: t.sp * 0.25),
          header(c, crews.indexOf(c)),
          if (membersShown(c) case final members when members.isNotEmpty) ...[
            SizedBox(height: t.sp * 0.25),
            CrewMembers(
              children: [
                for (final m in members)
                  indented(m, o!.membersOf(c), memberRow(c, members, m)),
              ],
            ),
          ],
        ],
      ],
    );

    return MouseRegion(
      onEnter: (_) => _hold(),
      onExit: (_) => _release(),
      child: SidebarFrame(
        footer: selection.isEmpty || o == null
            ? null
            : SelectionBar(overview: o),
        body: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: EdgeInsets.fromLTRB(
                t.sp * 1.5,
                sidebarTopInset(t),
                t.sp * 1.5,
                t.sp * 0.5,
              ),
              child: AgentsHeaderRow(
                current: overview,
                blocker: newSessionBlocker(async),
                onOverview: () => context.go(Routes.agents),
                onNewSession: () => launchAgentSession(context, ref),
              ),
            ),
            Expanded(
              child: ListView(
                padding: EdgeInsets.fromLTRB(t.sp, 0, t.sp, t.sp * 1.5),
                children: [
                  // Nothing is drawn as a crew until crews have loaded.
                  if (o == null) const SkeletonRows(rows: 3),
                  if (crews.isNotEmpty) crewsSection(),
                  // While a member is dragged RUNNING is always there to
                  // drop it on, even with no loose session running.
                  if (live.isNotEmpty || (dragging && crews.isNotEmpty)) ...[
                    sectionLabel('Running', t.sp * (crews.isEmpty ? 1.5 : 2)),
                    rows(live),
                  ],
                  if (ended.isNotEmpty) endedSection(),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// The sidebar's one header row (153e): "Agents", which opens the overview,
/// and a 28px ＋ that starts a session. It keeps the old heading's height,
/// so it stays on the rail's and the page titles' line.
class AgentsHeaderRow extends StatelessWidget {
  const AgentsHeaderRow({
    super.key,
    required this.current,
    required this.blocker,
    required this.onOverview,
    required this.onNewSession,
  });

  /// The overview is showing.
  final bool current;

  /// Why ＋ is off, or null when it starts a session.
  final String? blocker;
  final VoidCallback onOverview;
  final VoidCallback onNewSession;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final side = t.sp * 3.5;
    final on = blocker == null;
    final radius = BorderRadius.circular(t.rControl * 0.8);
    return SizedBox(
      height: side,
      child: Row(
        children: [
          Expanded(
            child: Semantics(
              button: true,
              selected: current,
              label: 'Agents, overview',
              excludeSemantics: true,
              onTap: onOverview,
              child: InkWell(
                key: const Key('agents-overview'),
                borderRadius: radius,
                onTap: onOverview,
                child: Padding(
                  padding: EdgeInsets.symmetric(horizontal: t.sp),
                  child: Align(
                    alignment: AlignmentDirectional.centerStart,
                    child: Text(
                      'Agents',
                      key: const Key('agents-heading'),
                      maxLines: 1,
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.uiSize,
                        fontWeight: FontWeight.w600,
                        color: t.text,
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
          Tooltip(
            message: blocker ?? 'New session $stormNewSessionChord',
            child: Semantics(
              button: true,
              enabled: on,
              label: 'New session',
              excludeSemantics: true,
              onTap: on ? onNewSession : null,
              child: Material(
                color: on ? t.accentSoft : t.surface2,
                borderRadius: radius,
                child: InkWell(
                  key: const Key('new-session'),
                  borderRadius: radius,
                  onTap: on ? onNewSession : null,
                  child: SizedBox.square(
                    dimension: side,
                    child: Icon(
                      LucideIcons.plus,
                      size: t.uiSize,
                      color: on ? t.accent : t.text3,
                    ),
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

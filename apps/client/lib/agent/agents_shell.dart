import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../router.dart';
import '../ui/breakpoints.dart';
import '../ui/controls.dart';
import '../ui/shell/sidebar_frame.dart';
import '../ui/states.dart' show SkeletonRows;
import '../ui/tokens.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart';
import 'agents_screen.dart';
import 'crew_changes.dart';
import 'crew_gestures.dart';
import 'crew_prefs.dart';
import 'crew_widgets.dart';

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
class AgentsShell extends StatelessWidget {
  const AgentsShell({super.key, required this.location, required this.child});

  final Uri location;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    if (!context.isExpanded) return child;
    return Scaffold(
      body: Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
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

/// Title, ＋ New session while a host is online, Overview, then CREWS,
/// RUNNING and ENDED (handoff §2.6; crews Round 4 §3). RUNNING and ENDED
/// list only sessions in no crew. Rows and crew headers drag, and ⌘/Ctrl-
/// and Shift-click select (138n); the sidebar is desktop only.
class AgentsSidebar extends ConsumerWidget {
  const AgentsSidebar({super.key, this.selected, this.selectedCrew});

  /// The open session, if any.
  final String? selected;

  /// The crew whose page is open, if any.
  final String? selectedCrew;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final o = ref.watch(agentOverviewProvider).value;
    final sessions = o?.sessions ?? const <AgentSession>[];
    final live = o?.looseLive ?? const <AgentSession>[];
    final ended = o?.looseEnded ?? const <AgentSession>[];
    final crews = o?.crews ?? const <Crew>[];
    final prefs = ref.watch(crewPrefsProvider).value ?? const CrewPrefs();
    final selection = ref.watch(sessionSelectionProvider);
    final dragging = ref.watch(crewDragActiveProvider);
    final renaming = ref.watch(crewRenameProvider);
    final overview = selected == null && selectedCrew == null;

    final shownCrews = prefs.section ? const <Crew>[] : crews;
    List<AgentSession> membersShown(Crew c) =>
        prefs.collapsed(c.id) ? const [] : liveFirst(o!.membersOf(c));
    // Every row, top to bottom: what a Shift-click range runs over.
    final order = [
      for (final c in shownCrews)
        for (final s in membersShown(c)) s.id,
      for (final s in live) s.id,
      for (final s in ended) s.id,
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

    Widget row(AgentSession s) => CrewDraggable<SessionDrag>(
      data: SessionDrag(s),
      child: SnapBackOutline(
        id: s.id,
        child: SessionRow(
          session: s,
          host: o?.hostName(s.hostId) ?? 'host',
          selected: s.id == selected || selection.contains(s.id),
          onTap: () => click(s),
        ),
      ),
    );

    // A loose row: another session dropped on it makes a crew of the two.
    Widget looseRow(AgentSession s) => ZoneDropTarget<SessionDrag>(
      zoneOf: (d, _) => d.session.id == s.id ? null : DropZone.onto,
      onDrop: (d, _) => makeCrew(context, ref, [s.id, d.session.id]),
      child: row(s),
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
          child: row(m),
        );

    // RUNNING and ENDED: a member dropped on either leaves its crew.
    Widget sectionLabel(String label, double top) =>
        ZoneDropTarget<SessionDrag>(
          zoneOf: (d, _) => d.session.crew == null ? null : DropZone.onto,
          onDrop: (d, _) => leaveCrew(context, ref, d.session),
          child: Padding(
            key: Key('section-${label.toLowerCase()}'),
            padding: EdgeInsets.fromLTRB(t.sp, top, t.sp, t.sp * 0.75),
            child: AgentsLabel(label),
          ),
        );

    Widget section(String label, List<AgentSession> list, double top) => Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        sectionLabel(label, top),
        for (final (i, s) in list.indexed) ...[
          if (i > 0) SizedBox(height: t.sp * 0.25),
          looseRow(s),
        ],
      ],
    );

    Widget header(Crew c, int i) {
      final next = i + 1 < crews.length ? crews[i + 1].id : null;
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
                members: o!.membersOf(c),
                collapsed: prefs.collapsed(c.id),
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
          header(c, i),
          if (membersShown(c) case final members when members.isNotEmpty) ...[
            SizedBox(height: t.sp * 0.25),
            CrewMembers(
              children: [for (final m in members) memberRow(c, members, m)],
            ),
          ],
        ],
      ],
    );

    return SidebarFrame(
      footer: selection.isEmpty || o == null ? null : SelectionBar(overview: o),
      body: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: EdgeInsets.fromLTRB(
              t.sp * 1.5,
              t.sp * 2,
              t.sp * 1.5,
              t.sp,
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Padding(
                  padding: EdgeInsets.fromLTRB(
                    t.sp,
                    t.sp * 0.75,
                    t.sp,
                    t.sp * 0.5,
                  ),
                  child: Text(
                    'Agents',
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.uiSize,
                      fontWeight: FontWeight.w600,
                      color: t.text,
                    ),
                  ),
                ),
                if (o != null && o.online.isNotEmpty) ...[
                  SizedBox(height: t.sp * 0.75),
                  StormButton.primary(
                    key: const Key('new-session'),
                    label: 'New session',
                    icon: LucideIcons.plus,
                    expand: true,
                    onPressed: () => launchAgentSession(context, ref),
                  ),
                ],
                SizedBox(height: t.sp * 0.75),
                _OverviewRow(
                  selected: overview && sessions.isNotEmpty,
                  current: overview,
                  onTap: () => context.go(Routes.agents),
                ),
              ],
            ),
          ),
          Expanded(
            child: ListView(
              padding: EdgeInsets.fromLTRB(t.sp, 0, t.sp, t.sp * 1.5),
              children: [
                // Nothing is drawn as a crew until crews have loaded.
                if (o == null) const SkeletonRows(rows: 3),
                if (crews.isNotEmpty) crewsSection(),
                // While a member is dragged RUNNING is always there to drop
                // it on, even with no loose session running.
                if (live.isNotEmpty || (dragging && crews.isNotEmpty))
                  section('Running', live, t.sp * (crews.isEmpty ? 1.5 : 2)),
                if (ended.isNotEmpty) section('Ended', ended, t.sp * 2),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _OverviewRow extends StatelessWidget {
  const _OverviewRow({
    required this.selected,
    required this.current,
    required this.onTap,
  });

  /// Filled: the overview is showing and has something on it.
  final bool selected;

  /// In `text`: no session is open.
  final bool current;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final radius = BorderRadius.circular(t.rControl);
    return Semantics(
      button: true,
      selected: current,
      label: 'Overview',
      excludeSemantics: true,
      onTap: onTap,
      child: Material(
        color: selected ? t.surface2 : Colors.transparent,
        borderRadius: radius,
        child: InkWell(
          key: const Key('agents-overview'),
          borderRadius: radius,
          onTap: onTap,
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 1.25,
              vertical: t.sp * 0.875,
            ),
            child: Text(
              'Overview',
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.codeSize,
                color: current ? t.text : t.text2,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

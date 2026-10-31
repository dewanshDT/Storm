import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../ui/controls.dart';
import '../ui/settings/settings_widgets.dart' show promptForText;
import '../ui/surfaces.dart';
import '../ui/tokens.dart';
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart';
import 'crew_changes.dart';
import 'crew_screen.dart' show endAllRunning, watchEndAllPlan;
import 'crew_widgets.dart';
import 'session_crew.dart' show sessionReadsStorm;
import 'spawn_models.dart' show kMovesWithParent;

/// The phone's crew sheets (decision 138n, handoff §3 Phone, 4k and 4l): a
/// long-press on a session raises its Crew and Role, two independent
/// sections; End all running is a sheet rather than the desk's banner.

/// The long-press sheet: the session, then where it is listed and its
/// label, each changed on its own.
Future<void> showSessionCrewSheet(
  BuildContext context,
  WidgetRef ref,
  AgentSession session,
) => showStormSheet<void>(
  context: context,
  title: 'Crew and role',
  heightFactor: 0.86,
  builder: (_) => Column(
    key: const Key('session-crew-sheet'),
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      _SessionHeading(session: session),
      CrewRoleSections(session: session),
    ],
  ),
);

class _SessionHeading extends ConsumerWidget {
  const _SessionHeading({required this.session});

  final AgentSession session;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final o = ref.watch(agentOverviewProvider).value;
    final s = o?.byId(session.id) ?? session;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.baseline,
      textBaseline: TextBaseline.alphabetic,
      children: [
        Flexible(
          child: Text(
            s.displayName,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: _titleStyle(t),
          ),
        ),
        SizedBox(width: t.sp),
        Flexible(
          child: Text(
            [
              s.statusLabel,
              o?.hostName(s.hostId) ?? 'host',
              s.workspace,
            ].join(' · '),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: agentMono(t),
          ),
        ),
      ],
    );
  }
}

TextStyle _titleStyle(StormTokens t) => TextStyle(
  fontFamily: StormTokens.sansFamily,
  fontSize: t.bodySize * 1.0625,
  fontWeight: FontWeight.w600,
  color: t.text,
);

/// A session's Crew and Role, as the long-press and details sheets show
/// them. Picking a crew moves it and leaves its role alone; picking a role
/// sets only the label (138f), so a session in no crew can carry one.
class CrewRoleSections extends ConsumerStatefulWidget {
  const CrewRoleSections({super.key, required this.session});

  /// The session as the caller had it; the list's copy wins once loaded.
  final AgentSession session;

  @override
  ConsumerState<CrewRoleSections> createState() => _CrewRoleSectionsState();
}

class _CrewRoleSectionsState extends ConsumerState<CrewRoleSections> {
  void _reload(String id) {
    if (mounted) ref.invalidate(sessionDetailProvider(id));
  }

  Future<void> _moveTo(AgentSession s, String? crewId) async {
    await sendCrewChange(
      context,
      what: 'move ${s.displayName}',
      patch: movePatch(s.id, crewId),
      send: (api) => api.moveToCrew(s.id, crewId),
    );
    _reload(s.id);
  }

  Future<void> _newCrew(AgentSession s) async {
    final name = (await promptForText(
      context,
      title: 'New crew',
      hint: 'Crew name',
      initial: 'New crew',
    ))?.trim();
    if (name == null || name.isEmpty || !mounted) return;
    await sendCrewChange(
      context,
      what: 'make the crew',
      patch: newCrewPatch(name, [s.id]),
      send: (api) => api.createCrew(name, sessionIds: [s.id]),
    );
    _reload(s.id);
  }

  Future<void> _label(AgentSession s, RoleNote? r) async {
    await sendLabelChange(context, s.id, r);
    _reload(s.id);
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final o = ref.watch(agentOverviewProvider).value;
    final roles = ref.watch(rolesProvider).value ?? const <RoleNote>[];
    final s = o?.byId(widget.session.id) ?? widget.session;
    final crews = o?.crews;
    final crewId = s.crew?.id;
    final role = s.role;
    final launch = s.launchRole;
    final groups = roleGroups(roles);

    final children = <Widget>[];
    // A server before decision 138 has no crews to list.
    if (crews != null && s.spawn.isChild) {
      // 145i: a child is in its root's crew and goes with it.
      children.addAll([
        _Group('Crew', "where it's listed"),
        _Note(
          '${o?.crewOf(s)?.name ?? 'Not in a crew'} · $kMovesWithParent',
          key: const Key('sheet-crew-moves-with-parent'),
        ),
      ]);
    } else if (crews != null) {
      children.addAll([
        _Group('Crew', "where it's listed"),
        _Item(
          key: const Key('sheet-crew-none'),
          label: 'Not in a crew',
          checked: crewId == null,
          onTap: () => _moveTo(s, null),
        ),
        for (final c in crews)
          _Item(
            key: Key('sheet-crew-${c.id}'),
            label: c.name,
            leading: CrewSquare(accent: c.accent, size: t.sp * 1.5),
            checked: crewId == c.id,
            onTap: crewId == c.id ? null : () => _moveTo(s, c.id),
          ),
        _Item(
          key: const Key('sheet-crew-new'),
          label: 'New crew…',
          leading: Icon(LucideIcons.plus, size: t.uiSize * 1.05, color: t.text),
          onTap: () => _newCrew(s),
        ),
      ]);
    }
    children.addAll([
      _Group('Role', 'a label, separate from crew'),
      if (launch != null)
        _Note(
          '${s.displayName} launched as ${launch.title ?? 'a role'} and '
          'keeps that note. Changing the role here only changes the label.',
        )
      else if (!sessionReadsStorm(o, s))
        _Note(
          "${providerLabel(s.provider)} can't read Storm notes, so a role "
          'here is only a label.',
        ),
      _Item(
        key: const Key('sheet-role-none'),
        label: 'None',
        detail: 'A plain agent',
        checked: role == null,
        onTap: role == null ? null : () => _label(s, null),
      ),
      for (final MapEntry(key: folder, value: notes) in groups.entries) ...[
        if (groups.length > 1)
          Padding(
            padding: EdgeInsets.only(top: t.sp * 1.25, bottom: t.sp * 0.25),
            child: AgentsLabel(roleFolderLabel(folder)),
          ),
        for (final r in notes)
          _Item(
            key: Key('sheet-role-${r.noteId}'),
            label: r.title,
            detail: r.summary,
            checked: role?.noteId == r.noteId && role?.vaultId == r.vaultId,
            onTap: role?.noteId == r.noteId ? null : () => _label(s, r),
          ),
      ],
    ]);
    return Column(
      key: const Key('crew-role-sections'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: children,
    );
  }
}

/// "CREW  where it's listed": a section's heading and what it means.
class _Group extends StatelessWidget {
  const _Group(this.label, this.detail);

  final String label;
  final String detail;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.only(top: t.sp * 1.5, bottom: t.sp * 0.25),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.baseline,
        textBaseline: TextBaseline.alphabetic,
        children: [
          AgentsLabel(label),
          SizedBox(width: t.sp),
          Flexible(
            child: Text(
              detail,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontFamily: StormTokens.sansFamily,
                fontSize: t.labelSize * 1.05,
                color: t.text3,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _Note extends StatelessWidget {
  const _Note(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Padding(
      padding: EdgeInsets.only(top: t.sp * 0.25, bottom: t.sp * 0.5),
      child: Text(
        text,
        style: TextStyle(
          fontFamily: StormTokens.sansFamily,
          fontSize: t.labelSize * 1.09,
          height: 1.45,
          color: t.text3,
        ),
      ),
    );
  }
}

/// A sheet row: 44 tall at least, a hairline under it, a ✓ when chosen.
class _Item extends StatelessWidget {
  const _Item({
    super.key,
    required this.label,
    this.detail,
    this.leading,
    this.checked = false,
    this.onTap,
  });

  final String label;
  final String? detail;
  final Widget? leading;
  final bool checked;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Semantics(
      button: true,
      selected: checked,
      label: detail == null ? label : '$label, $detail',
      excludeSemantics: true,
      onTap: onTap,
      child: InkWell(
        onTap: onTap,
        child: Container(
          constraints: BoxConstraints(minHeight: t.sp * 5.5),
          padding: EdgeInsets.symmetric(vertical: t.sp * 1.125),
          decoration: BoxDecoration(
            border: Border(
              bottom: BorderSide(color: t.border, width: t.bw),
            ),
          ),
          child: Row(
            children: [
              if (leading != null) ...[leading!, SizedBox(width: t.sp * 1.25)],
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      label,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.uiSize * 1.05,
                        color: t.text,
                      ),
                    ),
                    if (detail != null && detail!.isNotEmpty) ...[
                      SizedBox(height: t.sp * 0.125),
                      Text(
                        detail!,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontFamily: StormTokens.sansFamily,
                          fontSize: t.labelSize * 1.09,
                          color: t.text3,
                        ),
                      ),
                    ],
                  ],
                ),
              ),
              if (checked)
                Icon(
                  LucideIcons.check,
                  key: const Key('sheet-check'),
                  size: t.codeSize,
                  color: t.accent,
                ),
            ],
          ),
        ),
      ),
    );
  }
}

/// End all running as a sheet (138k, 4l): what it stops, what it can't
/// reach now and skips, what has already ended; then End n sessions.
Future<void> showEndAllSheet(
  BuildContext context,
  WidgetRef ref, {
  required Crew crew,
  required List<AgentSession> members,
  required AgentOverview overview,
}) => showStormSheet<void>(
  context: context,
  title: crew.name,
  heightFactor: 0.86,
  builder: (sheet) => _EndAllSheet(
    crew: crew,
    members: members,
    overview: overview,
    onDone: () => Navigator.of(sheet).pop(),
  ),
);

class _EndAllSheet extends ConsumerStatefulWidget {
  const _EndAllSheet({
    required this.crew,
    required this.members,
    required this.overview,
    required this.onDone,
  });

  final Crew crew;
  final List<AgentSession> members;
  final AgentOverview overview;
  final VoidCallback onDone;

  @override
  ConsumerState<_EndAllSheet> createState() => _EndAllSheetState();
}

class _EndAllSheetState extends ConsumerState<_EndAllSheet> {
  bool _ending = false;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final o = widget.overview;
    final plan = watchEndAllPlan(ref, o, widget.members);
    final n = plan.stops.length;
    Widget group(String label, Key key, List<Widget> items) => Column(
      key: key,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [_Group(label, ''), ...items],
    );
    return Column(
      key: const Key('end-all-sheet'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text('End all running?', style: _titleStyle(t)),
        SizedBox(height: t.sp * 0.75),
        _Note(
          'The agents and everything they started are stopped on their '
          'hosts.',
        ),
        if (plan.stops.isNotEmpty)
          group('Stops', const Key('end-all-stops'), [
            for (final s in plan.stops)
              _Item(label: s.displayName, detail: o.hostName(s.hostId)),
          ]),
        if (plan.reaches.isNotEmpty)
          group('What they started', const Key('end-all-reaches'), [
            for (final (child, parent) in plan.reaches)
              _Item(label: child, detail: 'started by $parent'),
          ]),
        if (plan.unreachable.isNotEmpty)
          group("Can't reach now", const Key('end-all-unreachable'), [
            for (final s in plan.unreachable)
              _Item(
                label: s.displayName,
                detail:
                    '${o.hostName(s.hostId)} is offline. Skipped: end it '
                    'once the host is back.',
              ),
          ]),
        if (plan.ended.isNotEmpty)
          group('Already ended', const Key('end-all-ended'), [
            for (final s in plan.ended) _Item(label: s.displayName),
          ]),
        SizedBox(height: t.sp * 1.75),
        StormButton.danger(
          key: const Key('end-all-confirm'),
          label: n == 1 ? 'End 1 session' : 'End $n sessions',
          expand: true,
          onPressed: _ending || !plan.ready
              ? null
              : () async {
                  setState(() => _ending = true);
                  await endAllRunning(
                    context,
                    ref,
                    plan.stops,
                    descendants: plan.descendants ?? const {},
                  );
                  if (mounted) widget.onDone();
                },
        ),
        SizedBox(height: t.sp * 0.5),
        Center(
          child: StormButton.text(
            key: const Key('end-all-cancel'),
            label: 'Cancel',
            onPressed: widget.onDone,
          ),
        ),
      ],
    );
  }
}

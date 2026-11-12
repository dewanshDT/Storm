import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../ui/controls.dart';
import '../ui/settings/settings_widgets.dart' show promptForText;
import '../ui/surfaces.dart';
import '../ui/tokens.dart';
import '../ui/widgets.dart' show PopoverItem, PopoverTone, StormCheckbox;
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart' show agentKey, agentMono;
import 'crew_changes.dart';
import 'crew_gestures.dart' show moveSessionsTo;
import 'crew_sheets.dart' show showSessionCrewSheet;
import 'crew_widgets.dart' show CrewSquare;
import 'spawn_models.dart' show kMovesWithParent;

/// Making a crew without gestures (153e gap 5, handoff §2): a session
/// row's ··· has Add to crew…, a crew's menu has Add sessions…. Both end in
/// the same crew changes a drag makes.

enum _RowAction { open, addToCrew, changeRole }

/// A session row's ··· menu: Open, Add to crew…, Change role….
Future<void> showSessionMenu(
  BuildContext context,
  WidgetRef ref, {
  required GlobalKey anchor,
  required AgentSession session,
  required AgentOverview overview,
  required VoidCallback onOpen,
}) async {
  final t = context.tokens;
  final child = session.spawn.isChild;
  final picked = await showStormPopover<_RowAction>(
    context: context,
    anchorKey: anchor,
    width: t.sp * 25,
    builder: (popover) => StormPopover(
      key: const Key('session-menu'),
      padding: t.sp * 0.75,
      children: [
        PopoverItem(
          key: const Key('session-action-open'),
          label: 'Open',
          onTap: () => Navigator.pop(popover, _RowAction.open),
        ),
        // A child is listed in its root's crew and goes with it (145i).
        PopoverItem(
          key: const Key('session-action-add-to-crew'),
          label: 'Add to crew…',
          subtitle: child ? kMovesWithParent : null,
          tone: child ? PopoverTone.muted : PopoverTone.normal,
          trailing: Icon(
            LucideIcons.chevron_right,
            size: t.labelSize,
            color: t.text3,
          ),
          onTap: child
              ? null
              : () => Navigator.pop(popover, _RowAction.addToCrew),
        ),
        PopoverItem(
          key: const Key('session-action-change-role'),
          label: 'Change role…',
          onTap: () => Navigator.pop(popover, _RowAction.changeRole),
        ),
      ],
    ),
  );
  if (picked == null || !context.mounted) return;
  switch (picked) {
    case _RowAction.open:
      onOpen();
    case _RowAction.addToCrew:
      await showAddToCrew(
        context,
        anchor: anchor,
        session: session,
        overview: overview,
      );
    case _RowAction.changeRole:
      await showSessionCrewSheet(context, ref, session);
  }
}

/// The crews to put [session] in, its own ticked, then Not in a crew and
/// New crew….
Future<void> showAddToCrew(
  BuildContext context, {
  required GlobalKey anchor,
  required AgentSession session,
  required AgentOverview overview,
}) async {
  final t = context.tokens;
  final crews = overview.crews ?? const <Crew>[];
  final current = overview.crewOf(session)?.id;
  final picked = await showStormPopover<(String?,)>(
    context: context,
    anchorKey: anchor,
    width: t.sp * 27.5,
    builder: (popover) => StormPopover(
      key: const Key('add-to-crew-menu'),
      padding: t.sp * 0.75,
      children: [
        for (final c in crews)
          PopoverItem(
            key: Key('add-to-crew-${c.id}'),
            label: c.name,
            leading: CrewSquare(accent: c.accent, size: t.sp * 1.25),
            trailing: c.id == current
                ? Icon(LucideIcons.check, size: t.labelSize, color: t.accent)
                : null,
            onTap: () => Navigator.pop(popover, (c.id,)),
          ),
        PopoverItem(
          key: const Key('add-to-crew-none'),
          label: 'Not in a crew',
          trailing: current == null
              ? Icon(LucideIcons.check, size: t.labelSize, color: t.accent)
              : null,
          onTap: () => Navigator.pop(popover, (null,)),
        ),
        const PopoverDivider(),
        PopoverItem(
          key: const Key('add-to-crew-new'),
          label: 'New crew…',
          tone: PopoverTone.accent,
          onTap: () => Navigator.pop(popover, ('',)),
        ),
      ],
    ),
  );
  if (picked == null || !context.mounted) return;
  final to = picked.$1;
  if (to == '') {
    final name = (await promptForText(
      context,
      title: 'New crew',
      hint: 'Crew name',
      initial: 'New crew',
    ))?.trim();
    if (name == null || name.isEmpty || !context.mounted) return;
    final ids = overview.rootIdsOf([session.id]);
    await sendCrewChange(
      context,
      what: 'make the crew',
      patch: newCrewPatch(name, ids),
      send: (api) => api.createCrew(name, sessionIds: ids),
    );
    return;
  }
  if (to == current) return;
  await moveSessionsTo(context, overview, [session.id], to);
}

/// What Add sessions… offers [crew]: the live sessions not in it, each a
/// tree's root, since a child goes with its tree (145i).
List<AgentSession> addableTo(AgentOverview o, Crew crew) => [
  for (final s in o.live)
    if (!s.spawn.isChild && o.crewOf(s)?.id != crew.id) s,
];

/// Add sessions…: a checklist of [addableTo], as a popover at [anchor] on a
/// wide screen and a sheet without one.
Future<void> showAddSessions(
  BuildContext context, {
  required Crew crew,
  required AgentOverview overview,
  GlobalKey? anchor,
}) async {
  final t = context.tokens;
  Widget list(BuildContext inner, {required bool sheet}) => AddSessionsList(
    crew: crew,
    overview: overview,
    sheet: sheet,
    onAdd: (ids) => Navigator.pop(inner, ids),
  );
  final picked = anchor == null
      ? await showStormSheet<List<String>>(
          context: context,
          title: 'Add to ${crew.name}',
          heightFactor: 0.86,
          builder: (inner) => list(inner, sheet: true),
        )
      : await showStormPopover<List<String>>(
          context: context,
          anchorKey: anchor,
          width: t.sp * 30,
          builder: (inner) => StormPopover(
            padding: t.sp * 1.25,
            children: [list(inner, sheet: false)],
          ),
        );
  if (picked == null || picked.isEmpty || !context.mounted) return;
  await moveSessionsTo(context, overview, picked, crew.id);
}

class AddSessionsList extends StatefulWidget {
  const AddSessionsList({
    super.key,
    required this.crew,
    required this.overview,
    required this.onAdd,
    this.sheet = false,
  });

  final Crew crew;
  final AgentOverview overview;
  final ValueChanged<List<String>> onAdd;
  final bool sheet;

  @override
  State<AddSessionsList> createState() => _AddSessionsListState();
}

class _AddSessionsListState extends State<AddSessionsList> {
  final _ticked = <String>{};

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final o = widget.overview;
    final sessions = addableTo(o, widget.crew);
    final n = _ticked.length;
    String where(AgentSession s) {
      final c = o.crewOf(s);
      return c == null ? s.workspace : 'in ${c.name}';
    }

    void toggle(String id) => setState(() {
      if (!_ticked.remove(id)) _ticked.add(id);
    });

    return Column(
      key: const Key('add-sessions'),
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (!widget.sheet)
          Padding(
            padding: EdgeInsets.fromLTRB(t.sp * 0.75, 0, t.sp * 0.75, t.sp),
            child: Text(
              'Add to ${widget.crew.name}'.toUpperCase(),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: agentKey(t),
            ),
          ),
        if (sessions.isEmpty)
          Padding(
            padding: EdgeInsets.all(t.sp * 0.75),
            child: Text(
              'Every running session is in this crew.',
              style: agentMono(t, color: t.text2),
            ),
          ),
        for (final s in sessions)
          InkWell(
            key: Key('add-session-${s.id}'),
            borderRadius: BorderRadius.circular(t.rControl * 0.8),
            onTap: () => toggle(s.id),
            child: Padding(
              padding: EdgeInsets.symmetric(
                horizontal: t.sp * 0.75,
                vertical: t.sp * 0.875,
              ),
              child: Row(
                children: [
                  StormCheckbox(
                    value: _ticked.contains(s.id),
                    size: t.sp * 2,
                    onChanged: (_) => toggle(s.id),
                  ),
                  SizedBox(width: t.sp),
                  Expanded(
                    child: Text(
                      s.displayName,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontFamily: StormTokens.sansFamily,
                        fontSize: t.codeSize,
                        color: t.text,
                      ),
                    ),
                  ),
                  SizedBox(width: t.sp),
                  Text(where(s), style: agentMono(t)),
                ],
              ),
            ),
          ),
        SizedBox(height: t.sp),
        StormButton.primary(
          key: const Key('add-sessions-confirm'),
          label: switch (n) {
            0 => 'Add',
            1 => 'Add 1 session',
            _ => 'Add $n sessions',
          },
          expand: true,
          onPressed: n == 0
              ? null
              : () => widget.onAdd([
                  for (final s in sessions)
                    if (_ticked.contains(s.id)) s.id,
                ]),
        ),
      ],
    );
  }
}

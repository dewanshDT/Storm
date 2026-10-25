import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/legacy.dart';

import '../state/app_state.dart' show rootMessengerKey;
import '../ui/accents.dart';
import '../ui/states.dart' show describeFailure;
import '../ui/tokens.dart';
import 'agent_api.dart';
import 'agent_models.dart';
import 'agent_state.dart';

/// Crew and label changes as this device makes them (decision 138j, 138c):
/// each is drawn at once over the list ([CrewPatch]), kept while the server
/// is asked, and taken off again when it answers — after the list has been
/// re-read on success, at once on a refusal, which snaps the row back with
/// an outline and a toast naming the reason. Other devices see the change on
/// their next poll. A crew the change emptied is offered back with Undo.

/// A toast on the app's one messenger, so it outlives the screen that raised
/// it (a crew page whose crew was just removed). [onUndo] adds Undo.
void showUndoToast(
  String message, {
  VoidCallback? onUndo,
  ScaffoldMessengerState? fallback,
}) {
  (rootMessengerKey.currentState ?? fallback)?.showSnackBar(
    SnackBar(
      content: Text(message),
      action: onUndo == null
          ? null
          : SnackBarAction(label: 'Undo', onPressed: onUndo),
      // An Undo would otherwise stay until dismissed; it is offered for a
      // while, as a removal it undoes is cheap to lose (decision 138,
      // Accepted).
      persist: false,
      duration: onUndo == null
          ? const Duration(seconds: 4)
          : const Duration(seconds: 8),
    ),
  );
}

/// Rows and crews a refused change just put back, each with a count that
/// moves on every snap. An entry lasts until a row has shown it, so a row
/// drawn in its old place only after the snap still flashes; the next change
/// clears what is left.
final crewSnapBackProvider = StateProvider<Map<String, int>>((ref) => const {});

const _snapFor = Duration(milliseconds: 1600);
var _snaps = 0;

void _snapBack(ProviderContainer c, CrewPatch p) {
  final ids = {...p.sessions, ...p.crews};
  if (ids.isEmpty) return;
  final n = ++_snaps;
  c.read(crewSnapBackProvider.notifier).state = {for (final id in ids) id: n};
}

/// The 1.5px `accent` outline a row or header wears for a moment after a
/// refused change put it back (handoff §5, 4i).
class SnapBackOutline extends ConsumerStatefulWidget {
  const SnapBackOutline({super.key, required this.id, required this.child});

  final String id;
  final Widget child;

  @override
  ConsumerState<SnapBackOutline> createState() => _SnapBackOutlineState();
}

class _SnapBackOutlineState extends ConsumerState<SnapBackOutline>
    with SingleTickerProviderStateMixin {
  late final _flash = AnimationController(vsync: this, duration: _snapFor);
  int? _played;

  @override
  void dispose() {
    _flash.dispose();
    super.dispose();
  }

  void _play(int n) {
    _played = n;
    // Started after this frame: the controller would rebuild during it.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _flash.forward(from: 0).whenComplete(() {
        if (!mounted) return;
        final snaps = ref.read(crewSnapBackProvider.notifier);
        if (snaps.state[widget.id] == n) {
          snaps.state = {...snaps.state}..remove(widget.id);
        }
      });
    });
  }

  @override
  Widget build(BuildContext context) {
    final n = ref.watch(crewSnapBackProvider.select((m) => m[widget.id]));
    if (n != null && n != _played) _play(n);
    final t = context.tokens;
    return AnimatedBuilder(
      animation: _flash,
      child: widget.child,
      builder: (context, child) {
        if (!_flash.isAnimating) return child!;
        final v = _flash.value;
        final fade = v < 0.75 ? 1.0 : (1 - v) * 4;
        return DecoratedBox(
          key: Key('snap-back-${widget.id}'),
          position: DecorationPosition.foreground,
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(t.rControl),
            border: Border.all(
              color: t.accent.withValues(alpha: fade),
              width: t.bw * 1.5,
            ),
          ),
          child: child,
        );
      },
    );
  }
}

// ---- what each change looks like before the server answers --------------

List<AgentSession> _membersById(AgentOverview o, String crewId) => [
  for (final s in o.sessions)
    if (s.crew?.id == crewId) s,
]..sort((a, b) => a.crew!.order.compareTo(b.crew!.order));

/// Drops those of [touched] no session is in any more, as the server does in
/// the same change (138c). A crew already empty and untouched stays.
AgentOverview _dropEmptied(AgentOverview o, Set<String> touched) {
  final crews = o.crews;
  if (crews == null || touched.isEmpty) return o;
  return o.copyWith(
    crews: [
      for (final c in crews)
        if (!touched.contains(c.id) ||
            o.sessions.any((s) => s.crew?.id == c.id))
          c,
    ],
  );
}

AgentOverview _move(AgentOverview o, String id, String? to, String? before) {
  final s = o.byId(id);
  if (s == null) return o;
  final from = s.crew?.id;
  final order = to == null
      ? const <String>[]
      : [
          for (final m in _membersById(o, to))
            if (m.id != id) m.id,
        ];
  if (to != null) {
    final at = before == null ? -1 : order.indexOf(before);
    order.insert(at < 0 ? order.length : at, id);
  }
  final moved = o.copyWith(
    sessions: [
      for (final m in o.sessions)
        if (m.id == id)
          m.withCrew(
            to == null ? null : SessionCrew(id: to, order: order.indexOf(id)),
          )
        else if (to != null && m.crew?.id == to)
          m.withCrew(SessionCrew(id: to, order: order.indexOf(m.id)))
        else
          m,
    ],
  );
  return from == null || from == to ? moved : _dropEmptied(moved, {from});
}

/// [sessionId] into [crewId] in front of [before] (last when null), or out
/// of any crew when [crewId] is null.
CrewPatch movePatch(String sessionId, String? crewId, {String? before}) =>
    CrewPatch(
      (o) => _move(o, sessionId, crewId, before),
      sessions: {sessionId},
    );

/// Several sessions into [crewId], in turn.
CrewPatch moveAllPatch(List<String> sessionIds, String? crewId) =>
    CrewPatch((o) {
      var out = o;
      for (final id in sessionIds) {
        out = _move(out, id, crewId, null);
      }
      return out;
    }, sessions: {...sessionIds});

/// The colour the server gives a crew made without one: the first palette
/// word no crew uses, cycling once all are taken (138-1).
String nextCrewColor(List<Crew> crews) {
  final palette = [
    for (final a in Accent.values)
      if (a != Accent.none) a.name,
  ];
  return palette.firstWhere(
    (c) => !crews.any((crew) => crew.color == c),
    orElse: () => palette[crews.length % palette.length],
  );
}

var _made = 0;

/// A crew that does not exist yet: [name] of [sessionIds] in that order, at
/// [index] among the crews (last when null), in [color] or the next free one.
CrewPatch newCrewPatch(
  String name,
  List<String> sessionIds, {
  String? color,
  int? index,
}) {
  final id = 'pending-crew-${_made++}';
  return CrewPatch(
    (o) {
      final crews = [...?o.crews];
      final at = index == null ? crews.length : index.clamp(0, crews.length);
      crews.insert(
        at,
        Crew(
          id: id,
          name: name,
          color: color ?? nextCrewColor(crews),
          order: at,
        ),
      );
      var out = o.copyWith(
        crews: [
          for (final (i, c) in crews.indexed)
            Crew(id: c.id, name: c.name, color: c.color, order: i),
        ],
      );
      for (final s in sessionIds) {
        out = _move(out, s, id, null);
      }
      return out;
    },
    sessions: {...sessionIds},
    crews: {id},
  );
}

/// [crewId] renamed or recoloured.
CrewPatch crewPatch(String crewId, {String? name, String? color}) => CrewPatch(
  (o) => o.copyWith(
    crews: [
      for (final c in o.crews ?? const <Crew>[])
        c.id == crewId
            ? Crew(
                id: c.id,
                name: name ?? c.name,
                color: color ?? c.color,
                order: c.order,
              )
            : c,
    ],
  ),
  crews: {crewId},
);

/// The crews in the order of [ids]; [moved] is the one the person moved,
/// which a refusal outlines.
CrewPatch crewOrderPatch(List<String> ids, {String? moved}) => CrewPatch((o) {
  final byId = {for (final c in o.crews ?? const <Crew>[]) c.id: c};
  final listed = [for (final id in ids) ?byId.remove(id), ...byId.values];
  return o.copyWith(
    crews: [
      for (final (i, c) in listed.indexed)
        Crew(id: c.id, name: c.name, color: c.color, order: i),
    ],
  );
}, crews: moved == null ? {...ids} : {moved});

/// [sessionId]'s label set to [role], or cleared.
CrewPatch rolePatch(String sessionId, RoleRef? role) => CrewPatch(
  (o) => o.copyWith(
    sessions: [
      for (final s in o.sessions) s.id == sessionId ? s.withRole(role) : s,
    ],
  ),
  sessions: {sessionId},
);

// ---- sending ------------------------------------------------------------

/// Where a removed crew stood, so Undo can put it back (138c): its name,
/// colour and place among the crews, and the sessions it lost that still
/// exist.
class RemovedCrew {
  const RemovedCrew({
    required this.crew,
    required this.index,
    this.next,
    this.members = const [],
  });

  final Crew crew;
  final int index;

  /// The crew listed after it then, which it goes back in front of.
  final String? next;

  /// For a move, the session to move back.
  final List<String> members;
}

/// [gone] as the list [before] the change had it.
RemovedCrew removedCrewFrom(AgentOverview? before, Crew gone) {
  final crews = before?.crews ?? const <Crew>[];
  final i = crews.indexWhere((c) => c.id == gone.id);
  final members = before == null
      ? const <String>[]
      : [for (final s in _membersById(before, gone.id)) s.id];
  return RemovedCrew(
    crew: gone,
    index: i < 0 ? crews.length : i,
    next: i >= 0 && i + 1 < crews.length ? crews[i + 1].id : null,
    members: members,
  );
}

Future<void> _reload(ProviderContainer c) async {
  // Listened to for the read, so an otherwise unwatched list isn't
  // disposed half way through it.
  final sub = c.listen(agentOverviewProvider, (_, _) {});
  try {
    c.invalidate(agentOverviewProvider);
    await c.read(agentOverviewProvider.future);
  } catch (_) {
    // The list draws its own unreachable state.
  } finally {
    sub.close();
  }
}

List<Crew> _removedBy(Object? result) => switch (result) {
  CrewChange(:final removedCrews) => removedCrews,
  _ => const [],
};

/// Sends one change, drawing [patch] until the server answers. A refusal
/// takes [patch] off at once, outlines what it touched and toasts
/// "Couldn't [what]: {reason}". Crews the change removed are toasted with
/// Undo, [why] naming the reason ("its last session left it"); [removed]
/// finds them in the answer, a [CrewChange]'s by default.
///
/// Returns the server's answer, or null when there was no session or the
/// change was refused.
Future<T?> sendCrewChange<T>(
  BuildContext context, {
  required String what,
  required Future<T> Function(AgentApi api) send,
  CrewPatch? patch,
  String why = 'its last session left it',
  List<Crew> Function(T result) removed = _removedBy,
}) async {
  final c = ProviderScope.containerOf(context, listen: false);
  final api = c.read(agentApiFactoryProvider)?.call();
  if (api == null) return null;
  final before = c.read(agentOverviewProvider).value;
  final messenger = ScaffoldMessenger.maybeOf(context);
  final patches = c.read(crewPatchesProvider.notifier);
  void unpatch() {
    if (patch != null) {
      patches.update(
        (l) => [
          for (final p in l)
            if (!identical(p, patch)) p,
        ],
      );
    }
  }

  c.read(crewSnapBackProvider.notifier).state = const {};
  if (patch != null) patches.update((l) => [...l, patch]);
  final T result;
  try {
    result = await send(api);
  } catch (e) {
    unpatch();
    if (patch != null) _snapBack(c, patch);
    showUndoToast("Couldn't $what: ${describeFailure(e)}", fallback: messenger);
    c.invalidate(agentOverviewProvider);
    return null;
  } finally {
    api.dispose();
  }
  await _reload(c);
  unpatch();
  for (final id in patch?.sessions ?? const <String>{}) {
    c.invalidate(sessionDetailProvider(id));
  }
  for (final gone in removed(result)) {
    announceRemovedCrew(
      c,
      removedCrewFrom(before, gone),
      why: why,
      fallback: messenger,
    );
  }
  return result;
}

/// "{crew} was removed: [why]. Undo" (138c).
void announceRemovedCrew(
  ProviderContainer c,
  RemovedCrew gone, {
  required String why,
  ScaffoldMessengerState? fallback,
}) => showUndoToast(
  '${gone.crew.name} was removed: $why.',
  onUndo: () => restoreCrew(c, gone, fallback: fallback),
  fallback: fallback,
);

/// Undo for a removed crew: the same name, colour and place, and for a move
/// the session moved back. It is a new crew with a new id (plan §7).
Future<void> restoreCrew(
  ProviderContainer c,
  RemovedCrew gone, {
  ScaffoldMessengerState? fallback,
}) async {
  final api = c.read(agentApiFactoryProvider)?.call();
  if (api == null) return;
  final now = c.read(agentOverviewProvider).value;
  final alive = [
    for (final id in gone.members)
      if (now?.byId(id) != null) id,
  ];
  final crews = now?.crews ?? const <Crew>[];
  final nextAt = gone.next == null
      ? -1
      : crews.indexWhere((x) => x.id == gone.next);
  final index = nextAt >= 0 ? nextAt : gone.index.clamp(0, crews.length);
  final patch = newCrewPatch(
    gone.crew.name,
    alive,
    color: gone.crew.color,
    index: index,
  );
  final patches = c.read(crewPatchesProvider.notifier);
  patches.update((l) => [...l, patch]);
  try {
    final change = await api.createCrew(
      gone.crew.name,
      color: gone.crew.color,
      sessionIds: alive,
    );
    final made = change.crew;
    if (made != null) {
      final ids = [
        for (final x
            in await api.crews()
              ..sort((a, b) => a.order.compareTo(b.order)))
          if (x.id != made.id) x.id,
      ];
      final at = gone.next == null ? -1 : ids.indexOf(gone.next!);
      ids.insert(at >= 0 ? at : gone.index.clamp(0, ids.length), made.id);
      if (ids.last != made.id) await api.reorderCrews(ids);
    }
    await _reload(c);
    for (final id in alive) {
      c.invalidate(sessionDetailProvider(id));
    }
  } catch (e) {
    showUndoToast(
      "Couldn't bring back ${gone.crew.name}: ${describeFailure(e)}",
      fallback: fallback,
    );
    c.invalidate(agentOverviewProvider);
  } finally {
    api.dispose();
    patches.update(
      (l) => [
        for (final p in l)
          if (!identical(p, patch)) p,
      ],
    );
  }
}

/// Sets [sessionId]'s label to [role], or clears it; only the label (138f).
Future<void> sendLabelChange(
  BuildContext context,
  String sessionId,
  RoleNote? role,
) => sendCrewChange(
  context,
  what: 'change the label',
  patch: rolePatch(
    sessionId,
    role == null
        ? null
        : RoleRef(
            vaultId: role.vaultId,
            noteId: role.noteId,
            title: role.title,
          ),
  ),
  send: (api) => role == null
      ? api.clearRole(sessionId)
      : api.setRole(sessionId, role.vaultId, role.noteId),
);

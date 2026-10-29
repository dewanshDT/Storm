import 'package:flutter/material.dart';
import 'package:flutter_lucide/flutter_lucide.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../api/models.dart';
import '../router.dart';
import '../state/app_state.dart';
import '../ui/breakpoints.dart';
import '../ui/controls.dart';
import '../ui/shell/storm_scaffold.dart' show StormChrome;
import '../ui/states.dart';
import '../ui/tokens.dart';
import 'agent_api.dart' show AgentRefusal;
import 'agent_models.dart';
import 'agent_state.dart';
import 'agent_widgets.dart';
import 'agents_screen.dart' show launchAgentSession;
import 'crew_sheets.dart';
import 'crew_widgets.dart';
import 'spawn_api.dart';
import 'spawn_models.dart' show DescendantRef, Descendants;
import 'spawn_state.dart' show childrenOf, sessionDescendantsProvider;
import 'spawn_widgets.dart' show lineageIndented, lineageLine;

/// One crew at `/agents/c/:id` (decision 138, handoff §3 Crew page, 4d): its
/// header and hosts, End all running in place, its members, the notes they
/// started from and what they wrote. A crew stores none of that: it is read
/// from its members.
class CrewScreen extends ConsumerStatefulWidget {
  const CrewScreen({super.key, required this.crewId});

  final String crewId;

  @override
  ConsumerState<CrewScreen> createState() => _CrewScreenState();
}

class _CrewScreenState extends ConsumerState<CrewScreen> {
  final _menu = GlobalKey();

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final o = ref.watch(agentOverviewProvider).value;
    final wide = context.isExpanded;
    final crew = o?.crews?.where((c) => c.id == widget.crewId).firstOrNull;

    final Widget body;
    if (o == null) {
      body = const SkeletonRows(rows: 4);
    } else if (o.unreachable) {
      body = EmptyState(
        icon: LucideIcons.cloud_off,
        title: kAgentsOfflineTitle,
        detail: kAgentsOfflineDetail,
        action: 'Try again',
        onAction: () => reloadAgents(ref),
      );
    } else if (crew == null) {
      body = EmptyState(
        key: const Key('crew-gone'),
        icon: LucideIcons.users,
        title: 'This crew is gone',
        detail: 'Its last session was dismissed, or it was deleted.',
        action: 'Agents',
        onAction: () => context.go(Routes.agents),
      );
    } else {
      body = _CrewPage(crew: crew, overview: o, wide: wide, menuKey: _menu);
    }

    if (wide) {
      return Scaffold(
        backgroundColor: t.bg,
        body: SafeArea(
          left: false,
          child: SingleChildScrollView(
            child: Align(
              alignment: Alignment.topLeft,
              child: ConstrainedBox(
                constraints: BoxConstraints(maxWidth: t.sp * 95),
                child: Padding(
                  padding: EdgeInsets.fromLTRB(
                    t.sp * 5,
                    t.sp * 3.5,
                    t.sp * 5,
                    t.sp * 7.5,
                  ),
                  child: body,
                ),
              ),
            ),
          ),
        ),
      );
    }
    // The phone's crew screen is pushed over the list, Back in the corner;
    // End all running is a sheet here, raised once its request lands.
    if (crew != null && ref.watch(crewEndAllProvider) == crew.id) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted || ref.read(crewEndAllProvider) != crew.id) return;
        ref.read(crewEndAllProvider.notifier).state = null;
        final now = ref.read(agentOverviewProvider).value ?? o!;
        showEndAllSheet(
          context,
          ref,
          crew: crew,
          members: liveFirst(now.membersOf(crew)),
          overview: now,
        );
      });
    }
    return Scaffold(
      body: StormChrome(
        showNav: false,
        onBack: () => context.go(Routes.agents),
        child: Stack(
          children: [
            Positioned.fill(
              child: ListView(
                padding: EdgeInsets.fromLTRB(
                  StormChrome.contentInset(context),
                  0,
                  StormChrome.contentInset(context),
                  StormChrome.navClearance(context),
                ),
                children: [body],
              ),
            ),
            if (crew != null && o != null && o.online.isNotEmpty)
              Positioned(
                left: 0,
                right: 0,
                bottom: 0,
                child: NewSessionPill(
                  key: const Key('crew-new-session-pill'),
                  label: 'New session in crew',
                  onTap: () => launchAgentSession(context, ref, crew: crew),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _CrewPage extends ConsumerWidget {
  const _CrewPage({
    required this.crew,
    required this.overview,
    required this.wide,
    required this.menuKey,
  });

  final Crew crew;
  final AgentOverview overview;
  final bool wide;
  final GlobalKey menuKey;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final o = overview;
    final members = liveFirst(o.membersOf(crew));
    final endAll = ref.watch(crewEndAllProvider) == crew.id;
    final hosts = crewHosts(o, members);
    final summary = [
      if (crewSummary(members).isNotEmpty) crewSummary(members),
      if (hosts.isNotEmpty) hosts.join(', '),
    ].join(' · ');

    void menu() => wide
        ? showCrewMenu(context, ref, anchor: menuKey, crew: crew, overview: o)
        : showCrewSheet(context, ref, crew: crew, overview: o, onPage: true);

    final started = _StartedFrom(members: members);
    final wrote = _Wrote(members: members, overview: o);

    return Column(
      key: Key('crew-page-${crew.id}'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        GestureDetector(
          // Right-click opens the same menu as ··· (138n, Q4).
          onSecondaryTapUp: wide ? (_) => menu() : null,
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Padding(
                padding: EdgeInsets.only(top: t.sp * (wide ? 1.125 : 0.75)),
                child: CrewSquare(
                  accent: crew.accent,
                  size: t.sp * (wide ? 2.25 : 2),
                ),
              ),
              SizedBox(width: t.sp * (wide ? 1.5 : 1.25)),
              Expanded(
                // Wraps, never ellipsizes (handoff §5, a 40-character name).
                child: Text(
                  crew.name,
                  key: const Key('crew-page-name'),
                  style: TextStyle(
                    fontFamily: StormTokens.sansFamily,
                    fontSize: t.titleSize,
                    fontWeight: FontWeight.w600,
                    height: wide ? 1.3 : 1.2,
                    color: t.text,
                  ),
                ),
              ),
              SizedBox(width: t.sp * 1.5),
              // On a phone it is the pill at the foot (4l).
              if (wide) ...[
                StormButton.soft(
                  key: const Key('crew-new-session'),
                  label: 'New session in crew',
                  icon: LucideIcons.plus,
                  onPressed: o.online.isEmpty
                      ? null
                      : () => launchAgentSession(context, ref, crew: crew),
                ),
                SizedBox(width: t.sp),
              ],
              KeyedSubtree(
                key: menuKey,
                child: CrewDots(
                  key: const Key('crew-page-menu'),
                  page: true,
                  onTap: menu,
                ),
              ),
            ],
          ),
        ),
        SizedBox(height: t.sp * 0.75),
        Text(
          summary,
          key: const Key('crew-page-summary'),
          style: TextStyle(
            fontFamily: StormTokens.monoFamily,
            fontSize: t.labelSize * 1.09,
            color: t.text3,
          ),
        ),
        if (endAll && wide) ...[
          SizedBox(height: t.sp * 2.5),
          EndAllBanner(crew: crew, members: members, overview: o),
        ],
        SizedBox(height: t.sp * (wide ? 2.5 : 1)),
        if (wide)
          Container(
            key: const Key('crew-page-members'),
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 1.25,
              vertical: t.sp * 1.5,
            ),
            decoration: BoxDecoration(
              color: t.surface,
              borderRadius: BorderRadius.circular(t.rCard),
              border: Border.all(color: t.border, width: t.bw),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                for (final s in members)
                  lineageIndented(
                    s,
                    members,
                    CrewCardRow(
                      session: s,
                      host: o.hostName(s.hostId),
                      showContext: false,
                      waiting: o.waitingChildOf(s),
                      onTap: () => openAgentSession(context, s.id),
                    ),
                  ),
              ],
            ),
          )
        else
          // The phone's rows, as in its list (4l); a long-press is the
          // session's Crew and Role sheet.
          Column(
            key: const Key('crew-page-members'),
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final s in members)
                lineageIndented(
                  s,
                  members,
                  SessionRow(
                    session: s,
                    host: o.hostName(s.hostId),
                    phone: true,
                    lineage: lineageLine(s, o),
                    waiting: o.waitingChildOf(s),
                    onTap: () => openAgentSession(context, s.id),
                    onLongPress: () => showSessionCrewSheet(context, ref, s),
                  ),
                ),
            ],
          ),
        SizedBox(height: t.sp * 3),
        if (wide)
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(flex: 4, child: started),
              SizedBox(width: t.sp * 3),
              Expanded(flex: 5, child: wrote),
            ],
          )
        else ...[
          started,
          SizedBox(height: t.sp * 3),
          wrote,
        ],
      ],
    );
  }
}

/// The distinct notes the members started from, each with how many did.
class _StartedFrom extends StatelessWidget {
  const _StartedFrom({required this.members});

  final List<AgentSession> members;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final notes = <String, (SessionContext, int)>{};
    for (final s in members) {
      final c = s.context;
      if (c == null) continue;
      final key = '${c.vaultId}/${c.noteId}';
      final seen = notes[key];
      notes[key] = (seen?.$1 ?? c, (seen?.$2 ?? 0) + 1);
    }
    return Column(
      key: const Key('crew-started-from'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const AgentsLabel('Started from'),
        SizedBox(height: t.sp * 1.25),
        if (notes.isEmpty)
          _Quiet('No session in this crew started from a note.')
        else
          for (final (c, n) in notes.values) ...[
            _StartedRow(note: c, sessions: n),
            SizedBox(height: t.sp),
          ],
      ],
    );
  }
}

class _StartedRow extends StatelessWidget {
  const _StartedRow({required this.note, required this.sessions});

  final SessionContext note;
  final int sessions;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final radius = BorderRadius.circular(t.rControl);
    void open() => context.go(Routes.note(note.vaultId, note.noteId));
    return Semantics(
      button: true,
      label: 'Open ${note.title}, $sessions ${_sessions(sessions)}',
      excludeSemantics: true,
      onTap: open,
      child: Material(
        color: Colors.transparent,
        shape: RoundedRectangleBorder(
          borderRadius: radius,
          side: BorderSide(color: t.border, width: t.bw),
        ),
        child: InkWell(
          key: Key('crew-started-${note.noteId}'),
          borderRadius: radius,
          hoverColor: t.surface,
          onTap: open,
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: t.sp * 1.5,
              vertical: t.sp * 1.25,
            ),
            child: Row(
              children: [
                Icon(LucideIcons.file, size: t.codeSize, color: t.text2),
                SizedBox(width: t.sp * 0.75),
                Expanded(
                  child: Text(
                    note.title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontFamily: StormTokens.sansFamily,
                      fontSize: t.uiSize,
                      color: t.text,
                    ),
                  ),
                ),
                SizedBox(width: t.sp),
                Text(
                  '$sessions ${_sessions(sessions)}',
                  style: TextStyle(
                    fontFamily: StormTokens.monoFamily,
                    fontSize: t.labelSize,
                    color: t.text3,
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

String _sessions(int n) => n == 1 ? 'session' : 'sessions';

/// Every member's Wrote, merged: each note once, at its newest write.
class _Wrote extends ConsumerWidget {
  const _Wrote({required this.members, required this.overview});

  final List<AgentSession> members;
  final AgentOverview overview;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = context.tokens;
    final key = [
      for (final s in members)
        if (s.wroteCount > 0) '${s.id}:${s.wroteCount}',
    ].join(',');
    final writes = key.isEmpty
        ? const AsyncValue<List<CrewWrite>?>.data([])
        : ref.watch(crewWritesProvider(key));
    final vaults = ref.watch(vaultsProvider).value ?? const <VaultInfo>[];
    final list = writes.value;
    return Column(
      key: const Key('crew-wrote'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const AgentsLabel('Wrote'),
        SizedBox(height: t.sp * 0.5),
        if (list == null)
          writes.isLoading
              ? const SkeletonRows(rows: 2)
              : _Quiet("Couldn't load what this crew wrote.")
        else if (list.isEmpty)
          Padding(
            padding: EdgeInsets.only(top: t.sp * 0.75),
            child: _Quiet('Nothing written yet.'),
          )
        else
          for (final w in list)
            _WroteRow(
              write: w,
              vault:
                  vaults
                      .where((v) => v.id == w.write.vaultId)
                      .firstOrNull
                      ?.name ??
                  '',
              by: overview.byId(w.sessionId)?.displayName ?? '',
            ),
      ],
    );
  }
}

/// One note a crew member wrote, and which member.
typedef CrewWrite = ({SessionWrite write, String sessionId});

/// The members' writes merged, newest first, each note once. Keyed by
/// `id:wrote_count` pairs, so it is read again only when a count moves.
final crewWritesProvider = FutureProvider.autoDispose
    .family<List<CrewWrite>?, String>((ref, key) async {
      final api = ref.watch(agentApiFactoryProvider)?.call();
      if (api == null) return null;
      try {
        final ids = [for (final p in key.split(',')) p.split(':').first];
        final lists = await Future.wait(ids.map(api.writes));
        final all = <CrewWrite>[
          for (final (i, l) in lists.indexed)
            for (final w in l) (write: w, sessionId: ids[i]),
        ]..sort((a, b) => b.write.at.compareTo(a.write.at));
        final seen = <String>{};
        return [
          for (final w in all)
            if (seen.add(
              w.write.noteId == null
                  ? 'path:${w.write.path}'
                  : '${w.write.vaultId}/${w.write.noteId}',
            ))
              w,
        ];
      } catch (_) {
        return null;
      } finally {
        api.dispose();
      }
    });

class _WroteRow extends StatelessWidget {
  const _WroteRow({required this.write, required this.vault, required this.by});

  final CrewWrite write;
  final String vault;
  final String by;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final w = write.write;
    final title = noteTitleOf(w.title, w.path);
    final shown = title.isNotEmpty ? title : (w.path ?? w.kind);
    final at = w.created
        ? 'new'
        : w.version != null
        ? 'v${w.version}'
        : 'edited';
    final meta = '$at · $by';
    final note = w.note;
    void open() => context.go(
      Routes.note(note!.vaultId, note.noteId, session: write.sessionId),
    );
    final mono = TextStyle(
      fontFamily: StormTokens.monoFamily,
      fontSize: t.labelSize,
      color: t.text3,
    );
    return Semantics(
      button: note != null,
      label: '$shown, $meta',
      excludeSemantics: true,
      onTap: note == null ? null : open,
      child: InkWell(
        key: Key('crew-wrote-${w.noteId ?? w.path}'),
        hoverColor: t.surface,
        onTap: note == null ? null : open,
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
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      shown,
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
                      style: mono,
                    ),
                  ],
                ),
              ),
              SizedBox(width: t.sp),
              Text(meta, style: mono),
            ],
          ),
        ),
      ),
    );
  }
}

class _Quiet extends StatelessWidget {
  const _Quiet(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    return Text(
      text,
      style: TextStyle(
        fontFamily: StormTokens.sansFamily,
        fontSize: t.codeSize,
        color: t.text3,
      ),
    );
  }
}

/// End all running, in place under the crew header (138k, 4d): every member
/// it will stop, those on a host Storm can't reach now (skipped, never
/// queued) and those already ended. It ends each with the one-session call.
class EndAllBanner extends ConsumerStatefulWidget {
  const EndAllBanner({
    super.key,
    required this.crew,
    required this.members,
    required this.overview,
  });

  final Crew crew;
  final List<AgentSession> members;
  final AgentOverview overview;

  @override
  ConsumerState<EndAllBanner> createState() => _EndAllBannerState();
}

/// What End all running would do to a crew's members now: the stops, those
/// on a host Storm can't reach, those already ended, and what each stop
/// takes with it.
class EndAllPlan {
  const EndAllPlan({
    required this.stops,
    required this.unreachable,
    required this.ended,
    required this.descendants,
    required this.reaches,
  });

  final List<AgentSession> stops;
  final List<AgentSession> unreachable;
  final List<AgentSession> ended;

  /// What each stop the list shows a child of would take with it (134-7's
  /// C1); null while one is still being read. A stop whose read failed is
  /// left out, and the server's 409 says so.
  final Map<String, Descendants>? descendants;

  /// The running sessions it reaches beyond its own stops, each with the
  /// stop it ends with.
  final List<(String, String)> reaches;

  /// Something to end, and every stop's descendants read.
  bool get ready => stops.isNotEmpty && descendants != null;
}

EndAllPlan watchEndAllPlan(
  WidgetRef ref,
  AgentOverview o,
  List<AgentSession> members,
) {
  bool reachable(AgentSession s) => o.host(s.hostId)?.online ?? false;
  final stops = [
    for (final s in members)
      if (!s.ended && reachable(s)) s,
  ];
  final descendants = <String, Descendants>{};
  var loading = false;
  for (final s in stops) {
    if (childrenOf(o, s.id).isEmpty) continue;
    final d = ref.watch(sessionDescendantsProvider(s.id));
    if (d.isLoading) loading = true;
    if (d.value case final v? when v.running.isNotEmpty) descendants[s.id] = v;
  }
  final read = loading ? null : descendants;
  final own = {for (final s in stops) s.id};
  return EndAllPlan(
    stops: stops,
    unreachable: [
      for (final s in members)
        if (!s.ended && !reachable(s)) s,
    ],
    ended: [
      for (final s in members)
        if (s.ended) s,
    ],
    descendants: read,
    reaches: [
      for (final s in stops)
        for (final r in read?[s.id]?.running ?? const <DescendantRef>[])
          if (!own.contains(r.id)) (r.name, s.displayName),
    ],
  );
}

/// Ends each of [stops] in order: with the descendants the person was shown
/// (C1) where it has any, else with the one-session call. A stop one of
/// those lists already holds ends with it and is not sent again. Every
/// failure gets its own toast line.
Future<void> endAllRunning(
  BuildContext context,
  WidgetRef ref,
  List<AgentSession> stops, {
  Map<String, Descendants> descendants = const {},
}) async {
  final api = agentApi(ref);
  if (api == null) return;
  final failed = <String>[];
  final reached = {
    for (final d in descendants.values)
      for (final r in d.running) r.id,
  };
  try {
    for (final s in stops) {
      if (reached.contains(s.id)) continue;
      final ids = <String>[
        for (final r in descendants[s.id]?.running ?? const <DescendantRef>[])
          r.id,
      ];
      try {
        if (ids.isEmpty) {
          await api.end(s.id);
        } else {
          await api.endConfirmed(s.id, descendants: ids);
        }
      } catch (e) {
        final changed =
            e is AgentRefusal &&
            (e.code == 'confirm_descendants' ||
                e.code == 'descendants_changed');
        failed.add(
          changed
              ? "Couldn't end ${s.displayName}: what it started changed. "
                    'Check the list again.'
              : "Couldn't end ${s.displayName}: ${describeFailure(e)}",
        );
      }
    }
  } finally {
    api.dispose();
  }
  if (!context.mounted) return;
  ref.invalidate(agentOverviewProvider);
  for (final id in descendants.keys) {
    ref.invalidate(sessionDescendantsProvider(id));
  }
  if (failed.isNotEmpty) {
    ScaffoldMessenger.maybeOf(
      context,
    )?.showSnackBar(SnackBar(content: Text(failed.join('\n'))));
  }
}

class _EndAllBannerState extends ConsumerState<EndAllBanner> {
  bool _ending = false;

  void _close() => ref.read(crewEndAllProvider.notifier).state = null;

  Future<void> _end(EndAllPlan plan) async {
    setState(() => _ending = true);
    await endAllRunning(
      context,
      ref,
      plan.stops,
      descendants: plan.descendants ?? const {},
    );
    if (!mounted) return;
    setState(() => _ending = false);
    _close();
  }

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final o = widget.overview;
    final plan = watchEndAllPlan(ref, o, widget.members);
    final n = plan.stops.length;
    final line = TextStyle(
      fontFamily: StormTokens.sansFamily,
      fontSize: t.codeSize,
      height: 1.45,
      color: t.text,
    );

    Widget group(String label, Key key, List<String> lines) => Padding(
      padding: EdgeInsets.only(top: t.sp * 1.5),
      child: Column(
        key: key,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          AgentsLabel(label),
          SizedBox(height: t.sp * 0.5),
          for (final l in lines) ...[
            Text(l, style: line),
            SizedBox(height: t.sp * 0.5),
          ],
        ],
      ),
    );

    return Container(
      key: const Key('end-all-banner'),
      padding: EdgeInsets.fromLTRB(t.sp * 2, t.sp * 2, t.sp * 2, t.sp * 1.5),
      decoration: BoxDecoration(
        color: t.surface2,
        borderRadius: BorderRadius.circular(t.rCard),
        border: Border.all(color: t.border, width: t.bw),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'End all running in ${widget.crew.name}?',
            style: line.copyWith(fontWeight: FontWeight.w600),
          ),
          SizedBox(height: t.sp * 0.75),
          Text(
            'The agents and everything they started are stopped on their '
            'hosts.',
            style: line.copyWith(color: t.text2),
          ),
          if (plan.stops.isNotEmpty)
            group('Stops', const Key('end-all-stops'), [
              for (final s in plan.stops)
                '${s.displayName} · ${o.hostName(s.hostId)}',
            ]),
          if (plan.reaches.isNotEmpty)
            group('What they started', const Key('end-all-reaches'), [
              for (final (child, parent) in plan.reaches)
                '$child · started by $parent',
            ]),
          if (plan.unreachable.isNotEmpty)
            group("Can't reach now", const Key('end-all-unreachable'), [
              for (final s in plan.unreachable)
                '${s.displayName} · ${o.hostName(s.hostId)} is offline. '
                    'Skipped: end it once the host is back.',
            ]),
          if (plan.ended.isNotEmpty)
            group('Already ended', const Key('end-all-ended'), [
              for (final s in plan.ended) s.displayName,
            ]),
          SizedBox(height: t.sp),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              StormButton.text(
                key: const Key('end-all-cancel'),
                label: 'Cancel',
                onPressed: _close,
              ),
              SizedBox(width: t.sp),
              StormButton.danger(
                key: const Key('end-all-confirm'),
                label: n == 1 ? 'End 1 session' : 'End $n sessions',
                onPressed: _ending || !plan.ready ? null : () => _end(plan),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

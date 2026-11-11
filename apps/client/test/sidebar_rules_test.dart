import 'package:flutter_test/flutter_test.dart';

import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/agent_state.dart';
import 'package:storm/agent/sidebar_rules.dart';

/// Decision 153e (handoff §2, Sidebar Rules A): the tiers, the order they
/// give the sidebar, the roll-ups and the sub-line's words, as pure rules.
void main() {
  final now = DateTime.utc(2026, 11, 12, 14, 0);
  String ago(Duration d) => now.subtract(d).toIso8601String();

  AgentSession s(
    String id, {
    String status = 'running',
    String? activity,
    String? parent,
    String? attention,
    Map<String, dynamic>? reason,
    Map<String, dynamic>? deleteRequest,
    List<Map<String, dynamic>>? decisions,
    String? endedAt,
    String? endReason,
    String? hostLastSeen,
    bool? workToTake,
    int? commits,
    String hostId = 'hst_1',
    String workspace = 'storm',
    (String, int)? crew,
  }) => AgentSession.fromJson({
    'id': id,
    'host_id': hostId,
    'workspace': workspace,
    'provider': 'claude-code',
    'status': status,
    'end_reason': endReason,
    'exit_code': null,
    'signal': null,
    'cols': 80,
    'rows': 24,
    'created_at': ago(const Duration(minutes: 40)),
    'ended_at': endedAt,
    'provider_fallback': null,
    'name': id,
    'activity': activity,
    'parent_session': parent,
    if (parent != null) 'depth': 1,
    'needs_attention': attention,
    'attention': reason,
    'delete_request': deleteRequest,
    'pending_decisions': decisions,
    'host_last_seen': hostLastSeen,
    'work_to_take': workToTake,
    'unapplied_commits': commits,
    if (crew != null) 'crew': {'id': crew.$1, 'order': crew.$2},
  });

  TierOf tiers([Map<String, SidebarTier>? held]) => tierLookup(now, held);
  List<String> ids(List<AgentSession> l) => [for (final x in l) x.id];

  group('tiers', () {
    test('each state has its tier', () {
      SidebarTier tier(AgentSession x) => tierOf(x, now);
      expect(
        tier(s('a', attention: 'permission_prompt')),
        SidebarTier.needsYou,
      );
      expect(tier(s('a', attention: 'delete_approval')), SidebarTier.needsYou);
      expect(
        tier(s('a', reason: {'kind': 'question'})),
        SidebarTier.needsYou,
        reason: 'a question from the reasoned attention',
      );
      expect(
        tier(
          s(
            'a',
            decisions: [
              {'kind': 'delete_request', 'id': 'dr_1'},
            ],
          ),
        ),
        SidebarTier.needsYou,
      );
      expect(tier(s('a', attention: 'blocked')), SidebarTier.blocked);
      expect(
        tier(
          s('a', status: 'failed', endedAt: ago(const Duration(minutes: 20))),
        ),
        SidebarTier.failed,
      );
      expect(
        tier(s('a', status: 'failed', endedAt: ago(const Duration(hours: 2)))),
        SidebarTier.rest,
        reason: 'failed for one hour only',
      );
      expect(tier(s('a', status: 'unknown')), SidebarTier.unknown);
      expect(tier(s('a', attention: 'usage_limit')), SidebarTier.outOfUsage);
      expect(tier(s('a', activity: 'working')), SidebarTier.working);
      expect(tier(s('a', activity: 'idle')), SidebarTier.idle);
      expect(tier(s('a', status: 'starting')), SidebarTier.idle);
      expect(tier(s('a', status: 'parked')), SidebarTier.parked);
      expect(tier(s('a', status: 'completed')), SidebarTier.rest);
    });

    test('an ended session never needs you', () {
      expect(
        tierOf(
          s('a', status: 'completed', attention: 'permission_prompt'),
          now,
        ),
        SidebarTier.rest,
      );
    });

    test('only the first four are urgent', () {
      expect(
        [
          for (final t in SidebarTier.values)
            if (t.urgent) t,
        ],
        [
          SidebarTier.needsYou,
          SidebarTier.blocked,
          SidebarTier.failed,
          SidebarTier.unknown,
        ],
      );
    });
  });

  group('sidebarOrder', () {
    test('needs you → blocked → failed → unknown, the rest as they were', () {
      final list = [
        s('working', activity: 'working'),
        s('unknown', status: 'unknown'),
        s('idle'),
        s('failed', status: 'failed', endedAt: ago(const Duration(minutes: 5))),
        s('usage', attention: 'usage_limit'),
        s('blocked', attention: 'blocked'),
        s('parked', status: 'parked'),
        s('needs', attention: 'permission_prompt'),
      ];
      expect(ids(sidebarOrder(list, tiers())), [
        'needs',
        'blocked',
        'failed',
        'unknown',
        // Below the top four nothing moves: the person's order holds.
        'working',
        'idle',
        'usage',
        'parked',
      ]);
    });

    test('a row only moves when it enters or leaves the top four', () {
      final before = [s('a', activity: 'working'), s('b'), s('c')];
      // c goes idle → out of usage: below the top four, so it stays.
      final after = [
        s('a', activity: 'working'),
        s('b'),
        s('c', attention: 'usage_limit'),
      ];
      expect(ids(sidebarOrder(before, tiers())), ['a', 'b', 'c']);
      expect(ids(sidebarOrder(after, tiers())), ['a', 'b', 'c']);
      // c is blocked: it enters the top four and moves up.
      final blocked = [s('a'), s('b'), s('c', attention: 'blocked')];
      expect(ids(sidebarOrder(blocked, tiers())), ['c', 'a', 'b']);
    });

    test('ties keep their order', () {
      final list = [
        s('a', attention: 'blocked'),
        s('b', attention: 'blocked'),
        s('c'),
        s('d', attention: 'blocked'),
      ];
      expect(ids(sidebarOrder(list, tiers())), ['a', 'b', 'd', 'c']);
    });

    test('held tiers freeze the order while the pointer is in', () {
      final calm = [s('a'), s('b'), s('c')];
      final held = tiersOf(calm, now);
      // c starts needing you while the pointer is over the sidebar.
      final now2 = [s('a'), s('b'), s('c', attention: 'permission_prompt')];
      expect(ids(sidebarOrder(now2, tiers(held))), ['a', 'b', 'c']);
      // The pointer leaves: it moves.
      expect(ids(sidebarOrder(now2, tiers())), ['c', 'a', 'b']);
    });

    test('a session new while held takes its own tier', () {
      final held = tiersOf([s('a'), s('b')], now);
      final list = [s('a'), s('b'), s('new', attention: 'blocked')];
      expect(ids(sidebarOrder(list, tiers(held))), ['new', 'a', 'b']);
    });

    test('children stay under their parent and a tree moves as one', () {
      final list = [
        s('x'),
        s('p'),
        s('c1', parent: 'p'),
        s('c2', parent: 'p', attention: 'permission_prompt'),
        s('y'),
      ];
      // c2 needs you: its whole tree goes first, c2 first under p.
      expect(ids(sidebarOrder(list, tiers())), ['p', 'c2', 'c1', 'x', 'y']);
    });

    test('a grandchild lifts its whole tree', () {
      final list = [
        s('x'),
        s('p'),
        s('c', parent: 'p'),
        s('g', parent: 'c', status: 'unknown'),
      ];
      expect(ids(sidebarOrder(list, tiers())), ['p', 'c', 'g', 'x']);
    });

    test('a child whose parent is listed elsewhere is a tree of its own', () {
      final list = [s('x'), s('c', parent: 'elsewhere', attention: 'blocked')];
      expect(ids(sidebarOrder(list, tiers())), ['c', 'x']);
    });

    test('collapsed trees hide their rows, at any depth', () {
      final list = [s('p'), s('c', parent: 'p'), s('g', parent: 'c'), s('x')];
      final order = sidebarOrder(list, tiers());
      expect(ids(withoutCollapsed(order, {'p'})), ['p', 'x']);
      expect(ids(withoutCollapsed(order, {'c'})), ['p', 'c', 'x']);
      expect(ids(descendantsIn(list, 'p')), ['c', 'g']);
    });
  });

  group('crewOrder', () {
    final crews = [
      for (final (i, id) in ['relay', 'gateway', 'docs'].indexed)
        Crew.fromJson({'id': id, 'name': id, 'color': 'sage', 'order': i}),
    ];

    test('crews sort by their most urgent member', () {
      final members = {
        'relay': [s('r1', status: 'failed', endedAt: ago(Duration.zero))],
        'gateway': [s('g1'), s('g2', attention: 'permission_prompt')],
        'docs': [s('d1', activity: 'working')],
      };
      expect(
        crewOrder(crews, (c) => members[c.id]!, tiers()).map((c) => c.id),
        ['gateway', 'relay', 'docs'],
      );
    });

    test('calm crews keep the person\'s order', () {
      final members = {
        'relay': [s('r1')],
        'gateway': [s('g1', attention: 'usage_limit')],
        'docs': [s('d1', status: 'parked')],
      };
      expect(
        crewOrder(crews, (c) => members[c.id]!, tiers()).map((c) => c.id),
        ['relay', 'gateway', 'docs'],
      );
    });
  });

  group('roll-ups', () {
    test('the most urgent state and how many share it', () {
      final r = rollUpOf([
        s('a', status: 'failed', endedAt: ago(Duration.zero)),
        s('b', attention: 'permission_prompt'),
        s('c', attention: 'delete_approval'),
      ], tiers())!;
      expect(r.words, '2 needs you');
      expect(r.outlined, isTrue);
      expect(r.loud, isTrue);
    });

    test('failed is danger without the outline', () {
      final r = rollUpOf([
        s('a', status: 'failed', endedAt: ago(Duration.zero)),
        s('b'),
      ], tiers())!;
      expect(r.words, '1 failed');
      expect(r.outlined, isFalse);
      expect(r.loud, isTrue);
    });

    test('blocked, offline and out of usage roll up quietly', () {
      expect(rollUpOf([s('a', attention: 'blocked')], tiers())!.loud, isFalse);
      expect(
        rollUpOf([s('a', status: 'unknown')], tiers())!.words,
        '1 host offline',
      );
      expect(
        rollUpOf([s('a', attention: 'usage_limit')], tiers())!.words,
        '1 out of usage',
      );
    });

    test('nothing to roll up for calm sessions', () {
      expect(rollUpOf([s('a'), s('b', activity: 'working')], tiers()), isNull);
    });
  });

  group('the sub-line says why, in words', () {
    SubLine? line(AgentSession x, {AgentOverview? o}) =>
        reasonLine(x, now: now, tier: tierOf(x, now), o: o);

    test('a permission prompt, with how long it has waited', () {
      expect(
        line(
          s(
            'a',
            attention: 'permission_prompt',
            reason: {
              'kind': 'permission_prompt',
              'since': ago(const Duration(minutes: 3)),
            },
          ),
        ),
        const SubLine('permission prompt · 3m', SubTone.loud),
      );
    });

    test('a delete request, from its pending decision', () {
      expect(
        line(
          s(
            'a',
            decisions: [
              {
                'kind': 'delete_request',
                'id': 'dr_1',
                'created_at': ago(const Duration(minutes: 1)),
              },
            ],
          ),
        ),
        const SubLine('delete request · 1m', SubTone.loud),
      );
    });

    test('blocked: on whom, else the agent\'s own words, else bare', () {
      final on = s('test-gateway');
      final o = AgentOverview(sessions: [on], hosts: const []);
      expect(
        line(
          s(
            'a',
            attention: 'blocked',
            reason: {'kind': 'blocked', 'blocked_on': 'test-gateway'},
          ),
          o: o,
        )!.text,
        'blocked · waiting on test-gateway',
      );
      expect(
        line(
          s(
            'a',
            attention: 'blocked',
            reason: {'kind': 'blocked', 'detail': 'need a key'},
          ),
        )!.text,
        'blocked · need a key',
      );
      expect(
        line(s('a', attention: 'blocked')),
        const SubLine('blocked', SubTone.quiet),
      );
    });

    test('failed, with why it ended', () {
      expect(
        line(
          s(
            'a',
            status: 'failed',
            endReason: 'host_restart',
            endedAt: ago(const Duration(minutes: 20)),
          ),
        ),
        const SubLine('failed · host restarted · 20m', SubTone.danger),
      );
      // Past the hour it is no longer loud.
      expect(
        line(
          s(
            'a',
            status: 'failed',
            endReason: 'host_restart',
            endedAt: ago(const Duration(hours: 3)),
          ),
        ),
        const SubLine('failed · host restarted · 3h', SubTone.plain),
      );
    });

    test('out of usage, with the reset only when known', () {
      expect(
        line(s('a', attention: 'usage_limit')),
        const SubLine('out of usage', SubTone.quiet),
      );
      final resets = DateTime(2026, 11, 12, 15, 0);
      expect(
        line(
          s(
            'a',
            attention: 'usage_limit',
            reason: {
              'kind': 'usage_limit',
              'resets_at': resets.toUtc().toIso8601String(),
            },
          ),
        )!.text,
        'out of usage · resets 15:00',
      );
    });

    test('host offline, with when it was last seen', () {
      expect(
        line(
          s(
            'a',
            status: 'unknown',
            hostLastSeen: ago(const Duration(minutes: 9)),
          ),
        ),
        const SubLine('host offline · last seen 9m', SubTone.quiet),
      );
      expect(line(s('a', status: 'unknown'))!.text, 'host offline');
    });

    test('commits to apply, or work to apply without a count', () {
      expect(
        line(s('a', status: 'completed', workToTake: true, commits: 2)),
        const SubLine('completed · 2 commits to apply', SubTone.accent),
      );
      expect(
        line(s('a', status: 'completed', workToTake: true, commits: 1))!.text,
        'completed · 1 commit to apply',
      );
      expect(
        line(s('a', status: 'completed', workToTake: true))!.text,
        'completed · work to apply',
      );
      expect(line(s('a', status: 'completed', workToTake: false)), isNull);
    });

    test('otherwise none: the row says workspace · age', () {
      expect(line(s('a')), isNull);
      expect(line(s('a', activity: 'working')), isNull);
    });

    test('an older server: the kind words only', () {
      expect(
        line(s('a', attention: 'permission_prompt'))!.text,
        'permission prompt',
      );
      expect(
        line(s('a', attention: 'delete_approval'))!.text,
        'delete request',
      );
      expect(line(s('a', attention: 'usage_limit'))!.text, 'out of usage');
      expect(line(s('a', status: 'unknown'))!.text, 'host offline');
    });
  });

  group('where a row stands', () {
    test('a child listed apart from its parent names it', () {
      final p = s('gateway-impl', crew: ('crw_1', 0));
      final c = s('bench-check', parent: 'gateway-impl', crew: ('crw_2', 0));
      final o = AgentOverview(sessions: [p, c], hosts: const []);
      expect(parentApart(c, [c], o)?.id, 'gateway-impl');
      expect(parentApart(c, [p, c], o), isNull);
      expect(parentApart(p, [p], o), isNull);
    });

    test('hosts are named only when there is more than one', () {
      expect(
        namesHosts(AgentOverview(sessions: [s('a'), s('b')], hosts: const [])),
        isFalse,
      );
      expect(
        namesHosts(
          AgentOverview(
            sessions: [
              s('a'),
              s('b', hostId: 'hst_2'),
            ],
            hosts: const [],
          ),
        ),
        isTrue,
      );
    });

    test('needs you: a prompt, a question or a delete approval only', () {
      expect(needsYou(s('a', attention: 'permission_prompt')), isTrue);
      expect(needsYou(s('a', attention: 'blocked')), isFalse);
      expect(needsYou(s('a', attention: 'usage_limit')), isFalse);
      expect(
        needsYou(s('a', status: 'failed', endedAt: ago(Duration.zero))),
        isFalse,
      );
    });
  });
}

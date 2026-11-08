import 'package:flutter_test/flutter_test.dart';

import 'package:storm/agent/agent_models.dart';

import 'agent_fakes.dart';
import 'spawn_fakes.dart';

void main() {
  test('withCrew/withRole keep spawn, identity and isolation', () {
    final s = AgentSession.fromJson({
      ...spawnSession(
        'ags_c',
        parent: 'ags_p',
        depth: 1,
        needsAttention: 'approval',
        spawn: {'enabled': true},
        childWorkspace: {'mode': 'clone', 'commit': 'abc1234'},
        identity: 'unavailable',
      ),
      'isolation': {'state': 'unavailable', 'reason': 'off'},
      'crew': {'id': 'crw_1', 'order': 0},
      'role': roleRef('k_rev', 'Storm Reviewer'),
    });
    for (final r in [
      s.withCrew(const SessionCrew(id: 'crw_2', order: 1)),
      s.withCrew(null),
      s.withRole(null),
      s.withRole(const RoleRef(vaultId: 'v-kit', noteId: 'k_coder')),
    ]) {
      expect(r.spawn.parentSession, 'ags_p');
      expect(r.spawn.rootSession, 'ags_p');
      expect(r.spawn.depth, 1);
      expect(r.spawn.needsAttention, 'approval');
      expect(r.spawn.switchState?.enabled, isTrue);
      expect(r.spawn.childWorkspace?.commit, 'abc1234');
      expect(r.identityState, 'unavailable');
      expect(r.isolationState, 'unavailable');
      expect(r.isolationReason, 'off');
    }
    expect(s.withCrew(null).crew, isNull);
    expect(s.withCrew(null).role?.noteId, 'k_rev');
    expect(s.withRole(null).role, isNull);
    expect(s.withRole(null).crew?.id, 'crw_1');
  });

  test('ran_as parses, survives a copy, and is null when not recorded', () {
    final s = AgentSession.fromJson({
      ...spawnSession('ags_r'),
      'ran_as': {
        'account': 'storm-runtime',
        'isolation': {
          'state': 'verified',
          'mechanism': 'systemd_units',
          'reason': null,
        },
        'identity': {
          'state': 'unavailable',
          'mechanism': null,
          'reason': 'old_host',
        },
      },
    });
    for (final r in [s, s.withCrew(null), s.withLive(s)]) {
      expect(r.ranAs?.account, 'storm-runtime');
      expect(r.ranAs?.isolationState, 'verified');
      expect(r.ranAs?.isolationReason, isNull);
      expect(r.ranAs?.identityState, 'unavailable');
    }
    final unnamed = AgentSession.fromJson({
      ...spawnSession('ags_u'),
      'ran_as': {
        'account': null,
        'isolation': {'state': 'unavailable', 'reason': 'off'},
        'identity': {'state': 'verified'},
      },
    });
    expect(unnamed.ranAs?.account, isNull);
    expect(unnamed.ranAs?.isolationReason, 'off');
    expect(AgentSession.fromJson(spawnSession('ags_o')).ranAs, isNull);
    expect(
      AgentSession.fromJson({...spawnSession('ags_n'), 'ran_as': null}).ranAs,
      isNull,
    );
  });

  test('a host says its account and isolation; an old host says neither', () {
    AgentHost host(Object? execution) => AgentHost.fromJson({
      'id': 'hst_1',
      'name': 'build-vm',
      'status': 'online',
      'execution': ?execution,
    });
    final mine = host({'account': 'dewansh', 'isolation': 'off'});
    expect(mine.execution?.account, 'dewansh');
    expect(mine.execution?.perSession, isFalse);
    expect(mine.executionLine, 'Runs as dewansh');
    final box = host({'account': 'storm-runtime', 'isolation': 'per-session'});
    expect(box.execution?.perSession, isTrue);
    expect(box.executionLine, 'Runs as storm-runtime · each session isolated');
    for (final old in [
      host(null),
      host('garbled'),
      host({'account': null, 'isolation': 'off'}),
      host({'account': '', 'isolation': 'per-session'}),
    ]) {
      expect(old.executionLine, kRunsAsUnknown);
    }
    expect(host(null).execution, isNull);
  });
}

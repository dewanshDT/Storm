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
}

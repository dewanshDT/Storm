import 'agent_api.dart';
import 'spawn_models.dart';

/// The spawning routes (decisions 134, 137 and 145), owner only on the
/// session tier: a person's device reads what agents did, turns the switch,
/// ends and dismisses, and reads and applies a child's work.
extension SpawnApi on AgentApi {
  String _s(String id) => '/v1/agent/sessions/${Uri.encodeComponent(id)}';

  /// What a session's agent started, messaged and ended, newest first
  /// (decision 145a: acts, never requests a person decides).
  Future<List<AgentAct>> acts(String sessionId) async {
    final json = await send('GET', '${_s(sessionId)}/requests') as Map;
    return [
      for (final r in json['requests'] as List? ?? const [])
        if (r is Map<String, dynamic>) AgentAct.fromJson(r),
    ];
  }

  /// Turns a session's spawn switch (141e: off in About, on only for a
  /// session launched with it).
  Future<void> setSpawn(String sessionId, bool enabled) async {
    await send('PUT', '${_s(sessionId)}/spawn', body: {'enabled': enabled});
  }

  Future<Descendants> descendants(String sessionId) async =>
      Descendants.fromJson(
        await send('GET', '${_s(sessionId)}/descendants')
            as Map<String, dynamic>,
      );

  /// A person's end of a session with the descendants they were shown (C1).
  Future<void> endConfirmed(
    String sessionId, {
    required List<String> descendants,
    bool reportFirst = false,
  }) async {
    await send(
      'POST',
      '${_s(sessionId)}/end',
      body: {'descendants': descendants, 'report_first': reportFirst},
    );
  }

  /// End all children (C5): the session itself keeps running.
  Future<void> endChildren(
    String sessionId, {
    required List<String> descendants,
    bool reportFirst = false,
  }) async {
    await send(
      'POST',
      '${_s(sessionId)}/end-children',
      body: {'descendants': descendants, 'report_first': reportFirst},
    );
  }

  Future<ChildWork> childWork(String sessionId) async => ChildWork.fromJson(
    await send('GET', '${_s(sessionId)}/child-work') as Map<String, dynamic>,
  );

  /// Applies an ended child's work into its parent's workspace (137e): a
  /// clone as `storm/<child>`, a scratch child as the directory [name].
  Future<Map<String, dynamic>> applyChildWork(
    String sessionId, {
    String? name,
  }) async =>
      (await send(
                'POST',
                '${_s(sessionId)}/child-work/apply',
                body: {'name': ?name},
              )
              as Map?)
          ?.cast<String, dynamic>() ??
      const {};

  Future<void> deleteChildWork(String sessionId) async {
    await send('DELETE', '${_s(sessionId)}/child-work');
  }
}

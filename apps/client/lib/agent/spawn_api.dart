import 'agent_api.dart';
import 'spawn_models.dart';

/// The spawning routes (decisions 134 and 137), owner only on the session
/// tier: a person's device decides every request (A5), turns the switch,
/// ends and dismisses, and reads and applies a child's work.
extension SpawnApi on AgentApi {
  String _s(String id) => '/v1/agent/sessions/${Uri.encodeComponent(id)}';
  String _r(String id) => '/v1/agent/requests/${Uri.encodeComponent(id)}';

  /// What a session asked for, newest first.
  Future<List<SpawnRequest>> requests(String sessionId) async {
    final json = await send('GET', '${_s(sessionId)}/requests') as Map;
    return [
      for (final r in json['requests'] as List? ?? const [])
        SpawnRequest.fromJson(r as Map<String, dynamic>),
    ];
  }

  Future<SpawnRequest> request(String id) async =>
      SpawnRequest.fromJson(await send('GET', _r(id)) as Map<String, dynamic>);

  /// Approves on this device. [writeVault] is D2's toggle, off unless the
  /// person ticked it. The answer is the outcome, a refusal included.
  Future<SpawnRequest> approve(String id, {bool writeVault = false}) async =>
      SpawnRequest.fromJson(
        await send(
              'POST',
              '${_r(id)}/approve',
              body: {'write_vault': writeVault},
            )
            as Map<String, dynamic>,
      );

  Future<SpawnRequest> deny(String id) async => SpawnRequest.fromJson(
    await send('POST', '${_r(id)}/deny', body: const {})
        as Map<String, dynamic>,
  );

  /// Turns a session's spawn switch; the answer counts the waiting requests
  /// turning it off refused.
  Future<int> setSpawn(String sessionId, bool enabled) async {
    final json =
        await send('PUT', '${_s(sessionId)}/spawn', body: {'enabled': enabled})
            as Map?;
    return (json?['refused_requests'] as num?)?.toInt() ?? 0;
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

  /// Dismisses a whole tree, once every member has ended (C8). The crews
  /// it emptied, when the server says.
  Future<List<Map<String, dynamic>>> dismissTree(String sessionId) async {
    final json = await send(
      'DELETE',
      _s(sessionId),
      query: const {'tree': 'true'},
    );
    return [
      if (json is Map)
        for (final c in json['removed_crews'] as List? ?? const [])
          if (c is Map<String, dynamic>) c,
    ];
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

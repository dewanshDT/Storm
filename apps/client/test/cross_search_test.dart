import 'package:flutter_test/flutter_test.dart';
import 'package:storm/api/models.dart';
import 'package:storm/api/storm_api.dart';

import 'fake_server.dart';

void main() {
  test('CrossSearchResult parses hits and skipped vaults', () {
    final result = CrossSearchResult.fromJson({
      'hits': [
        {
          'vault_id': 'v1',
          'vault_name': 'Work',
          'id': 'n1',
          'path': 'Plans/Q4.md',
          'title': 'Q4',
          'snippet': 'the <<plan>> for',
          'tier': 1,
        },
      ],
      'skipped': [
        {'vault_id': 'v2', 'vault_name': 'Home', 'reason': 'busy'},
      ],
    });

    final hit = result.hits.single;
    expect(hit.vaultId, 'v1');
    expect(hit.vaultName, 'Work');
    expect(hit.id, 'n1');
    expect(hit.path, 'Plans/Q4.md');
    expect(hit.title, 'Q4');
    expect(hit.snippet, 'the <<plan>> for');
    expect(hit.tier, 1);
    final skipped = result.skipped.single;
    expect(skipped.vaultId, 'v2');
    expect(skipped.vaultName, 'Home');
    expect(skipped.reason, 'busy');
  });

  test('a response without skipped parses as none skipped', () {
    final result = CrossSearchResult.fromJson({'hits': []});
    expect(result.hits, isEmpty);
    expect(result.skipped, isEmpty);
  });

  group('StormApi.searchAll', () {
    late FakeServer server;
    late StormApi api;

    setUp(() {
      server = FakeServer();
      api = StormApi(baseUrl: 'http://test', token: 't', client: server.client);
      server.byVault[FakeServer.primaryVault]!['a'] = ServerNote(
        id: 'a',
        path: 'A.md',
        content: 'alpha budget',
        version: 1,
      );
      server.addVault('v-home', 'Home')['b'] = ServerNote(
        id: 'b',
        path: 'B.md',
        content: 'budget review',
        version: 1,
      );
    });

    test('returns hits from every vault', () async {
      final result = await api.searchAll('budget');
      expect(result.hits.map((h) => h.vaultName), ['Primary', 'Home']);
      expect(result.skipped, isEmpty);
    });

    test('reports a skipped vault', () async {
      server.searchSkipped.add('v-home');
      final result = await api.searchAll('budget');
      expect(result.hits.map((h) => h.vaultId), [FakeServer.primaryVault]);
      expect(result.skipped.single.vaultName, 'Home');
    });

    test('an empty query sends nothing', () async {
      final before = server.requests;
      expect((await api.searchAll('   ')).hits, isEmpty);
      expect(server.requests, before);
    });

    test('the per-vault route still answers', () async {
      final hits = await api.search('v-home', 'review');
      expect(hits.single.id, 'b');
    });
  });
}

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/models.dart';
import 'package:storm/api/storm_api.dart';

/// Tag and backlink parsing, and the grouping the tag browser relies on.
void main() {
  StormApi apiReturning(String path, Object body) => StormApi(
        baseUrl: 'http://test',
        token: 't',
        client: MockClient((request) async {
          if (request.url.path == path) {
            return http.Response(jsonEncode(body), 200,
                headers: {'content-type': 'application/json'});
          }
          return http.Response('{"error":"unexpected ${request.url.path}"}', 404,
              headers: {'content-type': 'application/json'});
        }),
      );

  group('tags', () {
    test('parses the server response', () async {
      final api = apiReturning('/v1/tags', {
        'tags': [
          {'tag': 'homelab', 'count': 12},
          {'tag': 'proj/storm', 'count': 3},
        ]
      });

      final tags = await api.tags();
      expect(tags, hasLength(2));
      expect(tags.first.tag, 'homelab');
      expect(tags.first.count, 12);
    });

    test('topLevel groups hierarchical tags', () {
      // The browser collapses `proj/*` under `proj`; a flat list of forty
      // sibling tags is unusable.
      expect(const TagCount(tag: 'proj/storm', count: 1).topLevel, 'proj');
      expect(const TagCount(tag: 'proj/storm/sync', count: 1).topLevel, 'proj');
      expect(const TagCount(tag: 'homelab', count: 1).topLevel, 'homelab');
    });

    test('grouping keeps an exact parent tag distinct from its children', () {
      // `#proj` and `#proj/storm` are different tags and must not be merged.
      const all = [
        TagCount(tag: 'proj', count: 2),
        TagCount(tag: 'proj/storm', count: 5),
        TagCount(tag: 'proj/nas', count: 1),
      ];
      final groups = <String, List<TagCount>>{};
      for (final t in all) {
        groups.putIfAbsent(t.topLevel, () => []).add(t);
      }

      expect(groups.keys, ['proj']);
      final members = groups['proj']!;
      expect(members.where((m) => m.tag == 'proj').single.count, 2);
      expect(members.where((m) => m.tag != 'proj'), hasLength(2));
      expect(members.fold<int>(0, (s, m) => s + m.count), 8);
    });

    test('tag names are URL-encoded so slashes survive the path', () async {
      String? seen;
      final api = StormApi(
        baseUrl: 'http://test',
        token: 't',
        client: MockClient((request) async {
          seen = request.url.path;
          return http.Response('{"notes":[]}', 200,
              headers: {'content-type': 'application/json'});
        }),
      );

      await api.notesWithTag('proj/storm');
      expect(seen, '/v1/tags/proj%2Fstorm',
          reason: 'an unencoded slash would look like a nested route');
    });

    test('notes for a tag are parsed', () async {
      final api = apiReturning('/v1/tags/homelab', {
        'notes': [
          {
            'id': 'n1',
            'path': 'A.md',
            'title': 'A',
            'version': 1,
            'modified': '',
            'size': 5,
          }
        ]
      });

      final notes = await api.notesWithTag('homelab');
      expect(notes.single.id, 'n1');
    });
  });

  group('backlinks', () {
    test('parses the server response', () async {
      final api = apiReturning('/v1/notes/n1/backlinks', {
        'title': 'Target',
        'backlinks': [
          {
            'id': 'n2',
            'path': 'Folder/Source.md',
            'title': 'Source',
            'version': 3,
            'modified': '',
            'size': 10,
          }
        ]
      });

      final back = await api.backlinks('n1');
      expect(back, hasLength(1));
      expect(back.single.title, 'Source');
      expect(back.single.folder, 'Folder',
          reason: 'the panel shows the folder alongside the title');
    });

    test('a note with no backlinks yields an empty list, not an error',
        () async {
      final api = apiReturning('/v1/notes/n1/backlinks', {
        'title': 'Lonely',
        'backlinks': <dynamic>[],
      });
      expect(await api.backlinks('n1'), isEmpty);
    });
  });

  group('search snippets', () {
    test('parses hits with the server\'s highlight markers', () async {
      final api = apiReturning('/v1/search', {
        'hits': [
          {
            'id': 'n1',
            'path': 'A.md',
            'title': 'A',
            'snippet': 'the <<quick>> brown fox',
          }
        ]
      });

      final hits = await api.search('quick');
      expect(hits.single.snippet, contains('<<quick>>'));
    });

    test('an empty query short-circuits without hitting the server', () async {
      var called = false;
      final api = StormApi(
        baseUrl: 'http://test',
        token: 't',
        client: MockClient((_) async {
          called = true;
          return http.Response('{"hits":[]}', 200,
              headers: {'content-type': 'application/json'});
        }),
      );

      expect(await api.search('   '), isEmpty);
      expect(called, isFalse);
    });
  });
}

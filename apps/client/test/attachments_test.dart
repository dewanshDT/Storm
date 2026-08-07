import 'dart:convert';

import 'package:drift/native.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/api/storm_api.dart';
import 'package:storm/cache/cache_db.dart';
import 'package:storm/sync/sync_engine.dart';
import 'package:storm/ui/attachment_strip.dart';
import 'fake_server.dart';

/// Attachment upload and the preview strip.
void main() {
  group('uploading', () {
    late CacheDb cache;
    late SyncEngine engine;
    late Map<String, List<int>> uploaded;
    late bool unreachable;
    late int? failWith;

    setUp(() {
      cache = CacheDb(NativeDatabase.memory());
      uploaded = {};
      unreachable = false;
      failWith = null;

      final client = MockClient((request) async {
        if (unreachable) throw http.ClientException('Connection refused');
        if (failWith != null) {
          return http.Response(
            '{"error":"refused"}',
            failWith!,
            headers: {'content-type': 'application/json'},
          );
        }
        // Attachment routes are vault-scoped now; strip the prefix so this
        // fake still keys uploads by their vault-relative path.
        final path = Uri.decodeFull(
          request.url.path.replaceFirst(
            '/v1/vaults/${FakeServer.primaryVault}/attachments/',
            '',
          ),
        );
        uploaded[path] = request.bodyBytes;
        return http.Response(
          jsonEncode({'path': path, 'size': request.bodyBytes.length}),
          200,
          headers: {'content-type': 'application/json'},
        );
      });

      engine = SyncEngine(
        api: StormApi(baseUrl: 'http://test', token: 't', client: client),
        cache: cache,
        vaultId: FakeServer.primaryVault,
      );
    });

    tearDown(() async {
      engine.dispose();
      await cache.close();
    });

    test('bytes arrive unmodified', () async {
      // Deliberately not valid UTF-8 — treating an upload as text would
      // corrupt every image.
      final bytes = [0x89, 0x50, 0x4E, 0x47, 0x00, 0xFF, 0xFE];
      final result = await engine.attach(fileName: 'logo.png', bytes: bytes);

      expect(result.error, isNull);
      expect(uploaded[result.path], bytes);
    });

    test('lands under attachments/ with a safe, unique name', () async {
      final result = await engine.attach(
        fileName: 'My Photo (1).png',
        bytes: [1],
      );

      expect(result.path, startsWith('attachments/'));
      expect(result.path, endsWith('.png'));
      expect(result.path, isNot(contains(' ')));
      expect(result.path, isNot(contains('(')));
    });

    test('two files with the same name do not clobber each other', () async {
      final a = await engine.attach(fileName: 'shot.png', bytes: [1]);
      await Future.delayed(const Duration(milliseconds: 2));
      final b = await engine.attach(fileName: 'shot.png', bytes: [2]);

      expect(a.path, isNot(b.path));
      expect(uploaded[a.path], [1]);
      expect(uploaded[b.path], [2]);
    });

    test('a path separator in the name cannot escape the folder', () async {
      final result = await engine.attach(
        fileName: '../../etc/passwd',
        bytes: [1],
      );
      expect(result.path, startsWith('attachments/'));
      expect(result.path, isNot(contains('..')));
    });

    test('offline reports plainly rather than queueing', () async {
      // The outbox is for small text diffs, not megabytes of binary.
      unreachable = true;
      final result = await engine.attach(fileName: 'a.png', bytes: [1]);

      expect(result.path, isNull);
      expect(result.error, contains('Cannot reach the server'));
      expect(await cache.pendingCount(FakeServer.primaryVault), 0);
    });

    test('a server refusal surfaces', () async {
      failWith = 413;
      final result = await engine.attach(fileName: 'huge.png', bytes: [1]);
      expect(result.path, isNull);
      expect(result.error, isNotNull);
    });
  });

  group('the preview strip finds images', () {
    test('picks up embedded images', () {
      const body = '''
# Note

![a shot](attachments/shot-1.png)
Some text.
![](attachments/diagram.JPG)
''';
      expect(AttachmentStrip.imagePaths(body), [
        'attachments/shot-1.png',
        'attachments/diagram.JPG',
      ]);
    });

    test('ignores non-images and remote URLs', () {
      const body = '''
![doc](attachments/report.pdf)
![remote](https://example.com/x.png)
[not an embed](attachments/other.png)
''';
      // A PDF has no thumbnail, a remote image isn't ours, and a plain link
      // is not an embed.
      expect(AttachmentStrip.imagePaths(body), isEmpty);
    });

    test('dedupes repeats', () {
      const body = '![a](attachments/x.png)\n![b](attachments/x.png)\n';
      expect(AttachmentStrip.imagePaths(body), ['attachments/x.png']);
    });

    test('a note with no images yields none', () {
      expect(AttachmentStrip.imagePaths('# Just text\n'), isEmpty);
    });
  });

  group('attachment URLs', () {
    test('carry the token, since image widgets cannot set headers', () {
      final api = StormApi(baseUrl: 'http://host:8484', token: 'secret');
      final url = api.attachmentUrl(
        FakeServer.primaryVault,
        'attachments/x.png',
      );

      expect(
        url.path,
        '/v1/vaults/${FakeServer.primaryVault}/attachments/attachments/x.png',
      );
      expect(url.queryParameters['token'], 'secret');
      api.dispose();
    });
  });
}

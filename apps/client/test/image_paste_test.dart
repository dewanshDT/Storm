import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:storm/agent/agent_api.dart';
import 'package:storm/agent/image_paste.dart';
import 'package:storm/agent/session_controller.dart';
import 'package:storm/agent/terminal_events.dart';
import 'package:storm/agent/terminal_surface.dart';

/// D15 AM45: a pasted image is staged on the host and its path pasted.
void main() {
  final png = Uint8List.fromList([
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, //
  ]);

  group('the image\'s own bytes say what it is', () {
    test('the four staged types, and not text', () {
      expect(sniffImageMime(png), 'image/png');
      expect(
        sniffImageMime(Uint8List.fromList([0xFF, 0xD8, 0xFF, 0])),
        'image/jpeg',
      );
      expect(
        sniffImageMime(Uint8List.fromList(utf8.encode('GIF89a..'))),
        'image/gif',
      );
      expect(
        sniffImageMime(
          Uint8List.fromList(utf8.encode('RIFF\x00\x00\x00\x00WEBPVP8 ')),
        ),
        'image/webp',
      );
      expect(sniffImageMime(Uint8List.fromList(utf8.encode('hello'))), isNull);
      expect(sniffImageMime(Uint8List(0)), isNull);
    });

    testWidgets('a staged type passes as it is; text is no image', (
      tester,
    ) async {
      await tester.runAsync(() async {
        final kept = await normalizeImage(png);
        expect(kept!.mime, 'image/png');
        expect(identical(kept.bytes, png), isTrue);
        expect(
          await normalizeImage(Uint8List.fromList(utf8.encode('no'))),
          isNull,
        );
      });
    });
  });

  group('which keys are a paste', () {
    PasteChord? chord(
      TargetPlatform platform, {
      bool shift = false,
      bool control = false,
      bool alt = false,
      bool meta = false,
      LogicalKeyboardKey key = LogicalKeyboardKey.keyV,
    }) => StormTerminal.pasteChord(
      key,
      shift: shift,
      control: control,
      alt: alt,
      meta: meta,
      platform: platform,
    );

    test('Cmd+V on a Mac, Ctrl+Shift+V elsewhere, Ctrl+V everywhere', () {
      expect(chord(TargetPlatform.macOS, meta: true), PasteChord.paste);
      expect(
        chord(TargetPlatform.linux, control: true, shift: true),
        PasteChord.paste,
      );
      expect(
        chord(TargetPlatform.windows, control: true, shift: true),
        PasteChord.paste,
      );
      expect(chord(TargetPlatform.macOS, control: true), PasteChord.controlV);
      expect(chord(TargetPlatform.linux, control: true), PasteChord.controlV);
      expect(
        chord(TargetPlatform.linux, meta: true),
        isNull,
        reason: 'Super is not paste',
      );
      expect(chord(TargetPlatform.linux, control: true, alt: true), isNull);
      expect(chord(TargetPlatform.linux), isNull);
      expect(
        chord(TargetPlatform.macOS, meta: true, key: LogicalKeyboardKey.keyC),
        isNull,
      );
    });
  });

  group('the controller', () {
    late List<http.Request> requests;
    late Future<http.Response> Function(http.Request) stage;

    SessionController controller() {
      requests = [];
      final c = SessionController(
        api: AgentApi(
          baseUrl: 'http://s',
          token: 't',
          client: MockClient((req) async {
            requests.add(req);
            if (req.url.path.endsWith('/terminal/images')) return stage(req);
            return http.Response('', 204);
          }),
        ),
        sessionId: 'ags_1',
        open: (_) => const Stream<TerminalEvent>.empty(),
      )..start();
      return c;
    }

    List<String> typed() => [
      for (final r in requests)
        if (r.url.path.endsWith('/terminal/input')) utf8.decode(r.bodyBytes),
    ];

    test(
      'stages, then types the path; the chip waits 300 ms to appear',
      () async {
        final answer = Completer<http.Response>();
        stage = (_) => answer.future;
        final c = controller();
        final done = c.pasteImages([PastedImage(png, 'image/png')]);
        await Future<void>.delayed(const Duration(milliseconds: 100));
        expect(c.imagePaste.value, isNull, reason: 'quick pastes show nothing');
        await Future<void>.delayed(const Duration(milliseconds: 300));
        expect(c.imagePaste.value?.count, 1);
        answer.complete(
          http.Response(
            jsonEncode({'image': 'img_1', 'path': '/h/inbox/img_1.png'}),
            200,
          ),
        );
        await done;
        await Future<void>.delayed(const Duration(milliseconds: 50));
        final req = requests.firstWhere((r) => r.url.path.endsWith('/images'));
        expect(req.headers['content-type'], 'image/png');
        expect(req.bodyBytes, png);
        expect(typed().join(), '/h/inbox/img_1.png');
        expect(c.imagePaste.value, isNull);
        c.dispose();
      },
    );

    test('a failure types nothing and says why', () async {
      stage = (_) async => http.Response(
        jsonEncode({'error': 'an image is at most 10 MiB'}),
        413,
      );
      final c = controller();
      await c.pasteImages([PastedImage(png, 'image/png')]);
      await Future<void>.delayed(const Duration(milliseconds: 50));
      expect(typed(), isEmpty);
      expect(c.imagePaste.value?.error, contains('at most 10 MiB'));
      c.dismissPaste();
      expect(c.imagePaste.value, isNull);
      c.dispose();
    });

    test('a cancelled paste never types its path', () async {
      final answer = Completer<http.Response>();
      stage = (_) => answer.future;
      final c = controller();
      final done = c.pasteImages([PastedImage(png, 'image/png')]);
      await Future<void>.delayed(const Duration(milliseconds: 350));
      c.dismissPaste();
      answer.complete(http.Response(jsonEncode({'path': '/h/x.png'}), 200));
      await done;
      await Future<void>.delayed(const Duration(milliseconds: 50));
      expect(typed(), isEmpty);
      expect(c.imagePaste.value, isNull);
      c.dispose();
    });
  });

  group('the paste keys in the terminal', () {
    Future<List<String>> press(
      WidgetTester tester,
      Future<bool> Function() image,
      List<LogicalKeyboardKey> modifiers,
    ) async {
      final t = StormTerminal();
      addTearDown(t.dispose);
      final sent = <String>[];
      t.onInput = (b) => sent.add(latin1.decode(b));
      t.onImagePaste = image;
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(body: StormTerminalView(terminal: t, autofocus: true)),
        ),
      );
      await tester.pump();
      for (final m in modifiers) {
        await tester.sendKeyDownEvent(m);
      }
      await tester.sendKeyEvent(LogicalKeyboardKey.keyV);
      for (final m in modifiers.reversed) {
        await tester.sendKeyUpEvent(m);
      }
      await tester.pump();
      return sent;
    }

    testWidgets('Ctrl+V with an image sends no ^V', (tester) async {
      var asked = 0;
      final sent = await press(tester, () async {
        asked++;
        return true;
      }, [LogicalKeyboardKey.controlLeft]);
      expect(asked, 1);
      expect(sent, isEmpty);
    });

    testWidgets('Ctrl+V without an image is ^V, as before', (tester) async {
      final sent = await press(tester, () async => false, [
        LogicalKeyboardKey.controlLeft,
      ]);
      expect(sent, ['\x16']);
    });
  });
}

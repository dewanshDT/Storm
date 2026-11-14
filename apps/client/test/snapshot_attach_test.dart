import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:xterm2/xterm.dart';

import 'package:storm/agent/agent_api.dart';
import 'package:storm/agent/agent_models.dart';
import 'package:storm/agent/session_controller.dart';
import 'package:storm/agent/terminal_events.dart';
import 'package:storm/agent/terminal_stream_http.dart';
import 'package:storm/agent/terminal_surface.dart';

import 'agent_fakes.dart';

/// Decision 154, slice 154-5: the client renders the server's snapshot on a
/// cold open (AM110).
///
/// The fixtures under `test/fixtures/snapshot/` are the server's own
/// renderer (`apps/server/src/agent/snapshot.rs`, 154-S3) run over its
/// Claude Code capture (`claude_code_screen.bin`, 100×30), over a 4 MiB
/// Claude Code corpus whose cache evicted half of it (120×40, floor above 0),
/// and over 24 KiB of main-screen lines (80×24): the event's `data` JSON as
/// sent, and vt100's view of the same bytes (`.expect.json`).
void main() {
  Uint8List bytes(String s) => Uint8List.fromList(utf8.encode(s));

  AgentSession record({int? outputEnd, int cols = 80, int rows = 24}) =>
      AgentSession.fromJson({
        ...agentSession('ags_1'),
        'cols': cols,
        'rows': rows,
        'output_end': ?outputEnd,
      });

  String fixture(String name) =>
      File('test/fixtures/snapshot/$name').readAsStringSync();

  SnapshotEvent snapshotFixture(String name) =>
      SseParser.fromParts('snapshot', null, fixture('$name.json'))
          as SnapshotEvent;

  Map<String, dynamic> expected(String name) =>
      jsonDecode(fixture('$name.expect.json')) as Map<String, dynamic>;

  SnapshotEvent snapshot(
    String data, {
    int offset = 1000,
    int floor = 0,
    bool? complete,
  }) => SnapshotEvent(
    offset: offset,
    floor: floor,
    complete: complete ?? floor == 0,
    alt: false,
    historyRows: 0,
    modes: const {},
    data: bytes('\x1bc$data'),
  );

  Future<void> settle([int ms = 10]) =>
      Future<void>.delayed(Duration(milliseconds: ms));

  group('the event', () {
    test('decodes as the server sends it', () {
      final e = SseParser().feed(
        'event: snapshot\nid: 5803\ndata: ${fixture('claude_code.json')}\n\n',
      );
      final s = e.single as SnapshotEvent;
      expect((s.offset, s.floor, s.complete, s.alt), (5803, 0, true, true));
      expect(s.modes['kitty_flags'], 5);
      expect(s.data.sublist(0, 2), [0x1b, 0x63]);

      final evicted = snapshotFixture('claude_code_evicted');
      expect(evicted.floor, greaterThan(0));
      expect(evicted.complete, isFalse);
    });

    test('the web listens for it: every event name decodes', () {
      expect(SseParser.eventNames, contains('snapshot'));
      final samples = {
        'status': jsonEncode(agentSession('ags_1')),
        'output': base64.encode([0x61]),
        'gap': '{"from":0,"to":9}',
        'live': '9',
        'snapshot': fixture('claude_code.json'),
      };
      expect(SseParser.eventNames.toSet(), samples.keys.toSet());
      for (final name in SseParser.eventNames) {
        expect(
          SseParser.fromParts(name, '9', samples[name]!),
          isNotNull,
          reason: name,
        );
      }
    });

    test('the streamed GET asks for it only when told to', () async {
      final asked = <Uri>[];
      final api = AgentApi(
        baseUrl: 'http://s',
        token: 't',
        client: MockClient((req) async {
          asked.add(req.url);
          return http.Response('', 200);
        }),
      );
      await openStreamedTerminal(api, 'ags_1', 0, true).drain<void>();
      await openStreamedTerminal(api, 'ags_1', 42, false).drain<void>();
      expect(asked[0].queryParameters, {'offset': '0', 'attach': 'snapshot'});
      expect(asked[1].queryParameters, {'offset': '42'});
    });
  });

  group('the session controller', () {
    ({
      SessionController c,
      List<(int, bool)> opened,
      List<String> posted,
      List<String> inputs,
      List<String> replies,
      StreamController<TerminalEvent> Function() current,
    })
    open({
      Duration grace = const Duration(milliseconds: 50),
      (int, int)? size,
    }) {
      final opened = <(int, bool)>[];
      final posted = <String>[], inputs = <String>[], replies = <String>[];
      late StreamController<TerminalEvent> current;
      final c = SessionController(
        api: AgentApi(
          baseUrl: 'http://s',
          token: 't',
          client: MockClient((req) async {
            if (req.url.path.endsWith('/terminal/input')) {
              posted.add(latin1.decode(req.bodyBytes));
            }
            return http.Response('', 204);
          }),
        ),
        sessionId: 'ags_1',
        open: (offset, snapshot) {
          opened.add((offset, snapshot));
          current = StreamController<TerminalEvent>();
          return current.stream;
        },
        isForeground: () => true,
        hiddenGrace: () => grace,
      );
      if (size != null) c.terminal.emulatorForTest.resize(size.$1, size.$2);
      c.start();
      final onInput = c.terminal.onInput!, onReply = c.terminal.onReply!;
      c.terminal.onInput = (b) {
        inputs.add(latin1.decode(b));
        onInput(b);
      };
      c.terminal.onReply = (b) {
        replies.add(latin1.decode(b));
        onReply(b);
      };
      return (
        c: c,
        opened: opened,
        posted: posted,
        inputs: inputs,
        replies: replies,
        current: () => current,
      );
    }

    test('a cold open asks for one; it resets, renders, and output goes on '
        'from S', () async {
      final o = open();
      expect(o.opened, [(0, true)]);
      o.current()
        ..add(StatusEvent(record()))
        ..add(snapshot('\x1b[H\x1b[2Jscreen at S', offset: 4000))
        ..add(OutputEvent(4005, bytes(' more')))
        ..add(LiveEvent());
      await settle();
      expect(o.c.terminal.textForTest.trim(), 'screen at S more');
      expect(o.c.snapshotFloor, 0);

      // A drop mid-view resumes exactly, without asking again.
      await o.current().close();
      await settle(1100);
      expect(o.opened, [(0, true), (4005, false)]);
      o.c.dispose();
    });

    test('nothing it draws is answered or typed, even once the quiet '
        'timer has passed', () async {
      final o = open();
      o.current().add(StatusEvent(record()));
      await settle(SessionController.liveAfterQuiet.inMilliseconds + 100);
      // A snapshot holds no queries; were one there, it is still replay.
      o.current()
        ..add(snapshot('\x1b[c\x1b[6n\x1b[?u\x1b[=5;1u\x1b[?2031h', offset: 50))
        ..add(LiveEvent());
      await settle(100);
      expect(o.inputs, isEmpty, reason: 'drawing is never typing');
      expect(o.replies, isNotEmpty, reason: 'xterm2 did answer');
      expect(o.posted, isEmpty, reason: 'and every answer was dropped');

      o.current().add(OutputEvent(53, bytes('\x1b[c')));
      await settle(100);
      expect(o.posted, hasLength(1), reason: 'live output is answered');
      o.c.dispose();
    });

    test('a status between snapshot, output and live changes nothing it '
        'draws', () async {
      final o = open();
      o.current()
        ..add(StatusEvent(record(outputEnd: 4000)))
        ..add(snapshot('at S', offset: 4000))
        ..add(StatusEvent(record(outputEnd: 4003)))
        ..add(OutputEvent(4003, bytes(' +1')))
        ..add(StatusEvent(record(outputEnd: 4003)))
        ..add(LiveEvent())
        ..add(StatusEvent(record(outputEnd: 4006)))
        ..add(OutputEvent(4006, bytes(' +2')));
      await settle();
      expect(o.opened, [(0, true)], reason: 'no end below the offset');
      expect(o.c.terminal.textForTest.trim(), 'at S +1 +2');
      o.c.dispose();
    });

    test('an old server sends none: the full replay still renders', () async {
      final o = open();
      expect(o.opened.single, (0, true), reason: 'it ignores attach');
      o.current()
        ..add(StatusEvent(record()))
        ..add(OutputEvent(6, bytes('first ')))
        ..add(OutputEvent(12, bytes('second')))
        ..add(LiveEvent());
      await settle();
      expect(o.c.terminal.textForTest.trim(), 'first second');

      o.c.hide();
      await settle(80);
      o.c.show();
      await settle();
      expect(o.opened.last, (12, true));
      o.current()
        ..add(StatusEvent(record()))
        ..add(GapEvent(12, 9000))
        ..add(OutputEvent(9004, bytes('tail')));
      await settle();
      final text = o.c.terminal.textForTest;
      expect(text, isNot(contains('first')), reason: 'cleared at view start');
      expect(text, contains('no longer retained'));
      expect(text, contains('tail'));
      o.c.dispose();
    });

    test('an incomplete one says earlier output is gone, above what it '
        'draws', () async {
      final o = open();
      o.current()
        ..add(StatusEvent(record()))
        ..add(snapshot('line one\r\nline two', offset: 9000, floor: 4096));
      await settle();
      final text = o.c.terminal.textForTest;
      expect(text, contains('no longer retained'));
      expect(
        text.indexOf('no longer retained'),
        lessThan(text.indexOf('line one')),
      );
      expect(o.c.snapshotFloor, 4096);
      o.c.dispose();
    });

    group('with a kept terminal (154-2)', () {
      Future<
        ({
          SessionController c,
          List<(int, bool)> opened,
          StreamController<TerminalEvent> Function() current,
        })
      >
      kept() async {
        final o = open();
        o.current()
          ..add(StatusEvent(record()))
          ..add(OutputEvent(5, bytes('hello')))
          ..add(LiveEvent());
        await settle();
        o.c.hide();
        await settle(80);
        o.c.show();
        await settle();
        return (c: o.c, opened: o.opened, current: o.current);
      }

      test('a resume the server still has every byte for is exact', () async {
        final k = await kept();
        expect(k.opened, [(0, true), (5, true)]);
        k.current()
          ..add(StatusEvent(record()))
          ..add(OutputEvent(8, bytes(' w!')))
          ..add(LiveEvent());
        await settle();
        expect(k.c.terminal.textForTest.trim(), 'hello w!');
        k.c.dispose();
      });

      test('a resume below the floor takes the snapshot, not a replay from '
          'it', () async {
        final k = await kept();
        k.current()
          ..add(StatusEvent(record()))
          ..add(snapshot('now', offset: 9000, floor: 4096))
          ..add(OutputEvent(9004, bytes(' on')))
          ..add(LiveEvent());
        await settle();
        final text = k.c.terminal.textForTest;
        expect(text, isNot(contains('hello')));
        expect(text, contains('no longer retained'));
        expect(text, contains('now on'));

        await k.current().close();
        await settle(1100);
        expect(k.opened.last, (9004, false));
        k.c.dispose();
      });

      test('a renumbered stream reopens from 0 asking again', () async {
        final k = await kept();
        k.current().add(StatusEvent(record(outputEnd: 2)));
        await settle();
        expect(k.opened, [(0, true), (5, true), (0, true)]);
        k.c.dispose();
      });
    });

    group("the server's own renders", () {
      // `getText` skips cells nothing wrote, where vt100 reads a space.
      String row(BufferLine line) {
        final out = StringBuffer();
        for (var i = 0; i < line.length; i++) {
          final width = line.getWidth(i);
          if (width == 0 && i > 0 && line.getWidth(i - 1) == 2) continue;
          final code = line.getCodePoint(i);
          out.writeCharCode(code == 0 ? 0x20 : code);
        }
        return out.toString().trimRight();
      }

      List<String> screenRows(Buffer b) => [
        for (var i = b.scrollBack; i < b.height; i++) row(b.lines[i]),
      ];

      String trimEnd(List<String> rows) {
        final out = [...rows];
        while (out.isNotEmpty && out.last.isEmpty) {
          out.removeLast();
        }
        return out.join('\n');
      }

      String rows(Map<String, dynamic> want) => trimEnd([
        for (final r in (want['contents'] as String).split('\n')) r.trimRight(),
      ]);

      ({SessionController c, Terminal t}) render(String name) {
        final event = snapshotFixture(name);
        final j = jsonDecode(fixture('$name.json')) as Map<String, dynamic>;
        final size = (j['cols'] as int, j['rows'] as int);
        final o = open(size: size);
        o.current()
          ..add(StatusEvent(record(cols: size.$1, rows: size.$2)))
          ..add(event)
          ..add(LiveEvent());
        return (c: o.c, t: o.c.terminal.emulatorForTest);
      }

      test('Claude Code comes back on the alt screen with its text, cursor '
          'and modes', () async {
        final r = render('claude_code');
        await settle();
        final want = expected('claude_code');
        final t = r.t;
        expect(t.isUsingAltBuffer, isTrue);
        expect(trimEnd(screenRows(t.buffer)), rows(want));
        expect([t.buffer.cursorY, t.buffer.cursorX], want['cursor']);
        expect(t.kittyKeyboardMode, 5);
        expect(t.modifyOtherKeysMode, 2);
        expect(t.bracketedPasteMode, isTrue);
        expect(t.reportFocusMode, isTrue);
        expect(t.reportColorSchemeMode, isTrue);
        expect(t.mouseMode, MouseMode.upDownScrollMove);
        expect(t.mouseReportMode, MouseReportMode.sgr);
        expect(t.cursorVisibleMode, isTrue);
        expect(t.cursorKeysMode, isFalse);
        r.c.dispose();
      });

      test('past an eviction it is still the alt screen, and the note waits '
          'on the main one', () async {
        final r = render('claude_code_evicted');
        await settle();
        final want = expected('claude_code_evicted');
        final t = r.t;
        expect(t.isUsingAltBuffer, isTrue);
        expect(trimEnd(screenRows(t.buffer)), rows(want));
        expect([t.buffer.cursorY, t.buffer.cursorX], want['cursor']);
        expect(t.kittyKeyboardMode, 5);
        expect(t.cursorVisibleMode, isFalse);
        expect(t.mainBuffer.getText(), contains('no longer retained'));
        r.c.dispose();
      });

      test('a main-screen session gets its history above the screen', () async {
        final r = render('main_screen');
        await settle();
        final want = expected('main_screen');
        final j =
            jsonDecode(fixture('main_screen.json')) as Map<String, dynamic>;
        final t = r.t;
        expect(t.isUsingAltBuffer, isFalse);
        expect(t.buffer.scrollBack, greaterThanOrEqualTo(j['history_rows']));
        expect(t.buffer.getText(), contains('000000 abcdefghijklmnopqrst'));
        expect([t.buffer.cursorY, t.buffer.cursorX], want['cursor']);
        // vt100 joins a wrapped row to the next; so does reading xterm2's
        // screen without the row breaks.
        String flat(String s) => s.replaceAll('\n', '');
        expect(flat(trimEnd(screenRows(t.buffer))), flat(rows(want)));
        r.c.dispose();
      });

      testWidgets('after a Claude Code snapshot, Shift+Enter is the kitty '
          'CSI 13;2u', (tester) async {
        final r = render('claude_code');
        await tester.pump();
        expect(r.t.kittyKeyboardMode, 5);
        final sent = <String>[];
        r.c.terminal.onInput = (b) => sent.add(latin1.decode(b));
        await tester.pumpWidget(
          MaterialApp(
            home: Scaffold(
              body: StormTerminalView(terminal: r.c.terminal, autofocus: true),
            ),
          ),
        );
        await tester.pump();
        await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
        await tester.sendKeyEvent(LogicalKeyboardKey.enter);
        await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
        await tester.pump();
        expect(sent, contains('\x1b[13;2u'));
        expect(sent, isNot(contains('\n')));
        await tester.pumpWidget(const SizedBox());
        r.c.dispose();
      });
    });
  });
}

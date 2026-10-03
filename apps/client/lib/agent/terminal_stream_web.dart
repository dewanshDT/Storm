import 'dart:async';
import 'dart:js_interop';

import 'package:web/web.dart' as web;

import 'agent_api.dart';
import 'terminal_events.dart';

Stream<TerminalEvent> openTerminalStream(
  AgentApi api,
  String sessionId,
  int offset,
) {
  late final StreamController<TerminalEvent> controller;
  web.EventSource? source;

  Future<void> open() async {
    final ticket = await api.streamTicket();
    final url = Uri.parse(
      '${api.baseUrl}/v1/agent/sessions/${Uri.encodeComponent(sessionId)}'
      '/terminal/stream',
    ).replace(queryParameters: {'offset': '$offset', 'ticket': ticket});
    final es = web.EventSource(url.toString());
    source = es;

    void listen(String name) {
      es.addEventListener(
        name,
        ((web.MessageEvent e) {
          final data = (e.data as JSString?)?.toDart ?? '';
          final id = e.lastEventId.isEmpty ? null : e.lastEventId;
          final event = SseParser.fromParts(name, id, data);
          if (event != null && !controller.isClosed) controller.add(event);
        }).toJS,
      );
    }

    listen('status');
    listen('output');
    listen('gap');
    // The browser would reconnect by itself, with the spent ticket and
    // `Last-Event-ID`. Neither is how this stream resumes, so any error or
    // close ends it here, and the session controller reconnects by offset
    // with a fresh ticket.
    es.onerror = ((web.Event _) {
      es.close();
      if (!controller.isClosed) controller.close();
    }).toJS;
  }

  controller = StreamController<TerminalEvent>(
    onListen: () => open().catchError((Object e) {
      if (!controller.isClosed) {
        controller.addError(e);
        controller.close();
      }
    }),
    onCancel: () => source?.close(),
  );
  return controller.stream;
}

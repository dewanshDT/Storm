import 'dart:async';
import 'dart:js_interop';

import 'package:web/web.dart' as web;

import '../api/connection_client.dart';
import 'agent_api.dart';
import 'session_controller.dart';
import 'terminal_events.dart';
import 'terminal_stream_http.dart';

/// How long a hidden session keeps its stream open (decision 154): none on
/// web-direct, where each `EventSource` holds one of the browser's six
/// HTTP/1.1 sockets to the origin, shared by every tab.
Duration hiddenStreamGrace(AgentApi api) => switch (api.client) {
  ConnectionClient(isRelayed: true) => SessionController.streamGrace,
  _ => Duration.zero,
};

/// `EventSource` to the page's own server; over the relay, the streamed GET
/// instead, since `EventSource` cannot ride the tunnel and the tunnel, unlike
/// the browser's `http` client, does not buffer (decision 112).
Stream<TerminalEvent> openTerminalStream(
  AgentApi api,
  String sessionId,
  int offset,
  bool snapshot,
) {
  if (api.client case ConnectionClient(isRelayed: true)) {
    return openStreamedTerminal(api, sessionId, offset, snapshot);
  }
  late final StreamController<TerminalEvent> controller;
  web.EventSource? source;

  Future<void> open() async {
    final ticket = await api.streamTicket();
    final url =
        Uri.parse(
          '${api.baseUrl}/v1/agent/sessions/${Uri.encodeComponent(sessionId)}'
          '/terminal/stream',
        ).replace(
          queryParameters: {...streamQuery(offset, snapshot), 'ticket': ticket},
        );
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

    SseParser.eventNames.forEach(listen);
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

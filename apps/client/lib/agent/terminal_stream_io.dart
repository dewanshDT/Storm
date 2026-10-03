import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

import '../api/models.dart';
import 'agent_api.dart';
import 'terminal_events.dart';

/// The native stream: a streamed GET with the bearer header.
///
/// **A [StreamController], not an `async*` generator.** A generator learns of
/// a cancel only at its next `yield`, and while it waits for the next SSE
/// chunk that can be the 15 s keepalive away. Closing a tab would hold the
/// connection that long. Here, cancel tears the response down at once.
Stream<TerminalEvent> openTerminalStream(
  AgentApi api,
  String sessionId,
  int offset,
) {
  final client = http.Client();
  StreamSubscription<String>? body;
  late final StreamController<TerminalEvent> controller;

  Future<void> start() async {
    try {
      final request =
          http.Request(
              'GET',
              Uri.parse(
                '${api.baseUrl}/v1/agent/sessions/${Uri.encodeComponent(sessionId)}'
                '/terminal/stream',
              ).replace(queryParameters: {'offset': '$offset'}),
            )
            ..headers.addAll({
              'Authorization': 'Bearer ${api.token}',
              'Accept': 'text/event-stream',
            });
      final response = await client.send(request);
      if (controller.isClosed) return;
      if (response.statusCode != 200) {
        throw StormApiException(
          response.statusCode,
          'HTTP ${response.statusCode}',
        );
      }
      final parser = SseParser();
      body = response.stream
          .transform(utf8.decoder)
          .listen(
            (text) {
              for (final event in parser.feed(text)) {
                controller.add(event);
              }
            },
            onError: controller.addError,
            onDone: controller.close,
            cancelOnError: true,
          );
    } catch (e) {
      if (!controller.isClosed) {
        controller.addError(e);
        await controller.close();
      }
    }
  }

  controller = StreamController<TerminalEvent>(
    onListen: start,
    onCancel: () async {
      await body?.cancel();
      client.close();
    },
  );
  return controller.stream;
}

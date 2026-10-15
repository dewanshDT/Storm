import 'agent_api.dart';
import 'terminal_events.dart';
import 'terminal_stream_http.dart';

/// The native stream: a streamed GET with the bearer header.
Stream<TerminalEvent> openTerminalStream(
  AgentApi api,
  String sessionId,
  int offset,
) => openStreamedTerminal(api, sessionId, offset);

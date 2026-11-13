import 'agent_api.dart';
import 'session_controller.dart';
import 'terminal_events.dart';
import 'terminal_stream_http.dart';

/// How long a hidden session keeps its stream open (decision 154).
Duration hiddenStreamGrace(AgentApi api) => SessionController.streamGrace;

/// The native stream: a streamed GET with the bearer header.
Stream<TerminalEvent> openTerminalStream(
  AgentApi api,
  String sessionId,
  int offset,
  bool snapshot,
) => openStreamedTerminal(api, sessionId, offset, snapshot);

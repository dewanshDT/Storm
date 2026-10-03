import 'dart:convert';
import 'dart:typed_data';

import 'agent_models.dart';

/// One event from a session's terminal stream (freeze §11.2).
sealed class TerminalEvent {}

/// The whole session record. Every stream starts with one.
class StatusEvent extends TerminalEvent {
  StatusEvent(this.session);
  final AgentSession session;
}

/// Output bytes. [end] is the absolute offset just past them: what a reader
/// resumes from.
class OutputEvent extends TerminalEvent {
  OutputEvent(this.end, this.bytes);
  final int end;
  final Uint8List bytes;
}

/// `from..to` is no longer retained. Resume from [to].
class GapEvent extends TerminalEvent {
  GapEvent(this.from, this.to);
  final int from;
  final int to;
}

/// Server-sent events, parsed incrementally (the subset Storm sends:
/// `event:`, `id:`, `data:`, and `:` comments as keepalives).
class SseParser {
  final _buffer = StringBuffer();

  /// Feeds decoded text; returns every event it completed.
  List<TerminalEvent> feed(String chunk) {
    _buffer.write(chunk);
    final text = _buffer.toString().replaceAll('\r\n', '\n');
    final out = <TerminalEvent>[];
    var start = 0;
    while (true) {
      final end = text.indexOf('\n\n', start);
      if (end < 0) break;
      final event = decode(text.substring(start, end));
      if (event != null) out.add(event);
      start = end + 2;
    }
    _buffer
      ..clear()
      ..write(text.substring(start));
    return out;
  }

  /// One complete SSE block → a [TerminalEvent], or null for a keepalive or
  /// an event this client does not know.
  static TerminalEvent? decode(String block) {
    var name = 'message';
    String? id;
    final data = <String>[];
    for (final line in block.split('\n')) {
      if (line.isEmpty || line.startsWith(':')) continue;
      final colon = line.indexOf(':');
      final field = colon < 0 ? line : line.substring(0, colon);
      var value = colon < 0 ? '' : line.substring(colon + 1);
      if (value.startsWith(' ')) value = value.substring(1);
      switch (field) {
        case 'event':
          name = value;
        case 'id':
          id = value;
        case 'data':
          data.add(value);
      }
    }
    return fromParts(name, id, data.join('\n'));
  }

  static TerminalEvent? fromParts(String name, String? id, String data) {
    switch (name) {
      case 'status':
        return StatusEvent(
          AgentSession.fromJson(jsonDecode(data) as Map<String, dynamic>),
        );
      case 'output':
        final end = int.tryParse(id ?? '');
        if (end == null) return null;
        return OutputEvent(end, base64.decode(data));
      case 'gap':
        final j = jsonDecode(data) as Map<String, dynamic>;
        return GapEvent((j['from'] as num).toInt(), (j['to'] as num).toInt());
    }
    return null;
  }
}

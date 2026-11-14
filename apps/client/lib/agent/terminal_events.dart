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

/// Everything retained has been sent: what came before was replay
/// (decision 132, fix b). An older server never sends it.
class LiveEvent extends TerminalEvent {}

/// The screen at [offset], rendered by the server from its cache, in place of
/// the history before it (AM110): VT bytes that restore the grid, cursor and
/// modes, and ask nothing. Output resumes from [offset]. [complete] is false
/// when the cache had already dropped output from before [floor].
class SnapshotEvent extends TerminalEvent {
  SnapshotEvent({
    required this.offset,
    required this.floor,
    required this.complete,
    required this.alt,
    required this.historyRows,
    required this.modes,
    required this.data,
  });
  final int offset;
  final int floor;
  final bool complete;
  final bool alt;
  final int historyRows;
  final Map<String, Object?> modes;
  final Uint8List data;
}

/// `from..to` is no longer retained. Resume from [to].
class GapEvent extends TerminalEvent {
  GapEvent(this.from, this.to);
  final int from;
  final int to;
}

/// Server-sent events, parsed incrementally (the subset Storm sends:
/// `event:`, `id:`, `data:`, and `:` comments as keepalives).
///
/// Each chunk is scanned once, so a large event arriving in many chunks costs
/// its length, not its length squared.
class SseParser {
  /// The incomplete block so far, CRLF already folded to LF.
  final _parts = <String>[];
  bool _endsInLf = false;

  /// A chunk's trailing CR, held until the next one says if an LF follows.
  bool _heldCr = false;

  /// Feeds decoded text; returns every event it completed.
  List<TerminalEvent> feed(String chunk) {
    if (_heldCr) {
      chunk = '\r$chunk';
      _heldCr = false;
    }
    if (chunk.endsWith('\r')) {
      chunk = chunk.substring(0, chunk.length - 1);
      _heldCr = true;
    }
    final text = chunk.replaceAll('\r\n', '\n');
    if (text.isEmpty) return const [];
    final out = <TerminalEvent>[];
    var start = 0;
    if (_endsInLf && text.startsWith('\n')) {
      final last = _parts.removeLast();
      _parts.add(last.substring(0, last.length - 1));
      _complete('', out);
      start = 1;
    }
    while (true) {
      final end = text.indexOf('\n\n', start);
      if (end < 0) break;
      _complete(text.substring(start, end), out);
      start = end + 2;
    }
    if (start < text.length) {
      _parts.add(text.substring(start));
      _endsInLf = text.endsWith('\n');
    }
    return out;
  }

  void _complete(String tail, List<TerminalEvent> out) {
    final block = _parts.isEmpty ? tail : (_parts..add(tail)).join();
    _parts.clear();
    _endsInLf = false;
    final event = decode(block);
    if (event != null) out.add(event);
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

  /// Every event name [fromParts] decodes: what the web's `EventSource`
  /// listens for, since it delivers only named events it was asked for.
  static const eventNames = ['status', 'output', 'gap', 'live', 'snapshot'];

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
      case 'live':
        return LiveEvent();
      case 'gap':
        final j = jsonDecode(data) as Map<String, dynamic>;
        return GapEvent((j['from'] as num).toInt(), (j['to'] as num).toInt());
      case 'snapshot':
        final j = jsonDecode(data) as Map<String, dynamic>;
        final floor = (j['floor'] as num?)?.toInt() ?? 0;
        return SnapshotEvent(
          offset: (j['offset'] as num).toInt(),
          floor: floor,
          complete: j['complete'] as bool? ?? floor == 0,
          alt: j['alt'] as bool? ?? false,
          historyRows: (j['history_rows'] as num?)?.toInt() ?? 0,
          modes: (j['modes'] as Map<String, dynamic>?) ?? const {},
          data: base64.decode(j['data'] as String),
        );
    }
    return null;
  }
}

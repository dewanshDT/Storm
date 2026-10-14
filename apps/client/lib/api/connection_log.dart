import 'dart:collection';

import 'relay/srp_http_client.dart';
import 'relay/srp_trunk.dart';

/// One line of [ConnectionLog].
class ConnectionLogEntry {
  const ConnectionLogEntry(this.at, this.event);
  final DateTime at;
  final String event;

  @override
  String toString() => '${at.toUtc().toIso8601String()} $event';
}

/// A bounded, in-memory record of how the app reached its server: races,
/// probes, trunks, resets and status changes (decision 113).
///
/// Release builds log nothing to logcat, so this is the on-device evidence
/// Settings › Connection can show and copy. It holds hosts, tiers, timings
/// and reason codes only: never a token, key, ticket, path or note content.
/// It lives in memory and is lost when the app exits.
class ConnectionLog {
  ConnectionLog({this.capacity = 400});

  final int capacity;
  final _entries = ListQueue<ConnectionLogEntry>();

  /// The app-wide log.
  static final ConnectionLog instance = ConnectionLog();

  List<ConnectionLogEntry> get entries => List.unmodifiable(_entries);

  void add(String event) {
    _entries.add(ConnectionLogEntry(DateTime.now(), event));
    while (_entries.length > capacity) {
      _entries.removeFirst();
    }
  }

  void clear() => _entries.clear();

  @override
  String toString() => _entries.join('\n');
}

/// What a log line may say about an address: scheme, host and port only.
String logHost(String url) {
  final u = Uri.tryParse(url);
  if (u == null || u.host.isEmpty) return '?';
  return u.hasPort
      ? '${u.scheme}://${u.host}:${u.port}'
      : '${u.scheme}://${u.host}';
}

/// An error's type and, for the few transport errors whose text is a code,
/// that code. Never the message of an arbitrary exception, which can carry a
/// URL with a ticket in it.
String logError(Object e) => switch (e) {
  SrpTransportException(:final code) => 'transport $code',
  SrpHandshakeFailed(:final code) => 'handshake ${code ?? 'no answer'}',
  SrpOpenRefused(:final code) => 'refused $code',
  _ => e.runtimeType.toString(),
};

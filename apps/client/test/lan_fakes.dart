import 'dart:async';

import 'package:storm/api/lan_discovery.dart';

/// A browser the test drives: [emit] stands in for mDNS answers.
class FakeLanBrowser implements LanBrowser {
  StreamController<List<DiscoveredServer>>? _out;
  int starts = 0;
  int stops = 0;

  bool get browsing => _out != null;

  @override
  Stream<List<DiscoveredServer>> start() {
    starts++;
    return (_out = StreamController<List<DiscoveredServer>>()).stream;
  }

  void emit(List<DiscoveredServer> servers) => _out?.add(servers);

  @override
  Future<void> stop() async {
    stops++;
    await _out?.close();
    _out = null;
  }
}

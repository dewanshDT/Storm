/// Settings › Connection's "relayed although on the same network" hint
/// (decision 119, Layer 3).
library;

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/connection_log.dart';
import '../../api/lan_discovery.dart';
import '../lan_servers.dart'
    show lanBrowserFactoryProvider, lanDiscoverySupportedProvider;
import 'settings_widgets.dart';

/// While the route is relayed, browses for the paired server's own
/// advertisement. Seeing it means the server is on this network and only its
/// direct address failed, which is almost always the host firewall.
class RelayedOnLanHint extends ConsumerStatefulWidget {
  const RelayedOnLanHint({
    super.key,
    required this.relayed,
    required this.serverId,
  });

  final bool relayed;
  final String serverId;

  @override
  ConsumerState<RelayedOnLanHint> createState() => _RelayedOnLanHintState();
}

class _RelayedOnLanHintState extends ConsumerState<RelayedOnLanHint> {
  LanBrowser? _browser;
  StreamSubscription<List<DiscoveredServer>>? _sub;
  DiscoveredServer? _seen;

  bool get _wanted =>
      widget.relayed &&
      widget.serverId.isNotEmpty &&
      ref.read(lanDiscoverySupportedProvider);

  @override
  void initState() {
    super.initState();
    if (_wanted) _start();
  }

  @override
  void didUpdateWidget(RelayedOnLanHint old) {
    super.didUpdateWidget(old);
    if (old.relayed == widget.relayed && old.serverId == widget.serverId) {
      return;
    }
    _stop();
    _seen = null;
    if (_wanted) _start();
  }

  void _start() {
    final browser = _browser = ref.read(lanBrowserFactoryProvider)();
    _sub = browser.start().listen((servers) {
      final mine = servers
          .where((s) => s.serverId == widget.serverId)
          .firstOrNull;
      if (!mounted || mine == _seen) return;
      if (mine != null && _seen == null) {
        ConnectionLog.instance.add(
          'lan: server advertised at ${mine.host}:${mine.port} on this '
          'network, but the route is relayed (the direct probe failed)',
        );
      }
      setState(() => _seen = mine);
    }, onError: (_) {});
  }

  void _stop() {
    unawaited(_sub?.cancel());
    _sub = null;
    unawaited(_browser?.stop());
    _browser = null;
  }

  @override
  void dispose() {
    _stop();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.relayed || _seen == null) return const SizedBox.shrink();
    return const SettingsNote(
      "Connected through a relay although you're on the same network as your "
      "server. Allow Storm in that computer's firewall for a faster, private "
      'connection.',
      key: Key('relayed-on-lan'),
    );
  }
}

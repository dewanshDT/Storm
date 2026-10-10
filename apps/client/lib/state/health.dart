import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../agent/agent_state.dart';
import '../agent/integrations_api.dart';
import '../agent/integrations_screen.dart' show integrationsApiFactoryProvider;
import '../ui/widgets.dart' show DotStatus, dotStatusFor;
import 'app_state.dart';
import 'client_version.dart';

String relativeTime(DateTime? then) {
  if (then == null) return 'not yet';
  final d = DateTime.now().difference(then);
  if (d.inSeconds < 45) return 'just now';
  if (d.inMinutes < 60) return '${d.inMinutes}m ago';
  if (d.inHours < 24) return '${d.inHours}h ago';
  return '${d.inDays}d ago';
}

enum HealthTone { good, muted, danger }

@immutable
class HealthRow {
  const HealthRow(this.tone, this.text, {this.route});

  final HealthTone tone;
  final String text;

  /// Where acting on this row happens, if anywhere.
  final String? route;

  @override
  bool operator ==(Object other) =>
      other is HealthRow &&
      other.tone == tone &&
      other.text == text &&
      other.route == route;

  @override
  int get hashCode => Object.hash(tone, text, route);
}

/// The owner's connections, refreshed on the agent list's interval. Null
/// when the server cannot be asked.
final integrationsSummaryProvider =
    FutureProvider.autoDispose<List<Integration>?>((ref) async {
      final timer = Timer(agentRefreshInterval, ref.invalidateSelf);
      ref.onDispose(timer.cancel);
      final api = ref.watch(integrationsApiFactoryProvider)?.call();
      if (api == null) return null;
      try {
        return await api.list();
      } catch (_) {
        return null;
      } finally {
        api.dispose();
      }
    });

String syncLine(DotStatus status, int pending, DateTime? lastSynced) =>
    switch (status) {
      DotStatus.synced => 'Notes synced ${relativeTime(lastSynced)}',
      DotStatus.relayed => 'Notes synced ${relativeTime(lastSynced)} · relayed',
      DotStatus.syncing =>
        pending > 0
            ? 'Sending $pending edit${pending == 1 ? '' : 's'}…'
            : 'Syncing…',
      DotStatus.offline =>
        pending > 0
            ? 'Offline · $pending edit${pending == 1 ? '' : 's'} queued'
            : 'Offline · showing your cached copy',
      DotStatus.untrusted => 'Server identity failed · not syncing',
    };

/// `0.4.0+12` → `(0, 4)`; null when it does not start with two numbers.
(int, int)? _majorMinor(String version) {
  final m = RegExp(r'^v?(\d+)\.(\d+)').firstMatch(version.trim());
  if (m == null) return null;
  return (int.parse(m.group(1)!), int.parse(m.group(2)!));
}

/// Compatible means the same major.minor (plan §5.1).
bool versionsCompatible(String client, String server) {
  final a = _majorMinor(client);
  return a != null && a == _majorMinor(server);
}

/// Only once the server reports its version; an older one says nothing.
final compatibilityRowProvider = Provider.autoDispose<HealthRow?>((ref) {
  final server = ref.watch(serverConfigProvider).value?.version;
  final client = ref.watch(clientVersionProvider).value;
  if (server == null || client == null) return null;
  return versionsCompatible(client, server)
      ? const HealthRow(
          HealthTone.muted,
          'This client and the server are compatible',
        )
      : HealthRow(
          HealthTone.danger,
          'This client ($client) and the server ($server) may not be compatible',
        );
});

/// The health rows the rail dot, its popover and About & health share.
final healthRowsProvider = Provider.autoDispose<List<HealthRow>>((ref) {
  final engine = ref.watch(syncEngineProvider);
  final status = dotStatusFor(
    online: engine.isOnline,
    identityFailed: engine.serverIdentityFailed,
    syncing: engine.isSyncing,
    pending: engine.pendingCount,
    tier: engine.connectionTier,
  );
  final rows = <HealthRow>[
    HealthRow(switch (status) {
      DotStatus.synced || DotStatus.relayed => HealthTone.good,
      DotStatus.untrusted => HealthTone.danger,
      _ => HealthTone.muted,
    }, syncLine(status, engine.pendingCount, engine.lastSyncedAt)),
  ];

  final overview = ref.watch(agentOverviewProvider).value;
  if (overview != null && !overview.unreachable && overview.hosts.isNotEmpty) {
    final n = overview.hosts.length;
    final up = overview.online.length;
    rows.add(
      HealthRow(
        up == n ? HealthTone.good : HealthTone.muted,
        '$up of $n host${n == 1 ? '' : 's'} online',
        route: '/settings/hosts',
      ),
    );
  }

  for (final i in ref.watch(integrationsSummaryProvider).value ?? const []) {
    if (i.disabled) continue;
    if (i.needsReconnect) {
      rows.add(
        HealthRow(
          HealthTone.danger,
          '${i.displayName} needs you to sign in again',
          route: '/settings/integrations',
        ),
      );
    } else if (i.status == 'error') {
      rows.add(
        HealthRow(
          HealthTone.danger,
          "${i.displayName} can't be reached",
          route: '/settings/integrations',
        ),
      );
    }
  }
  final compatibility = ref.watch(compatibilityRowProvider);
  if (compatibility != null) rows.add(compatibility);
  return rows;
});

/// Danger if any row is, else healthy (handoff Q9: green as drawn).
final healthToneProvider = Provider.autoDispose<HealthTone>(
  (ref) => ref.watch(healthRowsProvider).any((r) => r.tone == HealthTone.danger)
      ? HealthTone.danger
      : HealthTone.good,
);

/// Whether a connection needs the owner: the Integrations nav dot.
final integrationsAttentionProvider = Provider.autoDispose<bool>(
  (ref) => (ref.watch(integrationsSummaryProvider).value ?? const []).any(
    (i) => !i.disabled && (i.needsReconnect || i.status == 'error'),
  ),
);

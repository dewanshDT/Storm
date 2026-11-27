/// Status indicators: the sync dot and the save-state label.
library;

import 'package:flutter/material.dart';

import '../api/storm_connection.dart';
import 'tokens.dart';

/// Synced, syncing, offline, or untrusted.
///
/// One dot, one meaning, everywhere it appears — green is synced and nothing
/// else, grey is offline and inactive, amber is work in progress, and red is
/// a server that answered without proving who it is.
class StatusDot extends StatelessWidget {
  const StatusDot({
    super.key,
    this.status,
    this.color,
    this.hollow = false,
    this.size = 8,
    this.ring,
  }) : assert(status != null || color != null);

  final DotStatus? status;

  /// Replaces the status colour, for a dot whose meaning is not a sync state.
  final Color? color;

  /// A ring in the dot's colour instead of a fill.
  final bool hollow;
  final double size;

  /// The ground to punch the dot out of where it overlaps something — the
  /// vault bubble's corner. Null everywhere the dot sits in open space.
  final Color? ring;

  @override
  Widget build(BuildContext context) {
    final t = context.tokens;
    final ink =
        color ??
        switch (status!) {
          DotStatus.synced => t.green,
          DotStatus.syncing => t.amber,
          DotStatus.offline || DotStatus.serverOffline => t.text3,
          DotStatus.untrusted => t.danger,
          DotStatus.relayed => t.accent,
        };
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: hollow ? null : ink,
        border: hollow
            ? Border.all(color: ink, width: t.bw * 1.5)
            : ring == null
            ? null
            : Border.all(color: ring!, width: t.bw * 1.5),
      ),
    );
  }
}

/// Dark ink for anything sitting on an accent tile.
///
/// The tile is always a light colour whatever the theme, so this does not
/// re-derive per preset — and the accent test measures this exact pairing.
const kTileInk = Color(0xFF1A1626);

enum DotStatus { synced, syncing, offline, serverOffline, untrusted, relayed }

/// The one place the engine's three flags become a dot, so the vault bubble,
/// the vault card and the popover cannot disagree about what "synced" is.
///
/// Takes an optional [tier] to distinguish direct vs relayed connections.
/// When [tier] is not `CandidateTier.direct`, returns [DotStatus.relayed]
/// instead of [DotStatus.synced] so the UI can show the transport difference.
DotStatus dotStatusFor({
  required bool online,
  required bool syncing,
  required int pending,
  bool identityFailed = false,
  CandidateTier? tier,
  ConnectionStatus? connection,
}) {
  // Outranks offline: a server that failed the challenge is reachable, and
  // reporting that as "offline" would tell the user to check their wifi when
  // the actual answer is that something is answering for their server.
  if (identityFailed) return DotStatus.untrusted;
  // Same grey as offline — nothing syncs either way — but a different line:
  // a relay answered for the server, so the user's network is not the problem.
  if (!online) {
    return connection == ConnectionStatus.serverOffline
        ? DotStatus.serverOffline
        : DotStatus.offline;
  }
  if (syncing || pending > 0) return DotStatus.syncing;
  if (tier != null && tier != CandidateTier.direct) return DotStatus.relayed;
  return DotStatus.synced;
}

/// Where a note is in its save cycle, as `VersionLine` colours it.
enum SaveTone { good, working, waiting, bad }

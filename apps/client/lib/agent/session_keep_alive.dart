import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/misc.dart' show KeepAliveLink;

import 'agent_models.dart';
import 'session_controller.dart';

/// How many hidden sessions a device keeps, and how much of their terminals
/// (decision 154, Q2). The visible session is never counted or evicted.
@immutable
class KeepAliveBudget {
  const KeepAliveBudget({
    required this.hidden,
    required this.bytes,
    this.ttl = const Duration(minutes: 30),
    this.grace,
    this.evictOnMemoryPressure = false,
  });

  final int hidden;

  /// Of estimated terminal cells ([SessionController.estimatedBytes]).
  final int bytes;

  /// A session hidden longer is let go. Null: never.
  final Duration? ttl;

  /// The hidden stream's grace, when not the connection's own
  /// (`hiddenStreamGrace`). Tests set it.
  final Duration? grace;

  final bool evictOnMemoryPressure;

  static const phone = KeepAliveBudget(
    hidden: 3,
    bytes: 48 << 20,
    evictOnMemoryPressure: true,
  );
  static const desktop = KeepAliveBudget(hidden: 6, bytes: 160 << 20);
  static const web = KeepAliveBudget(hidden: 3, bytes: 64 << 20);

  static KeepAliveBudget forPlatform() {
    if (kIsWeb) return web;
    return switch (defaultTargetPlatform) {
      TargetPlatform.android || TargetPlatform.iOS => phone,
      _ => desktop,
    };
  }
}

class _Kept {
  _Kept(this.controller, this.credentials, this.acquire) : link = acquire();

  final SessionController controller;
  final Object? credentials;
  final KeepAliveLink Function() acquire;
  KeepAliveLink? link;
  Timer? expiry;
  int missing = 0;

  bool get doomed => controller.gone || missing >= 2;
}

/// The recently viewed sessions' controllers, kept alive inside
/// `sessionControllerProvider` by a [KeepAliveLink] each, oldest hidden
/// first. Evicting closes the link and Riverpod disposes the rest, so the
/// provider stays the controllers' only owner.
class SessionKeepAlive {
  SessionKeepAlive(this.budget);

  final KeepAliveBudget budget;

  /// Least recently hidden first; a visible entry sits wherever it was.
  final _kept = <String, _Kept>{};
  bool _disposed = false;

  /// [controller] was built for [id] under [credentials]; it starts visible.
  void track(
    String id,
    SessionController controller,
    Object? credentials,
    KeepAliveLink Function() keepAlive,
  ) {
    _kept.remove(id)?.expiry?.cancel();
    _kept[id] = _Kept(controller, credentials, keepAlive);
  }

  /// The provider for [id] disposed [controller].
  void forget(String id, SessionController controller) {
    final k = _kept[id];
    if (k == null || k.controller != controller) return;
    k.expiry?.cancel();
    _kept.remove(id);
  }

  void hidden(String id) {
    final k = _kept.remove(id);
    if (k == null) return;
    _kept[id] = k;
    if (k.doomed) {
      _evict(k);
      return;
    }
    final ttl = budget.ttl;
    k.expiry?.cancel();
    if (ttl != null && k.link != null) {
      k.expiry = Timer(ttl, () => _evict(k));
    }
    _enforce();
  }

  void shown(String id) {
    final k = _kept[id];
    if (k == null) return;
    k.expiry?.cancel();
    k.expiry = null;
    if (k.link != null || k.doomed) return;
    // Kept on screen by a paused listener after an eviction: hold it again,
    // once out of the lifecycle callback, where Riverpod forbids `Ref`.
    scheduleMicrotask(() {
      if (_disposed || _kept[id] != k || k.link != null) return;
      if (k.controller.visible) k.link = k.acquire();
    });
  }

  /// The controller for [id] learnt its session is gone.
  void gone(String id) {
    final k = _kept[id];
    if (k != null && !k.controller.visible) _evict(k);
  }

  Iterable<_Kept> get _hiddenKept =>
      _kept.values.where((k) => !k.controller.visible && k.link != null);

  void _enforce() {
    while (true) {
      final hidden = _hiddenKept.toList();
      if (hidden.isEmpty) return;
      final bytes = hidden.fold(0, (n, k) => n + k.controller.estimatedBytes);
      if (hidden.length <= budget.hidden && bytes <= budget.bytes) return;
      _evict(hidden.first);
    }
  }

  void _evict(_Kept k) {
    k.expiry?.cancel();
    k.expiry = null;
    final link = k.link;
    k.link = null;
    link?.close();
  }

  /// A logout, a server switch or a new token: nothing built under the old
  /// credentials is kept.
  void credentialsChanged(Object? credentials) {
    for (final k in _kept.values.toList()) {
      if (k.credentials != credentials) _evict(k);
    }
  }

  /// `didHaveMemoryPressure`: every hidden session goes, on a phone.
  void memoryPressure() {
    if (!budget.evictOnMemoryPressure) return;
    _hiddenKept.toList().forEach(_evict);
  }

  /// The app is hidden or paused: every stream closes.
  void background() {
    for (final k in _kept.values) {
      k.controller.background();
    }
  }

  /// The app is back: the visible session reconnects.
  void foreground() {
    for (final k in _kept.values.toList()) {
      k.controller.foreground();
    }
  }

  /// A successful overview read at [askedAt]. A kept session missing from
  /// two in a row was dismissed or deleted elsewhere: web-direct cannot see
  /// the 404 an `EventSource` gets.
  void overviewSeen(List<AgentSession> sessions, DateTime askedAt) {
    final byId = {for (final s in sessions) s.id: s};
    for (final MapEntry(key: id, value: k) in _kept.entries.toList()) {
      final s = byId[id];
      if (s == null) {
        k.missing++;
        if (k.doomed && !k.controller.visible) _evict(k);
      } else {
        k.missing = 0;
        k.controller.checkOutputEnd(s.outputEnd, askedAt);
      }
    }
  }

  @visibleForTesting
  Set<String> get keptIds => {
    for (final MapEntry(key: id, value: k) in _kept.entries)
      if (k.link != null) id,
  };

  void dispose() {
    _disposed = true;
    for (final k in _kept.values) {
      k.expiry?.cancel();
    }
    _kept.clear();
  }
}

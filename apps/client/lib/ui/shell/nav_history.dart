import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../router.dart' show Routes;

/// The app bar's back and forward (decision 153c): the router locations this
/// window has shown, in memory only, never saved.
@immutable
class NavHistory {
  const NavHistory({this.entries = const [], this.index = -1});

  final List<String> entries;
  final int index;

  /// Long enough for a day's wandering; the oldest entries fall off.
  static const limit = 100;

  bool get canBack => index > 0;
  bool get canForward => index >= 0 && index < entries.length - 1;

  /// A new location: it drops anything forward of here, unless it is where
  /// back or forward just went. A browser's own buttons arrive the same way,
  /// as a location one step either side, when [browser] is set.
  NavHistory visit(String location, {bool browser = false}) {
    if (index >= 0 && entries[index] == location) return this;
    if (browser) {
      if (canBack && entries[index - 1] == location) return _at(index - 1);
      if (canForward && entries[index + 1] == location) return _at(index + 1);
    }
    final kept = [...entries.take(index + 1), location];
    final drop = kept.length - limit;
    return NavHistory(
      entries: drop > 0 ? kept.sublist(drop) : kept,
      index: kept.length - 1 - (drop > 0 ? drop : 0),
    );
  }

  NavHistory back() => canBack ? _at(index - 1) : this;
  NavHistory forward() => canForward ? _at(index + 1) : this;

  String? get current => index < 0 ? null : entries[index];

  NavHistory _at(int i) => NavHistory(entries: entries, index: i);
}

class NavHistoryNotifier extends Notifier<NavHistory> {
  @override
  NavHistory build() => const NavHistory();

  /// `/notes` only forwards to a vault, so back would bounce off it.
  void visit(String location) {
    if (location == Routes.notes) return;
    state = state.visit(location, browser: kIsWeb);
  }

  /// Steps back and returns where to go, or null at the start.
  String? back() => _step(state.back());

  String? forward() => _step(state.forward());

  String? _step(NavHistory next) {
    if (identical(next, state)) return null;
    state = next;
    return next.current;
  }
}

final navHistoryProvider = NotifierProvider<NavHistoryNotifier, NavHistory>(
  NavHistoryNotifier.new,
);

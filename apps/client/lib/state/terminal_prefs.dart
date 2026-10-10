import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'app_state.dart';

enum TerminalSpacing {
  compact('Compact', 1.15),
  normal('Default', 1.25),
  relaxed('Relaxed', 1.5);

  const TerminalSpacing(this.label, this.lineHeight);

  final String label;
  final double lineHeight;

  static TerminalSpacing fromName(String? name) =>
      values.firstWhere((v) => v.name == name, orElse: () => normal);
}

enum TerminalPadding {
  tight('Tight', 0.5),
  normal('Default', 1),
  roomy('Roomy', 1.5);

  const TerminalPadding(this.label, this.scale);

  final String label;
  final double scale;

  static TerminalPadding fromName(String? name) =>
      values.firstWhere((v) => v.name == name, orElse: () => normal);
}

/// Settings › Terminal: one set of values for every terminal in this app
/// (D15 AM47). Client-only; never sent to the server.
@immutable
class TerminalPrefs {
  const TerminalPrefs({
    this.fontSize,
    this.spacing = TerminalSpacing.normal,
    this.padding = TerminalPadding.normal,
  });

  static const minFontSize = 10.0;
  static const maxFontSize = 20.0;

  /// Null: each layout's own size.
  final double? fontSize;
  final TerminalSpacing spacing;
  final TerminalPadding padding;

  bool get isDefault =>
      fontSize == null &&
      spacing == TerminalSpacing.normal &&
      padding == TerminalPadding.normal;

  TerminalPrefs copyWith({
    double? fontSize,
    TerminalSpacing? spacing,
    TerminalPadding? padding,
  }) => TerminalPrefs(
    fontSize: fontSize ?? this.fontSize,
    spacing: spacing ?? this.spacing,
    padding: padding ?? this.padding,
  );

  static const _kFontSize = 'storm.terminal.fontSize';
  static const _kSpacing = 'storm.terminal.spacing';
  static const _kPadding = 'storm.terminal.padding';

  static TerminalPrefs read(SharedPreferences prefs) {
    final size = prefs.getDouble(_kFontSize);
    return TerminalPrefs(
      fontSize: size?.clamp(minFontSize, maxFontSize).toDouble(),
      spacing: TerminalSpacing.fromName(prefs.getString(_kSpacing)),
      padding: TerminalPadding.fromName(prefs.getString(_kPadding)),
    );
  }

  Future<void> write(SharedPreferences prefs) async {
    final size = fontSize;
    if (size == null) {
      await prefs.remove(_kFontSize);
    } else {
      await prefs.setDouble(_kFontSize, size);
    }
    await prefs.setString(_kSpacing, spacing.name);
    await prefs.setString(_kPadding, padding.name);
  }

  @override
  bool operator ==(Object other) =>
      other is TerminalPrefs &&
      other.fontSize == fontSize &&
      other.spacing == spacing &&
      other.padding == padding;

  @override
  int get hashCode => Object.hash(fontSize, spacing, padding);
}

/// Seeded from what [SettingsNotifier] read at launch, so a terminal opens at
/// its final size and the agent is not resized a moment later.
class TerminalPrefsNotifier extends Notifier<TerminalPrefs> {
  @override
  TerminalPrefs build() =>
      ref.read(settingsProvider).value?.terminal ?? const TerminalPrefs();

  void set(TerminalPrefs next) {
    if (next == state) return;
    state = next;
    _persist(next);
  }

  void reset() => set(const TerminalPrefs());

  Future<void> _persist(TerminalPrefs p) async {
    try {
      await p.write(await SharedPreferences.getInstance());
    } catch (_) {
      // Tests and previews have no preferences store; memory still works.
    }
  }
}

final terminalPrefsProvider =
    NotifierProvider<TerminalPrefsNotifier, TerminalPrefs>(
      TerminalPrefsNotifier.new,
    );

import 'package:flutter/material.dart';
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

/// Which brightness a terminal may take (decision 157d): the app's, or dark
/// whatever the app is.
enum TerminalColours {
  matchApp('Match app'),
  alwaysDark('Always dark');

  const TerminalColours(this.label);

  final String label;

  static TerminalColours fromName(String? name) =>
      values.firstWhere((v) => v.name == name, orElse: () => matchApp);
}

/// Settings › Terminal: one set of values for every terminal in this app
/// (D15 AM47). Client-only; never sent to the server.
@immutable
class TerminalPrefs {
  const TerminalPrefs({
    this.fontSize,
    this.spacing = TerminalSpacing.normal,
    this.padding = TerminalPadding.normal,
    this.colours = TerminalColours.matchApp,
  });

  static const minFontSize = 10.0;
  static const maxFontSize = 20.0;

  /// Null: each layout's own size.
  final double? fontSize;
  final TerminalSpacing spacing;
  final TerminalPadding padding;
  final TerminalColours colours;

  /// The brightness a terminal answers with and a shell shows (157c): light
  /// only in a light app with [TerminalColours.matchApp].
  Brightness target(Brightness app) =>
      app == Brightness.light && colours == TerminalColours.matchApp
      ? Brightness.light
      : Brightness.dark;

  bool get isDefault =>
      fontSize == null &&
      spacing == TerminalSpacing.normal &&
      padding == TerminalPadding.normal &&
      colours == TerminalColours.matchApp;

  TerminalPrefs copyWith({
    double? fontSize,
    TerminalSpacing? spacing,
    TerminalPadding? padding,
    TerminalColours? colours,
  }) => TerminalPrefs(
    fontSize: fontSize ?? this.fontSize,
    spacing: spacing ?? this.spacing,
    padding: padding ?? this.padding,
    colours: colours ?? this.colours,
  );

  static const _kFontSize = 'storm.terminal.fontSize';
  static const _kSpacing = 'storm.terminal.spacing';
  static const _kPadding = 'storm.terminal.padding';
  static const _kColours = 'storm.terminal.colours';

  static TerminalPrefs read(SharedPreferences prefs) {
    final size = prefs.getDouble(_kFontSize);
    return TerminalPrefs(
      fontSize: size?.clamp(minFontSize, maxFontSize).toDouble(),
      spacing: TerminalSpacing.fromName(prefs.getString(_kSpacing)),
      padding: TerminalPadding.fromName(prefs.getString(_kPadding)),
      colours: TerminalColours.fromName(prefs.getString(_kColours)),
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
    await prefs.setString(_kColours, colours.name);
  }

  @override
  bool operator ==(Object other) =>
      other is TerminalPrefs &&
      other.fontSize == fontSize &&
      other.spacing == spacing &&
      other.padding == padding &&
      other.colours == colours;

  @override
  int get hashCode => Object.hash(fontSize, spacing, padding, colours);
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

/// What a new session's terminal will answer with (157c), sent at launch so
/// the host can set `COLORFGBG` (171-8).
Brightness launchBrightness(BuildContext context, WidgetRef ref) =>
    ref.read(terminalPrefsProvider).target(Theme.of(context).brightness);

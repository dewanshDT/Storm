import 'dart:math' as math;

import 'package:flutter/material.dart';

import 'oklch.dart';

/// The whole visual system, derived from about fifteen numbers.
///
/// From `docs/design_handoff_storm_design_system/`. The point of the token
/// layer is that there are no per-screen styling decisions left to make: one
/// change to an input here moves every atom, molecule, organism and screen
/// together, and the two shipped identities are the *same* derivation with
/// different inputs rather than two hand-written palettes that drift.
///
/// **Semantic meaning is fixed and must not drift.** Accent is interactive —
/// links, active state, the primary action. Amber means tags and highlight and
/// nothing else. Green means synced. Danger means failure or conflict. Grey
/// ([text3]) means offline or inactive. A colour used for a second purpose
/// stops working as a signal.
@immutable
class StormTokens extends ThemeExtension<StormTokens> {
  const StormTokens({
    required this.preset,
    required this.brightness,
    required this.bg,
    required this.surface,
    required this.surface2,
    required this.border,
    required this.text,
    required this.text2,
    required this.text3,
    required this.accent,
    required this.accentSoft,
    required this.onAccent,
    required this.amber,
    required this.amberSoft,
    required this.green,
    required this.danger,
    required this.codePlate,
    required this.onCodePlate,
    required this.fs,
    required this.scale,
    required this.sp,
    required this.rCard,
    required this.rControl,
    required this.bw,
    required this.shadowDepth,
    required this.duration,
    required this.surfaceL,
    required this.dir,
  });

  /// Which identity this is, so a preset can be named in settings and in tests.
  final StormPreset preset;
  final Brightness brightness;

  /// The page. Semantic colours are *not* measured against this — see
  /// [surface].
  final Color bg;

  /// What cards, sheets and bars are painted on, and the ground every semantic
  /// colour is measured against.
  ///
  /// In light mode this is **darker** than [bg], not lighter. That inversion is
  /// the trap the handoff records failing accessibility review twice: checking
  /// contrast against the page rather than against the card passes while the
  /// real composition fails.
  final Color surface;
  final Color surface2;
  final Color border;

  final Color text;
  final Color text2;

  /// Also the "offline / inactive" signal.
  final Color text3;

  final Color accent;
  final Color accentSoft;
  final Color onAccent;

  /// Tags and highlight only.
  final Color amber;
  final Color amberSoft;

  /// Synced, and nothing else.
  final Color green;

  /// Failure and conflict.
  final Color danger;

  /// The plate a QR code is drawn on, and the ink drawn on it.
  ///
  /// **Fixed in both themes, deliberately.** A QR is read by a camera through
  /// contrast, so this is a functional requirement rather than a palette
  /// choice — themed to Storm dark it would be a dark code on a dark ground
  /// and simply would not scan. Named here rather than written as a literal so
  /// the reason travels with it and the conformance test stays absolute.
  final Color codePlate;
  final Color onCodePlate;

  /// Base type size. The user can move this between 12 and 24.
  final double fs;

  /// Type ratio. Sizes are `fs * scale^n`.
  final double scale;

  /// Spacing base. Every gap is a multiple of it.
  final double sp;

  /// Cards, sheets, vault tiles.
  final double rCard;

  /// Inputs, buttons, and the corner bubbles — which are rounded *squares*.
  /// The nav bubble is a pill (999). That shape difference is deliberate
  /// grammar: corners are places, the pill is an action bar.
  final double rControl;

  final double bw;
  final double shadowDepth;
  final Duration duration;

  /// The surface's own OKLCH lightness, and which way "away from the page"
  /// points (+1 dark, -1 light).
  ///
  /// Exposed because a note's accent tile has to sit a step off *this theme's*
  /// surface rather than at a fixed lightness. A fixed value cannot work: Storm
  /// light's surface is 0.87 and SlowFlow's is 0.79, so one constant either
  /// vanishes into the first or floats off the second.
  final double surfaceL;
  final double dir;

  // ---- type scale ------------------------------------------------------

  /// The smallest step any label may be.
  ///
  /// A ratio applied twice below a small base drives labels into illegibility;
  /// at `fs` 12 and ratio 1.3, `fs / scale²` is 7.1px. The floor is not a
  /// suggestion.
  static const double minStep = 11;

  double get displaySize => fs * math.pow(scale, 3).toDouble();
  double get headingSize => fs * scale;
  double get bodySize => fs;
  double get labelSize => math.max(minStep, fs / math.pow(scale, 2).toDouble());
  double get codeSize => fs / scale;

  /// Chrome and display. Bundled, so it is identical on every platform.
  static const String sansFamily = 'IBMPlexSans';

  /// Labels, metadata, key chips, code.
  static const String monoFamily = 'IBMPlexMono';

  /// Note prose. Bundled for the same reason it always was — the editor's text
  /// metrics cannot depend on the network.
  static const String serifFamily = 'Newsreader';

  // ---- spacing and depth ----------------------------------------------

  double get gap => sp * 2;
  double get cardPad => sp * 3;
  double get sectionRhythm => sp * 7;

  List<BoxShadow> get shadow => [
    BoxShadow(
      offset: Offset(0, shadowDepth / 3),
      blurRadius: shadowDepth * 1.2,
      spreadRadius: -8,
      color: Color.fromRGBO(0, 0, 0, shadowDepth / 100),
    ),
  ];

  /// Every colour that has to stay readable on [surface], by name.
  ///
  /// Used by the contrast test, and the reason it is here rather than in the
  /// test is that adding a semantic colour without adding it to this map should
  /// be the awkward path.
  Map<String, Color> get semanticColors => {
    'accent': accent,
    'amber': amber,
    'green': green,
    'danger': danger,
    'text': text,
    'text2': text2,
    'text3': text3,
  };

  @override
  StormTokens copyWith({double? fs, StormPreset? preset}) =>
      StormTokens.from(preset ?? this.preset, fs: fs ?? this.fs);

  /// Colour-space interpolation is deliberately not implemented.
  ///
  /// Themes here change on a user's explicit choice, not as animation. Lerping
  /// between two derived palettes halfway would produce colours that satisfy
  /// none of the contrast guarantees, for a transition nobody asked for.
  @override
  StormTokens lerp(ThemeExtension<StormTokens>? other, double t) =>
      t < 0.5 ? this : (other as StormTokens? ?? this);

  /// Derives the whole system from one preset's inputs.
  factory StormTokens.from(StormPreset preset, {double? fs}) {
    final i = preset.inputs;
    final dark = i.dark;
    final dir = dark ? 1.0 : -1.0;
    final bgL = i.bgL / 100;

    Color neutral(double l) => oklch(l, i.chroma / 1000, i.hue);

    // The accent's lightness is capped in light mode. Without the cap a bright
    // accent stays bright on a pale ground and the contrast guarantee fails.
    final accentL = dark ? i.accentL / 100 : math.min(0.40, i.accentL / 100);

    return StormTokens(
      preset: preset,
      brightness: dark ? Brightness.dark : Brightness.light,
      bg: neutral(bgL),
      surface: neutral(bgL + dir * 0.05),
      surface2: neutral(bgL + dir * 0.09),
      border: neutral(bgL + dir * 0.14),
      text: neutral(dark ? 0.95 : 0.18),
      text2: neutral(dark ? 0.74 : 0.34),
      text3: neutral(dark ? 0.66 : 0.40),
      accent: oklch(accentL, i.accentC, i.accentH),
      accentSoft: oklch(bgL + dir * 0.08, i.accentC / 3, i.accentH),
      // A pale accent needs dark text on it; a deep one needs white.
      onAccent: accentL > 0.6
          ? oklch(0.16, 0.03, i.accentH)
          : const Color(0xFFFFFFFF),
      amber: oklch(dark ? 0.74 : 0.38, 0.14, 68),
      amberSoft: oklch(bgL + dir * 0.08, 0.05, 68),
      green: oklch(dark ? 0.74 : 0.38, 0.13, 148),
      danger: oklch(dark ? 0.70 : 0.38, 0.17, 25),
      // Not theme-dependent: a camera needs the contrast either way.
      codePlate: const Color(0xFFFFFFFF),
      onCodePlate: const Color(0xFF000000),
      fs: fs ?? i.fs,
      scale: i.scale,
      sp: i.sp,
      rCard: i.rCard,
      rControl: i.rControl,
      bw: i.bw,
      shadowDepth: i.shadow,
      duration: Duration(milliseconds: i.dur),
      surfaceL: bgL + dir * 0.05,
      dir: dir,
    );
  }
}

/// The inputs. Everything else in [StormTokens] is derived from these.
@immutable
class TokenInputs {
  const TokenInputs({
    required this.dark,
    required this.bgL,
    required this.hue,
    required this.chroma,
    required this.accentH,
    required this.accentC,
    required this.accentL,
    this.fs = 16,
    required this.scale,
    this.sp = 8,
    required this.rCard,
    required this.rControl,
    this.bw = 1,
    required this.shadow,
    required this.dur,
  });

  final bool dark;

  /// Background lightness, 0–100.
  final double bgL;

  /// Neutral hue, and how warm the neutrals are (chroma is /1000).
  final double hue;
  final double chroma;

  final double accentH;

  /// OKLCH chroma directly — *not* divided by 100.
  ///
  /// The handoff's table gives `0.15` while its formula says `accentC/100`.
  /// The two disagree, and applying the division yields a near-grey accent
  /// (`C = 0.0015`) rather than the specified `#9d84ec`. The table value is the
  /// real one; this field is named and documented so the next reader does not
  /// have to rediscover that.
  final double accentC;

  /// Accent lightness as a percentage, capped at 40 in light mode.
  final double accentL;

  final double fs;
  final double scale;
  final double sp;
  final double rCard;
  final double rControl;
  final double bw;
  final double shadow;
  final int dur;
}

/// The shipped identities.
enum StormPreset {
  /// The product's real identity.
  stormDark(
    'storm-dark',
    'Storm dark',
    TokenInputs(
      dark: true,
      bgL: 22,
      hue: 55,
      chroma: 8,
      accentH: 293,
      accentC: 0.15,
      accentL: 68,
      scale: 1.25,
      rCard: 16,
      rControl: 10,
      shadow: 35,
      dur: 180,
    ),
  ),

  /// Storm's own light mode.
  ///
  /// Not in the handoff, which specifies only a dark Storm and a light
  /// SlowFlow — but the app has always had a light mode, and switching it
  /// should not change the product's identity. Same hue and same accent as
  /// [stormDark] with the derivation flipped; `bgL` 92 was chosen because the
  /// worst semantic contrast there is 6.2:1, with room either side.
  stormLight(
    'storm-light',
    'Storm light',
    TokenInputs(
      dark: false,
      bgL: 92,
      hue: 55,
      chroma: 8,
      accentH: 293,
      accentC: 0.15,
      accentL: 68,
      scale: 1.25,
      rCard: 16,
      rControl: 10,
      shadow: 20,
      dur: 180,
    ),
  ),

  /// A warm, light alternate — and the proof the token layer is genuinely
  /// theme-independent rather than dark-only with a light afterthought.
  slowflowEarth(
    'slowflow-earth',
    'SlowFlow earth',
    TokenInputs(
      dark: false,
      bgL: 84,
      hue: 62,
      chroma: 22,
      accentH: 55,
      accentC: 0.06,
      accentL: 45,
      scale: 1.30,
      rCard: 2,
      rControl: 10,
      shadow: 16,
      dur: 320,
    ),
  );

  const StormPreset(this.wire, this.label, this.inputs);

  /// How the choice is stored. Stable across renames of [label].
  final String wire;
  final String label;
  final TokenInputs inputs;

  static StormPreset parse(String? wire) => StormPreset.values.firstWhere(
    (p) => p.wire == wire,
    orElse: () => stormDark,
  );
}

/// Reaches the tokens from anywhere in the tree.
///
/// Falls back to the preset matching the ambient brightness when the extension
/// is absent, rather than throwing. The app always installs the real one, but a
/// widget can legitimately be rendered under a plain `MaterialApp` — a test
/// harness, a dialog with its own theme — and an atom that crashes there is an
/// atom that cannot be reused.
extension StormTokensOf on BuildContext {
  StormTokens get tokens {
    final theme = Theme.of(this);
    return theme.extension<StormTokens>() ??
        StormTokens.from(
          theme.brightness == Brightness.dark
              ? StormPreset.stormDark
              : StormPreset.stormLight,
        );
  }
}

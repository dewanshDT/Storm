import 'dart:convert';

import 'package:flutter/material.dart' show Brightness, immutable;

import '../tokens.dart';

/// Who a theme is inspired by, for Settings › About.
@immutable
class ThemeCredit {
  const ThemeCredit({
    required this.name,
    required this.url,
    required this.licence,
  });

  final String name;
  final String url;
  final String licence;
}

/// A theme family: one look with a light member, a dark one, or both.
@immutable
class ThemeFamily {
  const ThemeFamily({
    required this.id,
    required this.label,
    this.inspiredBy,
    this.light,
    this.dark,
  }) : assert(light != null || dark != null);

  final String id;
  final String label;
  final ThemeCredit? inspiredBy;
  final ThemeVariant? light;
  final ThemeVariant? dark;

  bool has(Brightness b) => (b == Brightness.dark ? dark : light) != null;

  /// The member for [b], or the only one the family has.
  ThemeVariant variantFor(Brightness b) =>
      (b == Brightness.dark ? dark ?? light : light ?? dark)!;
}

/// A theme config that cannot be used, and why.
class ThemeConfigError extends FormatException {
  const ThemeConfigError(super.message, [this.source_]);

  final String? source_;

  @override
  String toString() =>
      'Theme config${source_ == null ? '' : ' $source_'}: $message';
}

/// The shape every Storm-made theme shares; `shape: "storm"` in a config.
const stormShape = (scale: 1.25, rCard: 16.0, rControl: 10.0, dur: 180);

/// Parses one family from a config's text (decision 160).
///
/// The same parser is meant to read a family from anywhere, so it trusts
/// nothing: every number is range-checked, and anything wrong throws a
/// [ThemeConfigError] naming the field rather than producing a theme that
/// fails quietly.
ThemeFamily parseThemeFamily(String text, {String? source}) {
  final Object? json;
  try {
    json = jsonDecode(text);
  } on FormatException catch (e) {
    throw ThemeConfigError('not JSON (${e.message})', source);
  }
  return themeFamilyFromJson(json, source: source);
}

ThemeFamily themeFamilyFromJson(Object? json, {String? source}) {
  Never fail(String why) => throw ThemeConfigError(why, source);

  Map<String, Object?> object(Object? v, String at) {
    if (v is Map<String, Object?>) return v;
    fail('$at must be an object');
  }

  String string(Map<String, Object?> m, String key, String at) {
    final v = m[key];
    if (v is String && v.trim().isNotEmpty) return v.trim();
    fail('$at.$key must be a non-empty string');
  }

  double number(
    Map<String, Object?> m,
    String key,
    String at,
    double min,
    double max, {
    double? orElse,
  }) {
    final v = m[key];
    if (v == null && orElse != null) return orElse;
    if (v is! num) fail('$at.$key must be a number');
    final d = v.toDouble();
    if (!d.isFinite || d < min || d > max) {
      fail('$at.$key is $d, outside $min–$max');
    }
    return d;
  }

  List<double> sixteen(Object? v, String at, double min, double max) {
    final List<Object?> list;
    if (v is List) {
      list = v;
    } else if (v is Map<String, Object?>) {
      final keys = AnsiInputs.names.map((n) => n.replaceAll(' ', ''));
      list = [
        for (final (n, k) in keys.indexed)
          v[k] ?? v[AnsiInputs.names[n]] ?? v[_camel(AnsiInputs.names[n])],
      ];
    } else {
      fail('$at must be a list of 16 numbers or a map of slot names');
    }
    if (list.length != 16) fail('$at needs 16 values, has ${list.length}');
    return [
      for (final (n, x) in list.indexed)
        if (x is num && x.isFinite && x >= min && x <= max)
          x.toDouble()
        else
          fail('$at[$n] (${AnsiInputs.names[n]}) must be a number $min–$max'),
    ];
  }

  final root = object(json, 'config');
  final id = string(root, 'id', 'config');
  if (!RegExp(r'^[a-z0-9][a-z0-9-]{0,39}$').hasMatch(id)) {
    fail('config.id "$id" must be lower-case letters, digits and dashes');
  }
  final label = string(root, 'label', 'config');

  ThemeCredit? credit;
  final inspired = root['inspiredBy'];
  if (inspired != null) {
    final m = object(inspired, 'inspiredBy');
    credit = ThemeCredit(
      name: string(m, 'name', 'inspiredBy'),
      url: string(m, 'url', 'inspiredBy'),
      licence: string(m, 'licence', 'inspiredBy'),
    );
  }

  final shapeJson = root['shape'] ?? 'storm';
  ({double scale, double rCard, double rControl, double? shadow, int dur})
  shape;
  if (shapeJson == 'storm') {
    shape = (
      scale: stormShape.scale,
      rCard: stormShape.rCard,
      rControl: stormShape.rControl,
      shadow: null,
      dur: stormShape.dur,
    );
  } else if (shapeJson is Map<String, Object?>) {
    shape = (
      scale: number(shapeJson, 'scale', 'shape', 1, 2),
      rCard: number(shapeJson, 'rCard', 'shape', 0, 40),
      rControl: number(shapeJson, 'rControl', 'shape', 0, 40),
      shadow: number(shapeJson, 'shadow', 'shape', 0, 100),
      dur: number(shapeJson, 'dur', 'shape', 0, 2000).round(),
    );
  } else {
    fail('shape must be "storm" or an object');
  }

  final variants = object(root['variants'], 'variants');
  for (final k in variants.keys) {
    if (k != 'light' && k != 'dark') fail('variants.$k: only light and dark');
  }

  ThemeVariant? variant(String mode) {
    final raw = variants[mode];
    if (raw == null) return null;
    final at = 'variants.$mode';
    final v = object(raw, at);
    final dark = mode == 'dark';
    final bgL = number(v, 'bgL', at, 0, 100);
    if (dark ? bgL > 40 : bgL < 60) {
      fail('$at.bgL $bgL is not a $mode page (dark ≤ 40, light ≥ 60)');
    }
    final sem = v['semantic'] == null
        ? const <String, Object?>{}
        : object(v['semantic'], '$at.semantic');
    final hues = v['ansiHues'];
    final chromaJson = v['ansiChroma'];
    AnsiInputs ansi = AnsiInputs.storm;
    if (hues != null) {
      final h = sixteen(hues, '$at.ansiHues', 0, 360);
      final List<double> c;
      if (chromaJson is num) {
        if (chromaJson < 0 || chromaJson > 0.4) {
          fail('$at.ansiChroma must be 0–0.4');
        }
        c = List.filled(16, chromaJson.toDouble());
      } else if (chromaJson == null) {
        c = List.filled(16, 0.12);
      } else {
        c = sixteen(chromaJson, '$at.ansiChroma', 0, 0.4);
      }
      final l = v['ansiLightness'] == null
          ? null
          : sixteen(v['ansiLightness'], '$at.ansiLightness', 0, 100);
      ansi = AnsiInputs(hues: h, chroma: c, lightness: l);
    } else if (chromaJson != null || v['ansiLightness'] != null) {
      fail('$at.ansiChroma and ansiLightness need ansiHues');
    }
    return ConfiguredVariant(
      family: id,
      wire: '$id-$mode',
      label: v['label'] == null ? '$label $mode' : string(v, 'label', at),
      inputs: TokenInputs(
        dark: dark,
        bgL: bgL,
        hue: number(v, 'hue', at, 0, 360),
        chroma: number(v, 'chroma', at, 0, 100),
        accentH: number(v, 'accentH', at, 0, 360),
        accentC: number(v, 'accentC', at, 0, 0.4),
        accentL: number(v, 'accentL', at, 0, 100),
        scale: shape.scale,
        rCard: shape.rCard,
        rControl: shape.rControl,
        shadow: shape.shadow ?? (dark ? 35 : 20),
        dur: shape.dur,
        amberHue: number(sem, 'amber', '$at.semantic', 0, 360, orElse: 68),
        greenHue: number(sem, 'green', '$at.semantic', 0, 360, orElse: 148),
        dangerHue: number(sem, 'danger', '$at.semantic', 0, 360, orElse: 25),
        ansi: ansi,
      ),
    );
  }

  final light = variant('light');
  final dark = variant('dark');
  if (light == null && dark == null) fail('variants needs light or dark');
  return ThemeFamily(
    id: id,
    label: label,
    inspiredBy: credit,
    light: light,
    dark: dark,
  );
}

String _camel(String name) {
  final parts = name.split(' ');
  return parts.first +
      parts.skip(1).map((p) => p[0].toUpperCase() + p.substring(1)).join();
}
